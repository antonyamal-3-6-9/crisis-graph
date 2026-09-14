from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

from crisisgraph_ml.generate_triage_eval import REVIEWER_STATUSES, validate_cases
from crisisgraph_ml.triage_eval import DEFAULT_SCHEMA, load_cases, load_json


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate a CrisisGraph triage dataset")
    parser.add_argument("dataset", type=Path)
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    parser.add_argument(
        "--require-review-status",
        choices=sorted(REVIEWER_STATUSES),
        help="Fail unless every record has this review workflow status",
    )
    return parser.parse_args()


def require_review_status(cases: list[dict], required_status: str | None) -> None:
    if required_status is None:
        return
    mismatches = [case["id"] for case in cases if case["reviewer_status"] != required_status]
    if mismatches:
        preview = ", ".join(mismatches[:10])
        suffix = "" if len(mismatches) <= 10 else f", and {len(mismatches) - 10} more"
        raise ValueError(
            f"{len(mismatches)} records do not have reviewer_status={required_status!r}: "
            f"{preview}{suffix}"
        )


def main() -> None:
    args = parse_args()
    cases = load_cases(args.dataset)
    validate_cases(cases, load_json(args.schema))
    require_review_status(cases, args.require_review_status)
    print(
        json.dumps(
            {
                "dataset": str(args.dataset),
                "case_count": len(cases),
                "categories": Counter(case["category"] for case in cases),
                "languages": Counter(case["language"] for case in cases),
                "reviewer_status": Counter(case["reviewer_status"] for case in cases),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
