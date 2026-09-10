from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

from crisisgraph_ml.generate_triage_eval import validate_cases
from crisisgraph_ml.triage_eval import DEFAULT_SCHEMA, load_cases, load_json


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate a CrisisGraph triage dataset")
    parser.add_argument("dataset", type=Path)
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    cases = load_cases(args.dataset)
    validate_cases(cases, load_json(args.schema))
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
