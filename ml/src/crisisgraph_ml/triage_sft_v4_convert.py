"""Convert the SFT-V3 training and validation records to V3 targets (SFT-V4).

Each record keeps its message and every V2 label exactly as reviewed. The
target becomes ``triage-extraction-v3`` with ``victim_profile`` last (the
serving field order); profile labels follow the V3 guideline and
``triage-annotation-guidelines-v3.1-changes.md``. The system prompt is set at
export (prompt v3.2), not stored here.

Most SFT-V3 messages are templated dispatch requests with no profile facts;
only the records listed in ``PROFILES`` have any. Dispatch-style relays ("one
report requests…", "staff describe…") do not say who is reporting, so their
``reporter_relation`` stays null.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator

from crisisgraph_ml.triage_eval import ML_ROOT, load_cases, load_json
from crisisgraph_ml.triage_eval_v3_dev import write_cases
from crisisgraph_ml.urgency import RANK, RULES_VERSION, derive_urgency

DATA = ML_ROOT / "data"
EXPORT_MANIFEST = DATA / "triage-sft-v3-export-manifest.json"
SCHEMA_V3 = ML_ROOT.parent / "schemas" / "triage-extraction-v3.json"
DEFAULT_OUTPUT = DATA / "triage-sft-v4-converted-candidates.jsonl"

SELF = {"reporter_relation": "SELF", "reporter_on_scene": True}

# Keys: t = SFT-V3 train, v = validation, then the record index in that export.
PROFILES: dict[str, dict[str, Any]] = {
    # "We have N people who urgently need …" (t005, t017 …) is coordinator
    # phrasing that does not place the speaker in the group: relation null
    # (guideline change 5), so those records keep an empty profile.
    "t264": SELF,  # "We are still here and 19 people need …"
    "t269": SELF,  # "Njangal ivide 15 perundu" (we are 15 here)
    # "Cannot walk without help": a lasting inability (guideline change 6).
    "t270": {"vulnerable_groups": ["DISABLED"]},
    "t271": {"medical_conditions": ["CARDIAC"]},  # "report chest pain"
    "t273": {"other_conditions": ["stranded on a roof"]},  # guideline change 1
    "t274": {"vulnerable_groups": ["ELDERLY"]},  # "വയോധികർ" (elderly)
    "t275": {"medical_conditions": ["INJURY"]},  # "9 injured aalukal"
    "v057": {"vulnerable_groups": ["DISABLED"]},  # "wheelchair users"
}
NEEDS_OWNER_REVIEW: set[str] = set()

EMPTY_PROFILE: dict[str, Any] = {
    "reporter_relation": None,
    "reporter_on_scene": None,
    "medical_conditions": [],
    "other_conditions": [],
    "vulnerable_groups": [],
    "water_level": None,
    "water_trend": None,
}

# Gold urgency for the records with facts (clinical sense, guideline tiers);
# every other record has no profile facts and is UNKNOWN.
# t273 (stranded on a roof, nothing else stated) is URGENT to match the frozen
# v3.1.0 gold for the same situation (case 105); the rules make it CRITICAL.
URGENCY: dict[str, str] = {"t271": "CRITICAL", "t273": "URGENT", "t275": "URGENT"}

MALAYALAM = re.compile(r"[ഀ-ൿ]")
MANGLISH_WORDS = re.compile(
    r"\b(venam|aanu|perku|perundu|undu|njangal|ivide|aayi|aaya|ennum|paranjittilla|"
    r"cheythu|pakshe|ariyilla|ulla|aalukalkku|chodikkunnu|aakku|etha)\b",
    re.IGNORECASE,
)


def language(text: str) -> str:
    if MALAYALAM.search(text):
        return "ml"
    return "manglish" if MANGLISH_WORDS.search(text) else "en"


def verified_exports() -> dict[str, list[dict[str, Any]]]:
    """SFT-V3 exports, checked against the hashes in their manifest."""
    manifest = load_json(EXPORT_MANIFEST)
    exports = {}
    for split, entry in manifest["exports"].items():
        path = ML_ROOT / entry["output"].removeprefix("ml/")
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if digest != entry["output_sha256"]:
            raise ValueError(f"{path} does not match the SFT-V3 export manifest")
        exports[split] = load_cases(path)
    return exports


def suggested_urgency(gold: str) -> str | None:
    """Spec rule: the gold tier, or null when it is UNKNOWN."""
    return None if gold == "UNKNOWN" else gold


def convert(key: str, split: str, record: dict[str, Any]) -> dict[str, Any]:
    _, user, assistant = record["messages"]
    v2 = json.loads(assistant["content"])
    gold = URGENCY.get(key, "UNKNOWN")
    profile = {**EMPTY_PROFILE, **PROFILES.get(key, {}),
               "suggested_urgency": suggested_urgency(gold)}
    target = {**v2, "schema_version": "triage-extraction-v3"}
    target.pop("victim_profile", None)
    target["victim_profile"] = profile  # last: matches the serving field order
    return {
        "id": f"sft-v4-conv-{key}",
        "split": "train" if split == "train" else "validation",
        "source": f"triage-sft-v3-{split}.jsonl#{key[1:]}",
        "language": language(user["content"]),
        "reviewer_status": "pending_manual_review"
        if key in PROFILES else "v2_labels_reviewed_empty_profile",
        "input": user["content"],
        "target": target,
        "expected_urgency": gold,
    }


def build_records() -> list[dict[str, Any]]:
    exports = verified_exports()
    records = []
    for split, prefix in (("train", "t"), ("validation", "v")):
        for index, record in enumerate(exports[split]):
            records.append(convert(f"{prefix}{index:03d}", split, record))
    unknown = (set(PROFILES) | set(URGENCY)) - {r["id"].removeprefix("sft-v4-conv-")
                                                 for r in records}
    if unknown:
        raise ValueError(f"labels for unknown records: {sorted(unknown)}")
    return records


def validate_records(records: list[dict[str, Any]], rules: str = RULES_VERSION) -> None:
    validator = Draft202012Validator(load_json(SCHEMA_V3))
    for record in records:
        errors = list(validator.iter_errors(record["target"]))
        if errors:
            raise ValueError(f"{record['id']}: {errors[0].message}")
        if list(record["target"])[-1] != "victim_profile":
            raise ValueError(f"{record['id']}: victim_profile must be the last field")
        # A target must never teach under-triage relative to its own gold.
        derived = derive_urgency(record["target"]["victim_profile"], rules)
        if RANK[derived.urgency] < RANK[record["expected_urgency"]]:
            raise ValueError(f"{record['id']}: target urgency below gold")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    records = build_records()
    validate_records(records)
    write_cases(records, args.output)
    rich = [r for r in records if r["id"].removeprefix("sft-v4-conv-") in PROFILES]
    print(f"Wrote {len(records)} records ({len(rich)} with profile facts) to {args.output}")
    print(dict(Counter((r["split"], r["language"]) for r in records)))


if __name__ == "__main__":
    main()
