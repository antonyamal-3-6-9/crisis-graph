"""Blind V3 test set ``triage-eval-v3.1.0`` (SFT-V4 headline benchmark).

Drafted by a zero-context annotator agent from the guidelines only, before any
SFT-V4 training data existed (``data/triage-sft-v4-spec.md``). The agent's
candidates file is kept unchanged after delivery; owner review decisions live
in a separate corrections file, applied at freeze, so provenance stays visible.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any

from crisisgraph_ml.triage_eval import ML_ROOT, load_cases, load_json
from crisisgraph_ml.triage_eval_v3_dev import validate_cases, write_cases

DATA = ML_ROOT / "data"
CANDIDATES = DATA / "triage-eval-v3.1.0-candidates.jsonl"
CORRECTIONS = DATA / "triage-eval-v3.1.0-corrections.json"
FROZEN = DATA / "triage-eval-v3.1.0.jsonl"
MANIFEST = DATA / "triage-eval-v3.1.0.manifest.json"

CATEGORY_COUNTS = {
    "medical_critical": 28,
    "buried_adversarial": 14,
    "medical_other": 14,
    "vulnerable_water": 20,
    "reporter_location": 16,
    "negation_temporal": 12,
    "stranded_height": 6,
    "no_profile": 6,
    "irrelevant": 4,
}
CRITICAL_CONDITIONS = ("UNCONSCIOUS", "BREATHING_DIFFICULTY", "CARDIAC", "BLEEDING")
RELATIONS = {"SELF", "HOUSEHOLD", "THIRD_PARTY", "UNDER_REVIEW", None}

# Sets the blind test must not share inputs, places or 5-word sequences with.
OTHER_SETS = [
    DATA / "triage-eval-v3-dev-candidates.jsonl",
    DATA / "triage-eval-v3.0.1.jsonl",
    DATA / "triage-eval-v3-v2-carryover.jsonl",
]
SFT_EXPORTS = [
    DATA / "generated" / "triage-sft-v3-train.jsonl",
    DATA / "generated" / "triage-sft-v3-validation.jsonl",
]
PROMPT = ML_ROOT / "prompts" / "triage-extraction-v3.2.txt"
CONTRACT_FILES = {
    "schema": ML_ROOT.parent / "schemas" / "triage-extraction-v3.json",
    "annotation_guidelines": DATA / "triage-annotation-guidelines.md",
    "annotation_guideline_changes": DATA / "triage-annotation-guidelines-v3.1-changes.md",
    "urgency_rules": DATA / "urgency-rules-v2.jsonl",
}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def ngrams(text: str, n: int = 5) -> set[tuple[str, ...]]:
    # Words are runs of letters, digits and combining marks: Malayalam vowel signs
    # (Mn/Mc) are not \w, so a plain \w+ split would cut words into letters.
    words = re.findall(r"[\w\u0300-\u036f\u0d00-\u0d7f]+", text.casefold())
    return {tuple(words[i : i + n]) for i in range(len(words) - n + 1)}


def apply_corrections(
    cases: list[dict[str, Any]], corrections: dict[str, dict[str, Any]]
) -> list[dict[str, Any]]:
    """Apply owner decisions keyed by case id.

    Keys are dotted paths (``input``, ``expected_urgency``,
    ``expected.headcount``, ``expected.victim_profile.water_level``…), or
    ``drop: true`` to remove a case. Unknown ids or paths are errors.
    """
    by_id = {case["id"]: case for case in cases}
    unknown = set(corrections) - set(by_id)
    if unknown:
        raise ValueError(f"corrections for unknown ids: {sorted(unknown)}")
    result = []
    for case in cases:
        fix = corrections.get(case["id"], {})
        if fix.get("drop"):
            continue
        case = copy.deepcopy(case)
        for path, value in fix.items():
            if path in ("drop", "reason"):
                continue
            *parents, leaf = path.split(".")
            target = case
            for key in parents:
                target = target[key]
            if leaf not in target:
                raise ValueError(f"{case['id']}: unknown field {path}")
            target[leaf] = value
        result.append(case)
    return result


def coverage_problems(cases: list[dict[str, Any]]) -> list[str]:
    problems = []
    profiles = [case["expected"]["victim_profile"] for case in cases]
    categories = Counter(case["category"] for case in cases)
    if dict(categories) != CATEGORY_COUNTS:
        problems.append(f"category counts {dict(categories)} != {CATEGORY_COUNTS}")
    languages = Counter(case["language"] for case in cases)
    for language in ("ml", "manglish"):
        if languages[language] < 24:
            problems.append(f"only {languages[language]} {language} cases")
    medical = Counter(m for p in profiles for m in p["medical_conditions"])
    for condition in CRITICAL_CONDITIONS:
        if medical[condition] < 10:
            problems.append(f"only {medical[condition]} {condition} cases")
    if sum(p["reporter_on_scene"] is False for p in profiles) < 10:
        problems.append("fewer than 10 remote callers")
    missing = RELATIONS - {p["reporter_relation"] for p in profiles}
    if missing:
        problems.append(f"relations missing: {missing}")
    tiers = {case["expected_urgency"] for case in cases}
    if tiers != {"CRITICAL", "URGENT", "STABLE", "UNKNOWN"}:
        problems.append(f"urgency tiers present: {tiers}")
    return problems


def other_texts() -> tuple[set[str], set[str]]:
    """Inputs and places of every other dataset and of the prompt examples."""
    inputs: set[str] = set()
    places: set[str] = set()
    for path in OTHER_SETS:
        for case in load_cases(path):
            inputs.add(case["input"])
            places.add(case["expected"]["victim_location"] or "")
    for path in SFT_EXPORTS:
        if path.exists():
            for record in load_cases(path):
                inputs.add(record["messages"][1]["content"])
                places.add(json.loads(record["messages"][2]["content"]).get("victim_location") or "")
    for line in PROMPT.read_text(encoding="utf-8").splitlines():
        if line.startswith('Report: "'):
            inputs.add(line.removeprefix('Report: "').removesuffix('"'))
        elif line.startswith("{"):
            places.add(json.loads(line)["victim_location"] or "")
    return inputs, {p.casefold() for p in places if p}


# Between datasets, short shared phrases ("and the water is rising") are normal
# flood language, not copying; a shared 8-word run is treated as reuse.
DATASET_NGRAM = 8


def separation_problems(cases: list[dict[str, Any]]) -> list[str]:
    inputs, places = other_texts()
    other_grams = set().union(*(ngrams(text, DATASET_NGRAM) for text in inputs))
    problems = []
    for case in cases:
        if case["input"] in inputs:
            problems.append(f"{case['id']}: input reused")
        place = (case["expected"]["victim_location"] or "").casefold()
        if place and place in places:
            problems.append(f"{case['id']}: place reused: {place}")
        shared = ngrams(case["input"], DATASET_NGRAM) & other_grams
        if shared:
            problems.append(f"{case['id']}: shares wording {sorted(shared)[:2]}")
    return problems


def reviewed_cases() -> list[dict[str, Any]]:
    corrections = load_json(CORRECTIONS) if CORRECTIONS.exists() else {}
    cases = apply_corrections(load_cases(CANDIDATES), corrections)
    for case in cases:
        case["reviewer_status"] = "reviewed"
        case.pop("annotator_note", None)
    return cases


def freeze(frozen_on: str, review_note: str) -> dict[str, Any]:
    """Freeze after owner review: validate, check coverage and separation, write manifest."""
    cases = reviewed_cases()
    validate_cases(cases)
    problems = coverage_problems(cases) + separation_problems(cases)
    if problems:
        raise ValueError("cannot freeze:\n" + "\n".join(problems))
    write_cases(cases, FROZEN)
    manifest = {
        "dataset": "CrisisGraph Triage Eval V3.1",
        "version": "3.1.0",
        "artifact": FROZEN.name,
        "sha256": sha256(FROZEN),
        "record_count": len(cases),
        "frozen_on": frozen_on,
        "frozen_benchmark": True,
        "reviewer_status": "reviewed",
        "authoring": (
            "Drafted by a zero-context annotator agent from the guidelines only (no access to "
            "other datasets or the urgency rules), before any SFT-V4 training data existed."
        ),
        "review": review_note,
        "candidates_sha256": sha256(CANDIDATES),
        "corrections_sha256": sha256(CORRECTIONS) if CORRECTIONS.exists() else None,
        "contract_sha256": {name: sha256(path) for name, path in CONTRACT_FILES.items()},
        "counts": {
            "category": dict(Counter(c["category"] for c in cases)),
            "language": dict(Counter(c["language"] for c in cases)),
            "expected_urgency": dict(Counter(c["expected_urgency"] for c in cases)),
        },
        "permitted_use": ["final model evaluation", "regression testing", "cross-model comparison"],
        "prohibited_use": [
            "model training",
            "fine-tuning",
            "prompt tuning",
            "training-example generation",
        ],
        "required_reporting": ["overall", "per category", "per language"],
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="report coverage and separation")
    parser.add_argument("--freeze", action="store_true")
    parser.add_argument("--frozen-on")
    parser.add_argument("--review-note", default="")
    args = parser.parse_args()
    if args.freeze:
        manifest = freeze(args.frozen_on, args.review_note)
        print(f"Froze {manifest['record_count']} cases, sha256 {manifest['sha256']}")
        return
    cases = reviewed_cases()
    validate_cases(cases)
    for problem in coverage_problems(cases) + separation_problems(cases):
        print(problem)
    print(f"{len(cases)} cases checked")


if __name__ == "__main__":
    main()
