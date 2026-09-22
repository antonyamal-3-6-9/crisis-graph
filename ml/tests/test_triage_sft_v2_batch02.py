from copy import deepcopy

import pytest

from crisisgraph_ml.triage_sft_v2 import build_review_batch as batch01
from crisisgraph_ml.triage_sft_v2_batch02 import build_review_batch, validate_batch


def test_batch02_passes_and_remains_pending():
    rows = build_review_batch()
    validate_batch(rows, batch01())
    assert all(r["reviewer_status"] == "pending_manual_review" for r in rows)
    assert len(batch01() + rows) == 48


def test_reference_family_reuse_rejected_even_with_new_wording():
    rows = build_review_batch()
    rows[0]["scenario_family"] = batch01()[0]["scenario_family"]
    with pytest.raises(ValueError, match="reuses"):
        validate_batch(rows, batch01())


def test_confidence_mapping_checked():
    rows = build_review_batch()
    rows[0]["target"]["confidence_score"] = 0.95
    with pytest.raises(ValueError, match="Confidence"):
        validate_batch(rows, batch01())


def test_clock_timestamp_never_becomes_duration():
    rows = [r for r in build_review_batch() if r["category"] == "hazard_missing_duration"]
    assert len(rows) == 3
    for row in rows:
        assert row["target"]["hazards"][0]["duration_hours"] is None
        assert row["target"]["needs_human_review"] is True


def test_missing_count_and_location_keeps_requested_asset():
    rows = [r for r in build_review_batch() if r["category"] == "missing_multiple_fields"]
    assert len(rows) == 3
    for row in rows:
        value = row["target"]
        assert value["headcount"] is None and value["victim_location"] is None
        assert value["required_asset"] == "RESCUE_BOAT"
        assert len(value["uncertainty_reasons"]) == 2


def test_no_mutation_of_first_batch():
    original = batch01()
    snapshot = deepcopy(original)
    validate_batch(build_review_batch(), original)
    assert original == snapshot
