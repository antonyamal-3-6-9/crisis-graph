"""Build the hash-pinned SFT-V4 exports (train and validation) for QLoRA.

Sources are the converted SFT-V3 records and the new SFT-V4 records, both
reviewed. Every record is served with prompt v3.2 as the system message
(spec option a: train and serve with the same prompt) and its V3 target, with
``victim_profile`` last, as the assistant message.

The export refuses to run unless the spec's targets hold (counts, profile-rich
share, positives per critical condition, every relation value) and no record
shares an input, place or wording with an evaluation set. Outputs go to
``data/generated`` (not committed); the manifest with hashes is committed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any

from crisisgraph_ml.triage_eval import ML_ROOT, load_cases
from crisisgraph_ml.triage_sft_data import normalize_text
from crisisgraph_ml.triage_sft_v4_convert import validate_records
from crisisgraph_ml.triage_sft_v4_new import EVAL_SETS, LEAK_AUDIT, separation_problems
from crisisgraph_ml.urgency import RULES_VERSION

DATA = ML_ROOT / "data"
GENERATED = DATA / "generated"
MANIFEST = DATA / "triage-sft-v4-export-manifest.json"
PROMPT = ML_ROOT / "prompts" / "triage-extraction-v3.2.txt"
SCHEMA = ML_ROOT.parent / "schemas" / "triage-extraction-v3.json"
SOURCES = [
    DATA / "triage-sft-v4-converted-candidates.jsonl",
    DATA / "triage-sft-v4-new-candidates.jsonl",
]
ACCEPTED_STATUSES = {"reviewed", "v2_labels_reviewed_empty_profile"}
CRITICAL_CONDITIONS = ("UNCONSCIOUS", "BREATHING_DIFFICULTY", "CARDIAC", "BLEEDING")
RELATIONS = {"SELF", "HOUSEHOLD", "UNDER_REVIEW", "THIRD_PARTY", None}

# Spec targets (data/triage-sft-v4-spec.md, revised 2026-10-28).
MIN_TRAIN = 590
MIN_PROFILE_RICH_SHARE = 0.50
MIN_CRITICAL_POSITIVES = 25


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def profile_rich(record: dict[str, Any]) -> bool:
    profile = record["target"]["victim_profile"]
    return any(value not in (None, []) for key, value in profile.items()
               if key != "suggested_urgency")


def load_records() -> dict[str, list[dict[str, Any]]]:
    records = [record for path in SOURCES for record in load_cases(path)]
    splits: dict[str, list[dict[str, Any]]] = {"train": [], "validation": []}
    ids, inputs = set(), set()
    for record in records:
        if record["reviewer_status"] not in ACCEPTED_STATUSES:
            raise ValueError(f"{record['id']}: not reviewed ({record['reviewer_status']})")
        if record["id"] in ids:
            raise ValueError(f"duplicate id: {record['id']}")
        text = normalize_text(record["input"])
        if text in inputs:
            raise ValueError(f"duplicate input: {record['id']}")
        ids.add(record["id"])
        inputs.add(text)
        splits[record["split"]].append(record)
    return splits


def coverage(records: list[dict[str, Any]]) -> dict[str, Any]:
    profiles = [record["target"]["victim_profile"] for record in records]
    return {
        "records": len(records),
        "profile_rich": sum(profile_rich(record) for record in records),
        "language": dict(Counter(record["language"] for record in records)),
        "expected_urgency": dict(Counter(record["expected_urgency"] for record in records)),
        "medical_conditions": dict(Counter(m for p in profiles for m in p["medical_conditions"])),
        "reporter_relation": dict(Counter(str(p["reporter_relation"]) for p in profiles)),
    }


def target_problems(train: list[dict[str, Any]]) -> list[str]:
    stats = coverage(train)
    problems = []
    if stats["records"] < MIN_TRAIN:
        problems.append(f"only {stats['records']} training records")
    if stats["profile_rich"] / stats["records"] < MIN_PROFILE_RICH_SHARE:
        problems.append(f"profile-rich share {stats['profile_rich']}/{stats['records']}")
    for condition in CRITICAL_CONDITIONS:
        if stats["medical_conditions"].get(condition, 0) < MIN_CRITICAL_POSITIVES:
            problems.append(f"only {stats['medical_conditions'].get(condition, 0)} {condition}")
    missing = RELATIONS - {r["target"]["victim_profile"]["reporter_relation"] for r in train}
    if missing:
        problems.append(f"relations missing: {missing}")
    return problems


def leak_problems(splits: dict[str, list[dict[str, Any]]]) -> list[str]:
    """New records: the full separation check. Converted records: exact inputs."""
    everything = splits["train"] + splits["validation"]
    new = [record for record in everything if record["id"].startswith("sft-v4-new-")]
    problems = separation_problems(new)
    eval_inputs = {normalize_text(case["input"]) for path in EVAL_SETS for case in load_cases(path)}
    problems += [f"{record['id']}: input is in an evaluation set" for record in everything
                 if normalize_text(record["input"]) in eval_inputs]
    return problems


def message_row(prompt: str, record: dict[str, Any]) -> dict[str, Any]:
    return {
        "messages": [
            {"role": "system", "content": prompt},
            {"role": "user", "content": record["input"]},
            {
                "role": "assistant",
                "content": json.dumps(record["target"], ensure_ascii=False, separators=(",", ":")),
            },
        ]
    }


def write_jsonl(rows: list[dict[str, Any]], path: Path) -> None:
    path.write_text(
        "".join(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n" for row in rows),
        encoding="utf-8",
    )


def export(output_dir: Path = GENERATED, manifest_path: Path = MANIFEST) -> dict[str, Any]:
    outputs = {split: output_dir / f"triage-sft-v4-{split}.jsonl"
               for split in ("train", "validation")}
    if manifest_path.exists() or any(path.exists() for path in outputs.values()):
        raise FileExistsError("refusing to overwrite SFT-V4 exports or the export manifest")
    splits = load_records()
    for records in splits.values():
        validate_records(records)
    problems = target_problems(splits["train"]) + leak_problems(splits)
    if problems:
        raise ValueError("cannot export:\n" + "\n".join(problems))

    prompt = PROMPT.read_text(encoding="utf-8").strip()
    output_dir.mkdir(parents=True, exist_ok=True)
    exports = {}
    for split, records in splits.items():
        rows = [message_row(prompt, record) for record in records]
        write_jsonl(rows, outputs[split])
        longest = max(rows, key=lambda row: sum(len(m["content"]) for m in row["messages"]))
        exports[split] = {
            "output": f"data/generated/{outputs[split].name}",
            "output_sha256": digest(outputs[split]),
            "longest_record_chars": sum(len(m["content"]) for m in longest["messages"]),
            **coverage(records),
        }
    manifest = {
        "dataset": "CrisisGraph Triage SFT V4",
        "purpose": "QLoRA on V3 targets for every field (data/triage-sft-v4-spec.md)",
        "output_format": "JSONL with system (prompt v3.2), user and assistant messages",
        "exporter": {"path": "src/crisisgraph_ml/triage_sft_v4_export.py",
                     "sha256": digest(Path(__file__))},
        "prompt": {"path": "prompts/triage-extraction-v3.2.txt", "sha256": digest(PROMPT)},
        "schema": {"path": "../schemas/triage-extraction-v3.json", "sha256": digest(SCHEMA)},
        "urgency_rules": RULES_VERSION,
        "source_hashes": {path.name: digest(path) for path in SOURCES},
        "leak_audit": {"path": f"data/{LEAK_AUDIT.name}", "sha256": digest(LEAK_AUDIT)},
        "exports": exports,
        "dev_included": False,
        "frozen_evaluation_included": False,
        "text_authorship": (
            "Converted records keep their SFT-V3 texts. New records: texts written by "
            "zero-context agents that never saw the evaluation sets (batches 1-5 rewritten "
            "from reviewed labels, batches 6-7 authored from the guidelines)."
        ),
    }
    manifest_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="check targets and leaks only")
    args = parser.parse_args()
    if args.check:
        splits = load_records()
        for records in splits.values():
            validate_records(records)
        for problem in target_problems(splits["train"]) + leak_problems(splits):
            print(problem)
        for split, records in splits.items():
            print(split, json.dumps(coverage(records)))
        return
    manifest = export()
    for split, entry in manifest["exports"].items():
        print(f"{split}: {entry['records']} records, sha256 {entry['output_sha256'][:12]}…")


if __name__ == "__main__":
    main()
