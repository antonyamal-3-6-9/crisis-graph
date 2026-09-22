from copy import deepcopy

import pytest

from crisisgraph_ml.triage_sft_v2 import build_review_batch, references, validate_records


def test_batch_schema_and_reference_overlap():
    rows = build_review_batch()
    validate_records(rows, references())
    assert len(rows) == 24
    assert len({r["scenario_family"] for r in rows}) == 8
    assert all(r["reviewer_status"] == "pending_manual_review" for r in rows)


def test_conflict_cannot_silently_bypass_review():
    rows = build_review_batch()
    rows[0]["target"]["needs_human_review"] = False
    with pytest.raises(ValueError, match="must require review"):
        validate_records(rows, [])


def test_family_leak_detected_even_when_text_changes():
    rows = build_review_batch()
    other = deepcopy(rows[0])
    other.update(split="train", input="Different wording for the same held-out scenario")
    with pytest.raises(ValueError, match="family occurs across splits"):
        validate_records(rows, [other])


def test_exact_reference_overlap_rejected():
    rows = build_review_batch()
    with pytest.raises(ValueError, match="overlap"):
        validate_records(rows, [{"input": rows[0]["input"]}])


def test_irrelevant_numbers_do_not_change_victim_count():
    rows = [r for r in build_review_batch() if r["category"] == "clear"]
    assert all(r["target"]["headcount"] == 23 for r in rows)
    assert all(not r["target"]["needs_human_review"] for r in rows)


def test_uncertain_road_retains_explicit_status_and_duration():
    rows = [r for r in build_review_batch() if r["category"] == "hazard_unanchored_road"]
    assert all(
        r["target"]["hazards"] == [{"road_segment": None, "status": "BLOCKED", "duration_hours": 2}]
        for r in rows
    )
