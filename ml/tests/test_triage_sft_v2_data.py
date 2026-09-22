import json
from collections import Counter

import pytest

from crisisgraph_ml.triage_eval import load_cases, load_json
from crisisgraph_ml.triage_sft_data import DEFAULT_PROMPT, export_messages
from crisisgraph_ml.triage_sft_v2_data import (
    CATALOG,
    COUNTS,
    build_split,
    digest,
    generate,
    reference_paths,
    validate_datasets,
)


@pytest.fixture
def datasets():
    return {split: build_split(split, load_json(CATALOG)) for split in COUNTS}


def test_sizes_and_reference_isolation(datasets):
    refs = [r for p in reference_paths() for r in load_cases(p)]
    validate_datasets(datasets, refs)
    for split, rows in datasets.items():
        assert Counter(r["language"] for r in rows) == COUNTS[split]
        assert all(r["reviewer_status"] == "pending_manual_review" for r in rows)


def test_cross_split_family_leak_rejected(datasets):
    datasets["validation"][0]["scenario_family"] = datasets["train"][0]["scenario_family"]
    with pytest.raises(ValueError, match="Cross-split leakage"):
        validate_datasets(datasets, [])


def test_same_template_with_renamed_family_rejected(datasets):
    datasets["validation"][0]["template_sha256"] = datasets["train"][0]["template_sha256"]
    with pytest.raises(ValueError, match="Cross-split leakage"):
        validate_datasets(datasets, [])


def test_exact_benchmark_overlap_rejected(datasets):
    with pytest.raises(ValueError, match="Exact text overlap"):
        validate_datasets(datasets, [{"id": "heldout", "input": datasets["train"][0]["input"]}])


def test_missing_field_cannot_disable_review(datasets):
    row = next(r for r in datasets["train"] if r["category"] == "missing_asset")
    row["target"]["needs_human_review"] = False
    with pytest.raises(ValueError, match="must require review"):
        validate_datasets(datasets, [])


def test_confidence_mismatch_rejected(datasets):
    datasets["train"][0]["target"]["confidence_score"] = 0.10
    with pytest.raises(ValueError, match="Confidence"):
        validate_datasets(datasets, [])


def test_artifact_generation_protects_reviews_and_blocks_export(tmp_path):
    outputs = generate(tmp_path)
    manifest = json.loads((tmp_path / "triage-sft-v2-train-validation.manifest.json").read_text())
    for split, path in outputs.items():
        assert digest(path) == manifest["files"][split]["sha256"]
    old_hash = digest(outputs["train"])
    with pytest.raises(FileExistsError, match="Refusing to overwrite"):
        generate(tmp_path)
    assert digest(outputs["train"]) == old_hash
    for policy in ("independent", "pilot-first-pass"):
        with pytest.raises(ValueError, match="Refusing SFT export"):
            export_messages(outputs["train"], DEFAULT_PROMPT, tmp_path / "export.jsonl", policy)
    assert not (tmp_path / "export.jsonl").exists()


def test_adversarial_missing_count_does_not_become_one(datasets):
    rows = [r for r in datasets["train"] if r["category"] == "adversarial_missing_count"]
    assert len(rows) == 15
    assert all(r["target"]["headcount"] is None for r in rows)
    assert all(r["target"]["needs_human_review"] for r in rows)


def test_english_article_capitalization_and_duration_agreement(datasets):
    import re

    for rows in datasets.values():
        for row in rows:
            if row["language"] == "en":
                assert not row["input"][0].islower()
                assert not re.search(r"\b1 hours\b", row["input"])
                for item in row["target"]["hazards"]:
                    if item["duration_hours"] == 1:
                        assert "1 hour" in row["input"]
