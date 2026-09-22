"""Build hash-pinned V3 SFT exports from reviewed V1 and V3 candidates."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any

from crisisgraph_ml.triage_eval import load_cases
from crisisgraph_ml.triage_sft_data import DEFAULT_PROMPT, ML_ROOT, normalize_text

DATA = ML_ROOT / "data"
GENERATED = DATA / "generated"
SOURCES = {
    "train": (
        DATA / "triage-sft-v1-train-candidates.jsonl",
        DATA / "triage-sft-v3-train-augmentation-candidates.jsonl",
    ),
    "validation": (
        DATA / "triage-sft-v1-validation-candidates.jsonl",
        DATA / "triage-sft-v3-validation-extension-candidates.jsonl",
    ),
}
EXPECTED_SOURCE_HASHES = {
    "triage-sft-v1-train-candidates.jsonl": "2849a1e6585e70350aa35c46b9986b3893998fccb7368f0f4447bfc76b63b02f",
    "triage-sft-v1-validation-candidates.jsonl": "d2cdf4929c2496223a9c27cb4fe4304cb333365ae0f9a47815c1410a8041e963",
    "triage-sft-v3-train-augmentation-candidates.jsonl": "7f1121a642aa1e5b7c9bd7702fe346723c1bae268f577e10292b88c3d615475f",
    "triage-sft-v3-validation-extension-candidates.jsonl": "b1defba87ba6bed9d9a1e21b65301708d3e3cb798fcb485c206260f33d01e30a",
}
EXPECTED_COUNTS = {"train": 300, "validation": 72}
EXPECTED_LANGUAGES = {
    "train": {"en": 180, "ml": 60, "manglish": 60},
    "validation": {"en": 36, "ml": 18, "manglish": 18},
}
ACCEPTED_STATUSES = {"first_pass_reviewed", "reviewed"}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_and_validate_sources() -> dict[str, list[dict[str, Any]]]:
    datasets: dict[str, list[dict[str, Any]]] = {}
    all_ids: set[str] = set()
    all_inputs: set[str] = set()
    family_owner: dict[str, str] = {}
    for split, paths in SOURCES.items():
        rows: list[dict[str, Any]] = []
        for path in paths:
            actual = digest(path)
            if actual != EXPECTED_SOURCE_HASHES[path.name]:
                raise ValueError(f"Source hash mismatch for {path.name}: {actual}")
            rows.extend(load_cases(path))
        if len(rows) != EXPECTED_COUNTS[split]:
            raise ValueError(f"Wrong {split} count: {len(rows)}")
        if Counter(row["language"] for row in rows) != EXPECTED_LANGUAGES[split]:
            raise ValueError(f"Wrong {split} language allocation")
        for row in rows:
            if row["split"] != split:
                raise ValueError(f"Split mismatch: {row['id']}")
            if row["reviewer_status"] not in ACCEPTED_STATUSES:
                raise ValueError(f"Unaccepted record: {row['id']}")
            if row["id"] in all_ids:
                raise ValueError(f"Duplicate ID: {row['id']}")
            all_ids.add(row["id"])
            text = normalize_text(row["input"])
            if text in all_inputs:
                raise ValueError(f"Duplicate input: {row['id']}")
            all_inputs.add(text)
            family = row.get("scenario_family")
            if family and family_owner.setdefault(family, split) != split:
                raise ValueError(f"Scenario family crosses splits: {family}")
        datasets[split] = rows

    excluded = []
    for path in [DATA / "triage-sft-v3-dev-candidates.jsonl", DATA / "triage-eval-v2.0.0.jsonl"]:
        excluded.extend(load_cases(path))
    excluded_ids = {row["id"] for row in excluded}
    excluded_inputs = {normalize_text(row["input"]) for row in excluded}
    if all_ids & excluded_ids:
        raise ValueError("Dev or frozen-evaluation ID entered V3 export")
    if all_inputs & excluded_inputs:
        raise ValueError("Dev or frozen-evaluation input entered V3 export")
    return datasets


def message_row(prompt: str, case: dict[str, Any]) -> dict[str, Any]:
    return {
        "messages": [
            {"role": "system", "content": prompt},
            {"role": "user", "content": case["input"]},
            {
                "role": "assistant",
                "content": json.dumps(case["target"], ensure_ascii=False, separators=(",", ":")),
            },
        ]
    }


def write_jsonl(rows: list[dict[str, Any]], path: Path) -> None:
    path.write_text(
        "".join(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n" for row in rows),
        encoding="utf-8",
    )


def export(output_dir: Path = GENERATED) -> dict[str, Path]:
    outputs = {split: output_dir / f"triage-sft-v3-{split}.jsonl" for split in SOURCES}
    manifest_path = (
        DATA / "triage-sft-v3-export-manifest.json"
        if output_dir.resolve() == GENERATED.resolve()
        else output_dir / "triage-sft-v3-export-manifest.json"
    )
    if manifest_path.exists() or any(path.exists() for path in outputs.values()):
        raise FileExistsError("Refusing to overwrite V3 exports or export manifest")
    datasets = load_and_validate_sources()
    prompt = DEFAULT_PROMPT.read_text(encoding="utf-8").strip()
    output_dir.mkdir(parents=True, exist_ok=True)
    for split, rows in datasets.items():
        write_jsonl([message_row(prompt, row) for row in rows], outputs[split])
    manifest = {
        "dataset": "CrisisGraph Triage SFT V3",
        "purpose": "controlled QLoRA comparison with V1 and V2",
        "review_policy": "pilot-first-pass",
        "review_type": "AI-assisted first pass with exact correction verification",
        "independent_review_complete": False,
        "output_format": "JSONL with system, user, and assistant messages",
        "prompt": {"path": "prompts/triage-extraction-v2.1.1.txt", "sha256": digest(DEFAULT_PROMPT)},
        "source_hashes": {path.name: digest(path) for paths in SOURCES.values() for path in paths},
        "exports": {
            split: {
                "output": f"data/generated/{path.name}",
                "output_sha256": digest(path),
                "records": len(datasets[split]),
                "language_counts": dict(Counter(row["language"] for row in datasets[split])),
            }
            for split, path in outputs.items()
        },
        "dev_included": False,
        "frozen_evaluation_included": False,
        "production_training_approval": False,
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return outputs


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("export", "validate-sources"))
    parser.add_argument("--output-dir", type=Path, default=GENERATED)
    args = parser.parse_args()
    if args.command == "validate-sources":
        datasets = load_and_validate_sources()
        print(json.dumps({split: len(rows) for split, rows in datasets.items()}))
    else:
        outputs = export(args.output_dir)
        print(json.dumps({split: str(path) for split, path in outputs.items()}))


if __name__ == "__main__":
    main()
