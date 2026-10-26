"""Evaluate triage extraction V3: V2 fields plus ``victim_profile`` and urgency.

V2 fields are scored by the existing evaluator (``triage_eval.score_case``).
This module adds profile and urgency metrics, computed on the *model's*
profile run through the deterministic rule table (``urgency.derive_urgency``),
and compared with the annotator's independent gold urgency.

A failed or invalid model output counts as an empty profile: the runtime then
fails closed to manual triage, but for scoring it is the extraction that
missed every fact, so it is never hidden from recall or under-triage.

Run serially with a checkpoint (CPU llama.cpp), resumable after interruption:

    uv run crisisgraph-triage-eval-v3 --dataset data/triage-eval-v3-dev-candidates.jsonl \\
        --prompt prompts/triage-extraction-v3.0.txt --output runs/<name>.json
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import math
import os
from dataclasses import asdict
from pathlib import Path
from typing import Any

import httpx
from jsonschema import Draft202012Validator

from crisisgraph_ml.triage_eval import (
    ML_ROOT,
    CaseResult,
    evaluate_case,
    load_cases,
    load_json,
    summarize_metrics,
)
from crisisgraph_ml.urgency import RANK, RULE_VERSIONS, RULES_VERSION, derive_urgency

DEFAULT_SCHEMA = ML_ROOT.parent / "schemas" / "triage-extraction-v3.json"
DEFAULT_DATASET = ML_ROOT / "data" / "triage-eval-v3-dev-candidates.jsonl"
CRITICAL_CONDITIONS = ("UNCONSCIOUS", "BREATHING_DIFFICULTY", "CARDIAC", "BLEEDING")
MEDICAL = CRITICAL_CONDITIONS + ("INJURY", "CHRONIC_CARE")
GROUPS = ("CHILD", "INFANT", "ELDERLY", "PREGNANT", "DISABLED", "BEDRIDDEN")
SCALARS = ("reporter_relation", "reporter_on_scene", "water_level", "water_trend")
TIERS = ("CRITICAL", "URGENT", "UNKNOWN", "STABLE")
EMPTY_PROFILE: dict[str, Any] = {
    "reporter_relation": None,
    "reporter_on_scene": None,
    "medical_conditions": [],
    "other_conditions": [],
    "vulnerable_groups": [],
    "water_level": None,
    "water_trend": None,
    "suggested_urgency": None,
}

# Predeclared pass criteria (core implementation plan, ML track).
GATES = {
    "max_critical_under_triage_cases": 1,
    "min_critical_condition_recall": 0.90,
    "max_remote_caller_miss_rate": 0.10,
    "max_v2_exact_match_drop_cases": 2,
    "max_p95_latency_increase": 0.30,
}


def wilson_interval(successes: int, total: int, z: float = 1.96) -> tuple[float, float]:
    """95% Wilson score interval; honest for the small counts in these sets."""
    if total == 0:
        return (0.0, 0.0)
    p = successes / total
    denominator = 1 + z * z / total
    centre = (p + z * z / (2 * total)) / denominator
    margin = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / denominator
    return (max(0.0, centre - margin), min(1.0, centre + margin))


def rate(successes: int, total: int) -> dict[str, Any]:
    low, high = wilson_interval(successes, total)
    return {
        "value": successes / total if total else None,
        "n": total,
        "count": successes,
        "ci95": [round(low, 4), round(high, 4)],
    }


def predicted_profile(result: CaseResult) -> dict[str, Any]:
    """The model's profile, or an empty one when the output is unusable."""
    if not result.schema_valid or result.actual is None:
        return dict(EMPTY_PROFILE)
    profile = result.actual.get("victim_profile")
    return {**EMPTY_PROFILE, **profile} if isinstance(profile, dict) else dict(EMPTY_PROFILE)


def set_scores(expected: list[set[str]], actual: list[set[str]], labels: tuple[str, ...]):
    per_label = {}
    tp_total = fp_total = fn_total = 0
    for label in labels:
        tp = sum(label in e and label in a for e, a in zip(expected, actual))
        fn = sum(label in e and label not in a for e, a in zip(expected, actual))
        fp = sum(label not in e and label in a for e, a in zip(expected, actual))
        tp_total, fp_total, fn_total = tp_total + tp, fp_total + fp, fn_total + fn
        per_label[label] = {
            "recall": rate(tp, tp + fn),
            "precision": rate(tp, tp + fp),
            "false_positive": fp,
        }
    precision = tp_total / (tp_total + fp_total) if tp_total + fp_total else None
    recall = tp_total / (tp_total + fn_total) if tp_total + fn_total else None
    f1 = (
        2 * precision * recall / (precision + recall)
        if precision is not None and recall is not None and precision + recall
        else None
    )
    return {"per_label": per_label, "micro_precision": precision, "micro_recall": recall,
            "micro_f1": f1}


def invented_facts(expected: dict[str, Any], actual: dict[str, Any]) -> list[str]:
    """Profile facts the model asserted that the labels do not contain."""
    facts = [f"medical:{m}" for m in set(actual["medical_conditions"]) - set(expected["medical_conditions"])]
    facts += [f"group:{g}" for g in set(actual["vulnerable_groups"]) - set(expected["vulnerable_groups"])]
    if actual["other_conditions"] and not expected["other_conditions"]:
        facts.append("other_conditions")
    for field in SCALARS:
        if expected[field] is None and actual[field] is not None:
            facts.append(field)
    return facts


def score_profiles(
    results: list[CaseResult], cases: list[dict[str, Any]], rules: str = RULES_VERSION
) -> dict[str, Any]:
    by_id = {case["id"]: case for case in cases}
    rows = []
    for result in results:
        case = by_id[result.case_id]
        expected = {**EMPTY_PROFILE, **case["expected"]["victim_profile"]}
        actual = predicted_profile(result)
        derived = derive_urgency(actual, rules)
        rule_only = derive_urgency({**actual, "suggested_urgency": None}, rules)
        oracle = derive_urgency({**expected, "suggested_urgency": None}, rules)
        rows.append(
            {
                "case_id": result.case_id,
                "category": result.category,
                "language": result.language,
                "usable_output": result.schema_valid and result.actual is not None,
                "expected": expected,
                "actual": actual,
                "gold_urgency": case["expected_urgency"],
                "urgency": derived.urgency,
                "rule_only_urgency": rule_only.urgency,
                "oracle_urgency": oracle.urgency,
                "flags": derived.flags,
                "review_priority": derived.review_priority,
                "invented": invented_facts(expected, actual),
            }
        )
    return {"rows": rows, **profile_metrics(rows)}


def under(predicted: str, gold: str) -> bool:
    return RANK[predicted] < RANK[gold]


def profile_metrics(rows: list[dict[str, Any]]) -> dict[str, Any]:
    n = len(rows)
    expected = [r["expected"] for r in rows]
    actual = [r["actual"] for r in rows]
    gold_critical = [r for r in rows if r["gold_urgency"] == "CRITICAL"]
    remote = [r for r in rows if r["expected"]["reporter_on_scene"] is False]
    raised = [r for r in rows if "SUGGESTED_URGENCY_RAISED" in r["flags"]]

    def confusion(key: str) -> dict[str, dict[str, int]]:
        return {g: {p: sum(r["gold_urgency"] == g and r[key] == p for r in rows) for p in TIERS}
                for g in TIERS}

    return {
        "case_count": n,
        "usable_outputs": sum(r["usable_output"] for r in rows),
        "scalar_accuracy": {
            field: rate(sum(e[field] == a[field] for e, a in zip(expected, actual)), n)
            for field in SCALARS
        },
        "medical_conditions": set_scores(
            [set(e["medical_conditions"]) for e in expected],
            [set(a["medical_conditions"]) for a in actual],
            MEDICAL,
        ),
        "vulnerable_groups": set_scores(
            [set(e["vulnerable_groups"]) for e in expected],
            [set(a["vulnerable_groups"]) for a in actual],
            GROUPS,
        ),
        # Free text is not matched exactly; detection of "something unlisted" is.
        "other_conditions_detection": set_scores(
            [{"ANY"} if e["other_conditions"] else set() for e in expected],
            [{"ANY"} if a["other_conditions"] else set() for a in actual],
            ("ANY",),
        )["per_label"]["ANY"],
        "urgency": {
            "accuracy": rate(sum(r["urgency"] == r["gold_urgency"] for r in rows), n),
            "under_triage": rate(sum(under(r["urgency"], r["gold_urgency"]) for r in rows), n),
            "over_triage": rate(sum(under(r["gold_urgency"], r["urgency"]) for r in rows), n),
            "critical_under_triage": rate(
                sum(under(r["urgency"], "CRITICAL") for r in gold_critical), len(gold_critical)
            ),
            "critical_under_triage_cases": [
                r["case_id"] for r in gold_critical if under(r["urgency"], "CRITICAL")
            ],
            "rule_only_critical_under_triage": rate(
                sum(under(r["rule_only_urgency"], "CRITICAL") for r in gold_critical),
                len(gold_critical),
            ),
            # Ceiling: the rule table applied to the gold labels (no model error).
            "oracle_under_triage_cases": [
                r["case_id"] for r in rows if under(r["oracle_urgency"], r["gold_urgency"])
            ],
            "suggestion_raised": len(raised),
            "suggestion_raised_correctly": sum(
                RANK[r["urgency"]] <= RANK[r["gold_urgency"]] for r in raised
            ),
            "confusion_gold_by_predicted": confusion("urgency"),
        },
        "remote_caller_miss": rate(
            sum(r["actual"]["reporter_on_scene"] is True for r in remote), len(remote)
        ),
        "remote_caller_unknown": rate(
            sum(r["actual"]["reporter_on_scene"] is None for r in remote), len(remote)
        ),
        "invented_fact_cases": rate(sum(bool(r["invented"]) for r in rows), n),
        "invented_fact_count": sum(len(r["invented"]) for r in rows),
    }


def grouped(rows: list[dict[str, Any]], key: str) -> dict[str, Any]:
    return {
        value: profile_metrics([r for r in rows if r[key] == value])
        for value in sorted({r[key] for r in rows})
    }


def summarize_v3(
    results: list[CaseResult],
    cases: list[dict[str, Any]],
    identity: dict[str, Any],
    rules: str = RULES_VERSION,
) -> dict[str, Any]:
    profile = score_profiles(results, cases, rules)
    rows = profile.pop("rows")
    return {
        "identity": identity,
        "urgency_rules": rules,
        "v2_fields": summarize_metrics(results),
        "profile": profile,
        "profile_by_category": grouped(rows, "category"),
        "profile_by_language": grouped(rows, "language"),
        "rows": rows,
        "results": [asdict(result) for result in results],
    }


def check_gates(
    test_report: dict[str, Any],
    carryover_report: dict[str, Any],
    baseline_exact_match: int,
    baseline_review_recall: float,
    baseline_p95_ms: float,
    waived: frozenset[str] = frozenset(),
) -> dict[str, Any]:
    """Apply the predeclared pass criteria to a candidate's two reports.

    ``waived`` names checks the owner has waived (still computed and reported,
    but not counted toward ``passed``), e.g. ``latency_p95`` for CPU serving.
    """
    profile = test_report["profile"]
    medical = profile["medical_conditions"]["per_label"]
    v2 = carryover_report["v2_fields"]
    exact = round(v2["exact_match_accuracy"] * v2["case_count"])
    p95 = max(test_report["v2_fields"]["latency_ms"]["p95"], v2["latency_ms"]["p95"])
    checks = {
        "critical_under_triage": (
            profile["urgency"]["critical_under_triage"]["count"]
            <= GATES["max_critical_under_triage_cases"]
        ),
        **{
            f"recall_{condition}": (medical[condition]["recall"]["value"] or 0.0)
            >= GATES["min_critical_condition_recall"]
            for condition in CRITICAL_CONDITIONS
        },
        "remote_caller_miss": (profile["remote_caller_miss"]["value"] or 0.0)
        <= GATES["max_remote_caller_miss_rate"],
        "v2_exact_match": exact >= baseline_exact_match - GATES["max_v2_exact_match_drop_cases"],
        "v2_review_recall": v2["human_review_recall"] >= baseline_review_recall,
        "latency_p95": p95 <= baseline_p95_ms * (1 + GATES["max_p95_latency_increase"]),
    }
    unknown = waived - set(checks)
    if unknown:
        raise ValueError(f"unknown waived checks: {sorted(unknown)}")
    return {
        "passed": all(ok for name, ok in checks.items() if name not in waived),
        "checks": checks,
        "waived": sorted(waived),
        "v2_exact_match_cases": exact,
        "p95_ms": p95,
    }


def run_identity(args: argparse.Namespace) -> dict[str, Any]:
    def digest(path: Path) -> str:
        return hashlib.sha256(path.read_bytes()).hexdigest()

    return {
        "model": args.model,
        "adapter": args.adapter,
        "backend": args.backend,
        "base_url": args.base_url,
        "max_tokens": args.max_tokens,
        "dataset": args.dataset.name,
        "dataset_sha256": digest(args.dataset),
        "prompt": args.prompt.name,
        "prompt_sha256": digest(args.prompt),
        "schema_sha256": digest(args.schema),
    }


async def loaded_adapters(client: httpx.AsyncClient, args: argparse.Namespace) -> list[str]:
    """Adapters the server actually applies (llama.cpp ``/lora-adapters``)."""
    if args.backend != "llama-cpp":
        return []
    root = args.base_url.rstrip("/").removesuffix("/v1")
    response = await client.get(f"{root}/lora-adapters")
    response.raise_for_status()
    return sorted(Path(a["path"]).name for a in response.json() if a.get("scale", 1.0) != 0)


def check_adapter(declared: str, loaded: list[str]) -> None:
    """Refuse to run when --adapter does not describe what the server serves."""
    if declared == "none" and loaded:
        raise ValueError(f"--adapter none, but the server applies {loaded}")
    if declared != "none" and Path(declared).name not in loaded:
        raise ValueError(f"--adapter {declared}, but the server applies {loaded or 'none'}")


async def run(args: argparse.Namespace, transport: httpx.AsyncBaseTransport | None = None):
    cases = load_cases(args.dataset)
    prompt = args.prompt.read_text(encoding="utf-8").strip()
    schema = load_json(args.schema)
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    identity = run_identity(args)
    async with httpx.AsyncClient(timeout=httpx.Timeout(30.0), transport=transport) as client:
        identity["server_adapters"] = await loaded_adapters(client, args)
    check_adapter(args.adapter, identity["server_adapters"])

    checkpoint = args.output.with_suffix(".checkpoint.jsonl")
    identity_path = args.output.with_suffix(".identity.json")
    if identity_path.exists():
        if load_json(identity_path) != identity:
            raise ValueError("Checkpoint identity changed; use a new output path")
    else:
        if checkpoint.exists():
            raise ValueError("Checkpoint has no identity file")
        identity_path.parent.mkdir(parents=True, exist_ok=True)
        identity_path.write_text(json.dumps(identity, indent=2) + "\n", encoding="utf-8")

    results: list[CaseResult] = []
    if checkpoint.exists():
        results = [CaseResult(**json.loads(line)) for line in checkpoint.read_text().splitlines()]
        if [r.case_id for r in results] != [c["id"] for c in cases[: len(results)]]:
            raise ValueError("Checkpoint order differs from dataset")

    semaphore = asyncio.Semaphore(1)
    headers = {}
    if args.api_key_env:
        key = os.environ.get(args.api_key_env)
        if not key:
            raise SystemExit(f"environment variable {args.api_key_env} is not set")
        headers["Authorization"] = f"Bearer {key}"

    async with httpx.AsyncClient(
        timeout=httpx.Timeout(args.timeout), transport=transport, headers=headers
    ) as client:
        for case in cases[len(results):]:
            result = await evaluate_with_retry(client, semaphore, case, args, prompt, schema,
                                               validator)
            with checkpoint.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(asdict(result), ensure_ascii=False) + "\n")
            results.append(result)
            print(f"{len(results)}/{len(cases)} {result.case_id} "
                  f"{result.latency_ms / 1000:.1f}s error={result.error}", flush=True)

    report = summarize_v3(results, cases, identity, args.rules)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return report


TRANSIENT = ("429", "500", "502", "503", "504", "timed out", "Timeout")


def is_transient(error: str | None) -> bool:
    """API-side failures (rate limit, overload, timeout), not model failures."""
    return error is not None and any(marker in error for marker in TRANSIENT)


async def evaluate_with_retry(client, semaphore, case, args, prompt, schema, validator):
    """Pace requests and retry transient API errors with backoff.

    A transient error is never recorded as a model result: after the last
    retry the run stops, and rerunning the same command resumes from the
    checkpoint.
    """
    for attempt in range(args.max_retries + 1):
        if args.min_interval:
            await asyncio.sleep(args.min_interval)
        result = await evaluate_case(
            client, semaphore, case, args.backend, args.base_url, args.model, prompt,
            schema, validator, args.max_tokens,
        )
        if not is_transient(result.error):
            return result
        wait = min(30 * 2**attempt, 600)
        print(f"  transient error on {case['id']} (attempt {attempt + 1}): "
              f"{result.error[:120]}; waiting {wait}s", flush=True)
        await asyncio.sleep(wait)
    raise SystemExit(f"API still failing on {case['id']}; rerun the same command to resume")


def headline(report: dict[str, Any]) -> dict[str, Any]:
    profile, v2 = report["profile"], report["v2_fields"]
    medical = profile["medical_conditions"]["per_label"]
    return {
        "cases": profile["case_count"],
        "usable_outputs": profile["usable_outputs"],
        "v2_exact_match": round(v2["exact_match_accuracy"] * v2["case_count"]),
        "v2_review_recall": round(v2["human_review_recall"], 3),
        "critical_under_triage": profile["urgency"]["critical_under_triage_cases"],
        "critical_recall": {c: medical[c]["recall"]["value"] for c in CRITICAL_CONDITIONS},
        "remote_caller_miss": profile["remote_caller_miss"]["value"],
        "invented_fact_cases": profile["invented_fact_cases"]["count"],
        "latency_p50_s": round(v2["latency_ms"]["p50"] / 1000, 2),
        "latency_p95_s": round(v2["latency_ms"]["p95"] / 1000, 2),
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Evaluate CrisisGraph triage extraction V3")
    parser.add_argument("--backend", choices=("llama-cpp", "vllm", "openai"), default="llama-cpp")
    parser.add_argument("--api-key-env", default=None,
                        help="environment variable holding the API key (never logged)")
    parser.add_argument("--min-interval", type=float, default=0.0,
                        help="seconds to wait before each request (rate limits)")
    parser.add_argument("--max-retries", type=int, default=5)
    parser.add_argument("--base-url", default="http://127.0.0.1:8000/v1")
    parser.add_argument("--model", default="Qwen/Qwen3-4B-Instruct-2507")
    parser.add_argument("--adapter", default="none",
                        help="adapter file the server must be applying, or 'none' (checked)")
    parser.add_argument("--dataset", type=Path, default=DEFAULT_DATASET)
    parser.add_argument("--prompt", type=Path, required=True)
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    parser.add_argument("--max-tokens", type=int, default=768)
    parser.add_argument("--rules", choices=RULE_VERSIONS, default=RULES_VERSION,
                        help="urgency rules used for scoring (not part of the run identity)")
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args(argv)


def rescore(report: dict[str, Any], dataset: Path, rules: str) -> dict[str, Any]:
    """Score saved model outputs again under other urgency rules (no model calls)."""
    if hashlib.sha256(dataset.read_bytes()).hexdigest() != report["identity"]["dataset_sha256"]:
        raise ValueError("dataset differs from the one the run used")
    results = [CaseResult(**result) for result in report["results"]]
    return summarize_v3(results, load_cases(dataset), report["identity"], rules)


def main() -> None:
    report = asyncio.run(run(parse_args()))
    print(json.dumps(headline(report), indent=2))


def main_rescore() -> None:
    parser = argparse.ArgumentParser(description="Re-score a V3 run under other urgency rules")
    parser.add_argument("report", type=Path)
    parser.add_argument("--dataset", type=Path, required=True)
    parser.add_argument("--rules", choices=RULE_VERSIONS, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = rescore(load_json(args.report), args.dataset, args.rules)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(headline(report), indent=2))


if __name__ == "__main__":
    main()
