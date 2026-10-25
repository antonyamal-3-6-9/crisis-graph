"""V3 labels for the frozen V2 benchmark inputs (V2 non-regression set).

Reads ``triage-eval-v2.0.0.jsonl`` (verified against its manifest checksum),
keeps every V2 label unchanged and adds ``victim_profile`` and
``expected_urgency``. The frozen V2 file itself is never modified.

Most V2 messages state no profile facts, so their profile is all unknown; this
measures whether the model invents profile facts on plain requests. Only the
cases listed in ``PROFILES`` / ``URGENCY`` differ from that default.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any

from crisisgraph_ml.triage_eval import ML_ROOT, load_cases, load_json
from crisisgraph_ml.triage_eval_v3_dev import validate_cases, write_cases

V2_FROZEN = ML_ROOT / "data" / "triage-eval-v2.0.0.jsonl"
V2_MANIFEST = ML_ROOT / "data" / "triage-eval-v2.0.0.manifest.json"
DEFAULT_OUTPUT = ML_ROOT / "data" / "triage-eval-v3-v2-carryover.jsonl"

SELF = {"reporter_relation": "SELF", "reporter_on_scene": True}
INJURED = {"medical_conditions": ["INJURY"]}
# "Critical patient" states severity but no category: kept in other_conditions so
# it is not lost (URGENT by rule, flagged for review).
CRITICAL_PATIENT = {"other_conditions": ["critical condition"]}

PROFILES: dict[str, dict[str, Any]] = {
    "v2-clear-001": INJURED,
    "v2-clear-005": SELF,
    "v2-clear-006": {"vulnerable_groups": ["ELDERLY"]},
    # "We need an ambulance" puts the reporter with the child, but headcount 1
    # means the reporter is not a victim, so the relation is unknown.
    "v2-clear-013": {
        "reporter_on_scene": True,
        "medical_conditions": ["INJURY"],
        "vulnerable_groups": ["CHILD"],
    },
    "v2-clear-016": CRITICAL_PATIENT,
    "v2-missing-information-001": SELF,
    "v2-missing-information-002": INJURED,
    "v2-missing-information-004": SELF,
    "v2-missing-information-005": SELF,
    "v2-missing-information-015": INJURED,
    "v2-missing-information-018": CRITICAL_PATIENT,
    "v2-contradictory-004": SELF,
    "v2-hazard-012": INJURED,
    "v2-multilingual-manglish-004": CRITICAL_PATIENT,
}

URGENCY: dict[str, str] = {
    "v2-clear-001": "URGENT",
    "v2-clear-008": "CRITICAL",
    "v2-clear-013": "URGENT",
    "v2-clear-016": "CRITICAL",
    "v2-missing-information-002": "URGENT",
    "v2-missing-information-015": "URGENT",
    "v2-missing-information-018": "CRITICAL",
    "v2-missing-information-019": "CRITICAL",
    "v2-hazard-012": "URGENT",
    "v2-multilingual-manglish-004": "CRITICAL",
}

EMPTY_PROFILE = {
    "reporter_relation": None,
    "reporter_on_scene": None,
    "medical_conditions": [],
    "other_conditions": [],
    "vulnerable_groups": [],
    "water_level": None,
    "water_trend": None,
}


def load_v2() -> list[dict[str, Any]]:
    expected = load_json(V2_MANIFEST)["sha256"]
    actual = hashlib.sha256(V2_FROZEN.read_bytes()).hexdigest()
    if actual != expected:
        raise ValueError(f"frozen V2 benchmark changed: {actual} != {expected}")
    return load_cases(V2_FROZEN)


def build_cases() -> list[dict[str, Any]]:
    cases = []
    v2_ids = set()
    for v2 in load_v2():
        v2_ids.add(v2["id"])
        expected = dict(v2["expected"])
        expected["victim_profile"] = {**EMPTY_PROFILE, **PROFILES.get(v2["id"], {})}
        cases.append(
            {
                "id": f"v3-carryover-{v2['id'].removeprefix('v2-')}",
                "source_id": v2["id"],
                "category": v2["category"],
                "language": v2["language"],
                "scenario_family": v2["scenario_family"],
                "provenance": "triage-eval-v2.0.0 input with V3 profile labels",
                "reviewer_status": "pending_manual_review",
                "input": v2["input"],
                "expected": expected,
                "expected_urgency": URGENCY.get(v2["id"], "UNKNOWN"),
            }
        )
    unknown = (set(PROFILES) | set(URGENCY)) - v2_ids
    if unknown:
        raise ValueError(f"labels for unknown V2 ids: {sorted(unknown)}")
    return cases


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    cases = build_cases()
    validate_cases(cases)
    write_cases(cases, args.output)
    labelled = sum(c["expected"]["victim_profile"] != EMPTY_PROFILE for c in cases)
    print(f"Wrote {len(cases)} cases ({labelled} with profile facts) to {args.output}")
    print(json.dumps(dict(Counter(c["expected_urgency"] for c in cases))))


if __name__ == "__main__":
    main()
