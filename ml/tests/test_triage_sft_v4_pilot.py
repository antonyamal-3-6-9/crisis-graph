from collections import Counter

import pytest

from crisisgraph_ml.triage_sft_v4_pilot import candidate_rows, validate


def test_v4_counterexample_pilot_allocation_and_schema():
    rows = candidate_rows()
    validate(rows)
    assert len(rows) == 12
    assert Counter(row["language"] for row in rows) == {
        "en": 4,
        "ml": 4,
        "manglish": 4,
    }
    assert Counter(row["category"] for row in rows) == {
        "explicit_asset_clear": 6,
        "adversarial_false_hazard": 6,
    }
    assert Counter(row["target"]["required_asset"] for row in rows) == {
        "AMBULANCE": 3,
        "RESCUE_BOAT": 3,
        "EVAC_TRUCK": 3,
        "HELICOPTER": 3,
    }


def test_v4_pilot_is_complete_hazard_free_and_pending_review():
    for row in candidate_rows():
        assert row["split"] == "dev"
        assert row["target"]["hazards"] == []
        assert row["target"]["needs_human_review"] is False
        assert row["target"]["uncertainty_reasons"] == []
        assert row["reviewer_status"] == "pending_manual_review"


def test_v4_validation_rejects_duplicate_scenario_family():
    rows = candidate_rows()
    rows[1]["scenario_family"] = rows[0]["scenario_family"]
    with pytest.raises(ValueError, match="Duplicate scenario_family"):
        validate(rows)
