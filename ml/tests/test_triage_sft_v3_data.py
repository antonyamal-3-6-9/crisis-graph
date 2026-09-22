from collections import Counter

import pytest

from crisisgraph_ml.triage_sft_v3_data import train_rows, validate, validation_rows


def datasets():
    return {
        "train_augmentation": train_rows(),
        "validation_extension": validation_rows(),
    }


def test_v3_targeted_addition_allocations_and_schema():
    values = datasets()
    validate(values)
    assert len(values["train_augmentation"]) == 60
    assert len(values["validation_extension"]) == 24
    assert Counter(row["language"] for row in values["train_augmentation"]) == {
        "en": 36,
        "ml": 12,
        "manglish": 12,
    }
    assert Counter(row["language"] for row in values["validation_extension"]) == {
        "en": 12,
        "ml": 6,
        "manglish": 6,
    }


def test_v3_all_candidates_are_pending_review_and_families_are_unique():
    rows = [row for values in datasets().values() for row in values]
    assert all(row["reviewer_status"] == "pending_manual_review" for row in rows)
    assert len({row["scenario_family"] for row in rows}) == 84
    assert len({row["wording_family"] for row in rows}) == 84


def test_v3_validation_rejects_duplicate_family():
    values = datasets()
    values["validation_extension"][0]["scenario_family"] = values[
        "train_augmentation"
    ][0]["scenario_family"]
    with pytest.raises(ValueError, match="Duplicate scenario_family"):
        validate(values)
