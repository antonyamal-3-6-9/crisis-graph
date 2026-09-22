"""Versioned V2 train/validation candidates, with dev families held out."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator

from crisisgraph_ml.generate_triage_eval import REVIEWER_STATUSES, write_jsonl
from crisisgraph_ml.triage_eval import DEFAULT_SCHEMA, load_cases, load_json
from crisisgraph_ml.triage_sft_data import (
    ASSET_TEXT,
    ASSETS,
    ML_ROOT,
    hazard,
    normalize_text,
    target,
)

CATALOG = ML_ROOT / "data/triage-sft-v2-scenario-catalog.json"
COUNTS = {
    "train": {"en": 144, "ml": 48, "manglish": 48},
    "validation": {"en": 24, "ml": 12, "manglish": 12},
}
REPEATS = {
    "train": {"en": 9, "ml": 3, "manglish": 3},
    "validation": {"en": 2, "ml": 1, "manglish": 1},
}
FAMILY_COUNTS = {"train": 16, "validation": 12}
PLACE_NAMES = {
    "train": (
        ("Larch Rest Hall", "ലാർച്ച് വിശ്രമ ഹാൾ"),
        ("Mango Workers Hostel", "മാംഗോ തൊഴിലാളി ഹോസ്റ്റൽ"),
        ("Rose Training Centre", "റോസ് പരിശീലന കേന്ദ്രം"),
        ("Bamboo Reading Room", "ബാംബൂ വായനശാല"),
        ("Lotus Meeting Hall", "ലോട്ടസ് യോഗ ഹാൾ"),
        ("Palm Crafts Centre", "പാം കരകൗശല കേന്ദ്രം"),
        ("Teak Sports Hall", "ടീക്ക് കായിക ഹാൾ"),
        ("Orchid Guest House", "ഓർക്കിഡ് അതിഥി മന്ദിരം"),
        ("Pine Study Centre", "പൈൻ പഠന കേന്ദ്രം"),
        ("Jasmine Rest House", "ജാസ്മിൻ വിശ്രമ മന്ദിരം"),
        ("Coconut Youth Hall", "കോക്കനട്ട് യുവജന ഹാൾ"),
        ("Fig Assembly Hall", "ഫിഗ് സമ്മേളന ഹാൾ"),
    ),
    "validation": (
        ("Silver Community Room", "സിൽവർ പൊതുമുറി"),
        ("Golden Recreation Hall", "ഗോൾഡൻ വിനോദ ഹാൾ"),
        ("Violet Learning Centre", "വയലറ്റ് പഠന കേന്ദ്രം"),
        ("Indigo Rest Pavilion", "ഇൻഡിഗോ വിശ്രമ പവിലിയൻ"),
        ("Bronze Welfare Hall", "ബ്രോൺസ് ക്ഷേമ ഹാൾ"),
        ("Saffron Guest Lodge", "സാഫ്രൺ അതിഥി ലോഡ്ജ്"),
        ("Azure Activity Centre", "അഷ്വർ പ്രവർത്തന കേന്ദ്രം"),
        ("Ivory Reading Hall", "ഐവറി വായന ഹാൾ"),
        ("Ochre Meeting Room", "ഓക്കർ യോഗ മുറി"),
        ("Lilac Training Hall", "ലൈലാക് പരിശീലന ഹാൾ"),
        ("Maroon Cultural Centre", "മറൂൺ സാംസ്കാരിക കേന്ദ്രം"),
        ("Scarlet Assembly Room", "സ്കാർലറ്റ് സമ്മേളന മുറി"),
    ),
}
ROADS = {
    "train": ("Larch Link Road", "ലാർച്ച് ലിങ്ക് റോഡ്"),
    "validation": ("Silver Cross Road", "സിൽവർ ക്രോസ് റോഡ്"),
}
REASONS = {
    "count_conflict": "Conflicting headcounts refer to the same group",
    "count_uncertain": "The exact headcount is uncertain",
    "location_conflict": "Pickup location is unresolved between two places",
    "asset_conflict": "The requested asset is unresolved between two choices",
    "asset_missing": "No specific asset was requested",
    "count_missing": "Headcount was not provided",
    "location_missing": "Pickup location was not provided",
    "hazard_unanchored": "The affected road is not identified or explicitly anchored",
    "hazard_missing_duration": "Hazard duration was not provided",
    "irrelevant": "The message contains no emergency dispatch facts",
}
RULES = set(REASONS) | {"complete", "hazard_complete"}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def signature(text: str) -> str:
    return hashlib.sha256(normalize_text(text).encode()).hexdigest()


def build_split(split: str, catalog: dict[str, Any]) -> list[dict[str, Any]]:
    specs = [s for s in catalog["scenarios"] if s["split"] == split]
    if len(specs) != FAMILY_COUNTS[split]:
        raise ValueError("Unexpected scenario catalog size")
    rows = []
    for index, spec in enumerate(specs):
        rule = spec["rule"]
        if rule not in RULES:
            raise ValueError(f"Unknown annotation rule: {rule}")
        for language, repeats in REPEATS[split].items():
            lang_index = 1 if language == "ml" else 0
            template = spec["texts"][language]
            for variant in range(repeats):
                places = PLACE_NAMES[split]
                place = places[(index + variant) % len(places)][lang_index]
                alternate = places[(index + variant + 5) % len(places)][lang_index]
                count = 3 + (index * 7 + variant * 3) % 83
                asset = ASSETS[(index + variant) % len(ASSETS)]
                other_asset = ASSETS[(index + variant + 1) % len(ASSETS)]
                slots = {
                    "place": place,
                    "alt_place": alternate,
                    "count": count,
                    "other_count": count + 4,
                    "asset": ASSET_TEXT[language][asset],
                    "other_asset": ASSET_TEXT[language][other_asset],
                    "road": ROADS[split][lang_index],
                    "hours": 1 + variant % 7,
                }
                location_value = (
                    None
                    if rule in {"location_conflict", "location_missing", "irrelevant"}
                    else place
                )
                count_value = (
                    None
                    if rule in {"count_conflict", "count_uncertain", "count_missing", "irrelevant"}
                    else count
                )
                asset_value = (
                    None if rule in {"asset_conflict", "asset_missing", "irrelevant"} else asset
                )
                hazards = []
                if rule in {"hazard_complete", "hazard_missing_duration", "hazard_unanchored"}:
                    hazards = [
                        hazard(
                            None if rule == "hazard_unanchored" else slots["road"],
                            "FLOODED" if rule == "hazard_unanchored" else "BLOCKED",
                            None if rule == "hazard_missing_duration" else slots["hours"],
                        )
                    ]
                expected = target(
                    location_value,
                    count_value,
                    asset_value,
                    hazards,
                    [REASONS[rule]] if rule in REASONS else [],
                )
                if rule == "irrelevant":
                    expected["confidence_score"] = 0.10
                rendered = template.format(**slots)
                if language == "en":
                    rendered = rendered[:1].upper() + rendered[1:]
                    rendered = re.sub(r"\b1 hours\b", "1 hour", rendered)
                rows.append(
                    {
                        "id": f"sft-v2-{split}-{language}-{spec['scenario_family']}-{variant + 1:02}",
                        "split": split,
                        "category": spec["category"],
                        "language": language,
                        "scenario_family": spec["scenario_family"],
                        "template_family": spec["scenario_family"],
                        "template_sha256": signature(template),
                        "variant": variant + 1,
                        "candidate_revision": 2,
                        "provenance": "assistant_authored_synthetic_catalog_with_deterministic_slot_variation",
                        "reviewer_status": "pending_manual_review",
                        "input": rendered,
                        "target": expected,
                    }
                )
    return rows


def reference_paths() -> list[Path]:
    data = ML_ROOT / "data"
    return [
        data / "triage-eval-v2.0.0.jsonl",
        *sorted(data.glob("triage-sft-v1-*-candidates.jsonl")),
        *sorted(data.glob("triage-sft-v2-dev-review-batch-*.jsonl")),
    ]


def validate_datasets(datasets: dict[str, list[dict]], reference_rows: list[dict]) -> None:
    if set(datasets) != set(COUNTS):
        raise ValueError("Both train and validation splits are required")
    validator = Draft202012Validator(load_json(DEFAULT_SCHEMA))
    seen_ids = {r["id"] for r in reference_rows}
    seen_texts = {normalize_text(r["input"]) for r in reference_rows}
    owners: dict[tuple[str, str], str] = {}
    for row in reference_rows + [r for rows in datasets.values() for r in rows]:
        for field in ("scenario_family", "template_family", "template_sha256"):
            if value := row.get(field):
                key = (field, value)
                split = row.get("split", "reference")
                if owners.setdefault(key, split) != split:
                    raise ValueError(f"Cross-split leakage: {field} {value}")
    for split, rows in datasets.items():
        if Counter(r["language"] for r in rows) != COUNTS[split]:
            raise ValueError(f"Wrong language counts: {split}")
        if len({r["scenario_family"] for r in rows}) != FAMILY_COUNTS[split]:
            raise ValueError(f"Wrong family count: {split}")
        for row in rows:
            if row["split"] != split:
                raise ValueError("Split metadata mismatch")
            if row["id"] in seen_ids:
                raise ValueError("Duplicate record ID")
            seen_ids.add(row["id"])
            normalized = normalize_text(row["input"])
            if normalized in seen_texts:
                raise ValueError("Exact text overlap with another split or reference")
            seen_texts.add(normalized)
            if re.search(
                r"\{(?:place|count|asset|road|hours|alt_place|other_count|other_asset)\}",
                row["input"],
            ):
                raise ValueError("Unrendered template slot")
            if row["reviewer_status"] not in REVIEWER_STATUSES:
                raise ValueError("Invalid review status")
            value = row["target"]
            validator.validate(value)
            incomplete = any(
                value[f] is None for f in ("victim_location", "headcount", "required_asset")
            ) or any(v is None for h in value["hazards"] for v in h.values())
            if (incomplete or row["category"].startswith("contradictory")) and not value[
                "needs_human_review"
            ]:
                raise ValueError("Incomplete or contradictory record must require review")
            if bool(value["uncertainty_reasons"]) != value["needs_human_review"]:
                raise ValueError("Review decision and reasons disagree")
            confidence = (
                0.10
                if row["category"] == "irrelevant"
                else 0.55
                if value["needs_human_review"]
                else 0.95
            )
            if value["confidence_score"] != confidence:
                raise ValueError("Confidence label differs from policy")
            if row["category"] == "irrelevant" and (
                any(
                    value[f] is not None for f in ("victim_location", "headcount", "required_asset")
                )
                or value["hazards"]
                or not value["needs_human_review"]
            ):
                raise ValueError("Irrelevant message contains dispatch facts")


def paths(directory: Path) -> dict[str, Path]:
    return {split: directory / f"triage-sft-v2-{split}-candidates.jsonl" for split in COUNTS}


def generate(directory: Path, catalog_path: Path = CATALOG) -> dict[str, Path]:
    outputs = paths(directory)
    manifest_path = directory / "triage-sft-v2-train-validation.manifest.json"
    if any(p.exists() for p in [*outputs.values(), manifest_path]):
        raise FileExistsError("Refusing to overwrite candidates, manifests, or review history")
    catalog = load_json(catalog_path)
    datasets = {split: build_split(split, catalog) for split in COUNTS}
    refs = reference_paths()
    validate_datasets(datasets, [r for p in refs for r in load_cases(p)])
    for split, path in outputs.items():
        write_jsonl(datasets[split], path)
    manifest = {
        "dataset": "CrisisGraph SFT V2 train and validation candidates",
        "candidate_revision": 2,
        "catalog_sha256": digest(catalog_path),
        "generator_sha256": digest(Path(__file__)),
        "provenance": "synthetic authored templates with repeated slot variants",
        "generation_uses_benchmark_records": False,
        "references_used_for_overlap_checks_only": True,
        "reference_sha256": {p.name: digest(p) for p in refs},
        "training_ready": False,
        "exported": False,
        "independent_review_complete": False,
        "files": {
            split: {
                "path": p.name,
                "sha256": digest(p),
                "records": len(datasets[split]),
                "language_counts": COUNTS[split],
                "semantic_families": FAMILY_COUNTS[split],
                "category_counts": dict(Counter(r["category"] for r in datasets[split])),
                "reviewer_status_counts": {"pending_manual_review": len(datasets[split])},
            }
            for split, p in outputs.items()
        },
        "limitations": [
            "240 training records expand 16 families; they are not 240 independent scenarios",
            "48 validation records expand 12 separate families; translations and variants are correlated",
            "No automated check proves semantic independence or Malayalam/Manglish fluency",
            "New reviewed blind holdout still required for final generalization claims",
        ],
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return outputs


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("generate", "validate"))
    parser.add_argument("--data-dir", type=Path, default=ML_ROOT / "data")
    args = parser.parse_args()
    if args.command == "generate":
        output = generate(args.data_dir)
    else:
        output = paths(args.data_dir)
        validate_datasets(
            {split: load_cases(p) for split, p in output.items()},
            [r for p in reference_paths() for r in load_cases(p)],
        )
    print(
        json.dumps({"mechanical_checks": "passed", "files": {s: str(p) for s, p in output.items()}})
    )


if __name__ == "__main__":
    main()
