from __future__ import annotations

import argparse
import asyncio
import json
import statistics
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

import httpx
from jsonschema import Draft202012Validator

ML_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DATASET = ML_ROOT / "data" / "triage-eval-v1.jsonl"
DEFAULT_PROMPT = ML_ROOT / "prompts" / "triage-extraction-v2.txt"
DEFAULT_SCHEMA = ML_ROOT.parent / "schemas" / "triage-extraction-v2.json"
CRITICAL_FIELDS = (
    "victim_location",
    "headcount",
    "required_asset",
    "hazards",
    "needs_human_review",
)


@dataclass(frozen=True)
class CaseResult:
    case_id: str
    category: str
    schema_valid: bool
    critical_fields_correct: int
    critical_fields_total: int
    exact_match: bool
    human_review_expected: bool
    human_review_actual: bool | None
    unsupported_fact_count: int
    latency_ms: float
    error: str | None
    expected: dict[str, Any]
    actual: dict[str, Any] | None
    raw_response: str | None


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def load_cases(path: Path) -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            try:
                cases.append(json.loads(line))
            except json.JSONDecodeError as exc:
                raise ValueError(f"Invalid JSONL at {path}:{line_number}: {exc}") from exc
    return cases


def normalize(value: Any) -> Any:
    if isinstance(value, str):
        return " ".join(value.casefold().split())
    if isinstance(value, list):
        return [normalize(item) for item in value]
    if isinstance(value, dict):
        return {key: normalize(item) for key, item in value.items()}
    return value


def unsupported_fact_count(expected: dict[str, Any], actual: dict[str, Any]) -> int:
    count = 0
    for field in ("victim_location", "headcount", "required_asset"):
        if expected.get(field) is None and actual.get(field) is not None:
            count += 1

    expected_hazards = expected.get("hazards", [])
    actual_hazards = actual.get("hazards", [])
    if not isinstance(actual_hazards, list):
        return count
    if len(actual_hazards) > len(expected_hazards):
        count += len(actual_hazards) - len(expected_hazards)

    for index, expected_hazard in enumerate(expected_hazards):
        if index >= len(actual_hazards) or not isinstance(actual_hazards[index], dict):
            continue
        actual_hazard = actual_hazards[index]
        for field in ("road_segment", "status", "duration_hours"):
            if expected_hazard.get(field) is None and actual_hazard.get(field) is not None:
                count += 1
    return count


def score_case(
    case: dict[str, Any],
    actual: dict[str, Any],
    validator: Draft202012Validator,
    latency_ms: float = 0.0,
    raw_response: str | None = None,
) -> CaseResult:
    errors = sorted(validator.iter_errors(actual), key=lambda item: list(item.path))
    expected = case["expected"]
    correct = sum(
        normalize(actual.get(field)) == normalize(expected.get(field)) for field in CRITICAL_FIELDS
    )
    return CaseResult(
        case_id=case["id"],
        category=case["category"],
        schema_valid=not errors,
        critical_fields_correct=correct,
        critical_fields_total=len(CRITICAL_FIELDS),
        exact_match=correct == len(CRITICAL_FIELDS),
        human_review_expected=bool(expected["needs_human_review"]),
        human_review_actual=actual.get("needs_human_review"),
        unsupported_fact_count=unsupported_fact_count(expected, actual),
        latency_ms=latency_ms,
        error="; ".join(error.message for error in errors) or None,
        expected=expected,
        actual=actual,
        raw_response=raw_response,
    )


def error_result(
    case: dict[str, Any], latency_ms: float, error: str, raw_response: str | None = None
) -> CaseResult:
    return CaseResult(
        case_id=case["id"],
        category=case["category"],
        schema_valid=False,
        critical_fields_correct=0,
        critical_fields_total=len(CRITICAL_FIELDS),
        exact_match=False,
        human_review_expected=bool(case["expected"]["needs_human_review"]),
        human_review_actual=None,
        unsupported_fact_count=0,
        latency_ms=latency_ms,
        error=error,
        expected=case["expected"],
        actual=None,
        raw_response=raw_response,
    )


def request_payload(
    backend: str, model: str, prompt: str, schema: dict[str, Any], message: str
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "model": model,
        "messages": [
            {"role": "system", "content": prompt},
            {"role": "user", "content": message},
        ],
        "temperature": 0.0,
        "max_tokens": 512,
    }
    if backend == "llama-cpp":
        payload["response_format"] = {
            "type": "json_schema",
            "json_schema": {
                "name": "crisisgraph_triage_extraction",
                "strict": True,
                "schema": schema,
            },
        }
    elif backend == "vllm":
        payload["structured_outputs"] = {"json": schema}
    else:
        raise ValueError(f"Unsupported backend: {backend}")
    return payload


async def evaluate_case(
    client: httpx.AsyncClient,
    semaphore: asyncio.Semaphore,
    case: dict[str, Any],
    backend: str,
    base_url: str,
    model: str,
    prompt: str,
    schema: dict[str, Any],
    validator: Draft202012Validator,
) -> CaseResult:
    payload = request_payload(backend, model, prompt, schema, case["input"])
    raw_response: str | None = None
    try:
        async with semaphore:
            started = time.perf_counter()
            response = await client.post(f"{base_url.rstrip('/')}/chat/completions", json=payload)
            response.raise_for_status()
            latency_ms = (time.perf_counter() - started) * 1000
        body = response.json()
        raw_response = body["choices"][0]["message"]["content"]
        actual = json.loads(raw_response)
        if not isinstance(actual, dict):
            return error_result(
                case, latency_ms, "Model output must be a JSON object", raw_response
            )
        return score_case(case, actual, validator, latency_ms, raw_response)
    except (httpx.HTTPError, KeyError, IndexError, TypeError, json.JSONDecodeError) as exc:
        latency_ms = (time.perf_counter() - started) * 1000 if "started" in locals() else 0.0
        return error_result(case, latency_ms, str(exc), raw_response)


def percentile(values: list[float], quantile: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(round((len(ordered) - 1) * quantile), len(ordered) - 1)
    return ordered[index]


def summarize(results: list[CaseResult], model: str, backend: str) -> dict[str, Any]:
    count = len(results)
    review_cases = [result for result in results if result.human_review_expected]
    review_true_positives = sum(result.human_review_actual is True for result in review_cases)
    total_fields = sum(result.critical_fields_total for result in results)
    correct_fields = sum(result.critical_fields_correct for result in results)
    latencies = [result.latency_ms for result in results]
    field_accuracy = {
        field: (
            sum(
                normalize(result.actual.get(field)) == normalize(result.expected.get(field))
                for result in results
                if result.actual is not None
            )
            / count
            if count
            else 0.0
        )
        for field in CRITICAL_FIELDS
    }
    return {
        "model": model,
        "backend": backend,
        "case_count": count,
        "schema_validity": sum(result.schema_valid for result in results) / count if count else 0.0,
        "critical_field_accuracy": correct_fields / total_fields if total_fields else 0.0,
        "field_accuracy": field_accuracy,
        "exact_match_accuracy": sum(result.exact_match for result in results) / count
        if count
        else 0.0,
        "human_review_recall": review_true_positives / len(review_cases) if review_cases else 0.0,
        "unsupported_fact_count": sum(result.unsupported_fact_count for result in results),
        "unsupported_fact_case_rate": (
            sum(result.unsupported_fact_count > 0 for result in results) / count if count else 0.0
        ),
        "error_count": sum(result.error is not None for result in results),
        "latency_ms": {
            "mean": statistics.fmean(latencies) if latencies else 0.0,
            "p50": percentile(latencies, 0.50),
            "p95": percentile(latencies, 0.95),
        },
        "results": [asdict(result) for result in results],
    }


async def run(args: argparse.Namespace) -> dict[str, Any]:
    cases = load_cases(args.dataset)
    prompt = args.prompt.read_text(encoding="utf-8").strip()
    schema = load_json(args.schema)
    validator = Draft202012Validator(schema)
    validator.check_schema(schema)
    semaphore = asyncio.Semaphore(args.concurrency)
    timeout = httpx.Timeout(args.timeout)
    async with httpx.AsyncClient(timeout=timeout) as client:
        results = await asyncio.gather(
            *(
                evaluate_case(
                    client,
                    semaphore,
                    case,
                    args.backend,
                    args.base_url,
                    args.model,
                    prompt,
                    schema,
                    validator,
                )
                for case in cases
            )
        )
    return summarize(results, args.model, args.backend)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Evaluate CrisisGraph triage extraction")
    parser.add_argument("--backend", choices=("llama-cpp", "vllm"), default="llama-cpp")
    parser.add_argument("--base-url", default="http://127.0.0.1:8000/v1")
    parser.add_argument("--model", default="Qwen/Qwen3-4B-Instruct-2507")
    parser.add_argument("--dataset", type=Path, default=DEFAULT_DATASET)
    parser.add_argument("--prompt", type=Path, default=DEFAULT_PROMPT)
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    parser.add_argument("--concurrency", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120.0)
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.concurrency < 1:
        raise SystemExit("--concurrency must be at least 1")
    report = asyncio.run(run(args))
    rendered = json.dumps(report, ensure_ascii=False, indent=2)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)


if __name__ == "__main__":
    main()
