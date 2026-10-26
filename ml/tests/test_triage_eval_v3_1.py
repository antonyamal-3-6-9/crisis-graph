import pytest

from crisisgraph_ml.review_sheet import render
from crisisgraph_ml.triage_eval_v3_1 import CANDIDATES, apply_corrections, coverage_problems
from crisisgraph_ml.triage_eval_v3_dev import build_cases as build_dev_cases


def sample():
    return build_dev_cases()[:3]


def test_corrections_change_only_named_fields():
    cases = sample()
    fixed = apply_corrections(cases, {
        cases[0]["id"]: {"expected.victim_profile.water_level": "WAIST",
                         "expected_urgency": "URGENT", "reason": "owner"},
    })
    assert fixed[0]["expected"]["victim_profile"]["water_level"] == "WAIST"
    assert fixed[0]["expected_urgency"] == "URGENT"
    assert fixed[1:] == cases[1:]
    assert cases[0]["expected"]["victim_profile"]["water_level"] is None  # input untouched


def test_drop_removes_a_case():
    cases = sample()
    assert len(apply_corrections(cases, {cases[1]["id"]: {"drop": True}})) == 2


@pytest.mark.parametrize("corrections, message", [
    ({"v3-dev-999": {"input": "x"}}, "unknown ids"),
    ({"v3-dev-001": {"expected.victim_profile.colour": "x"}}, "unknown field"),
])
def test_bad_corrections_are_rejected(corrections, message):
    with pytest.raises(ValueError, match=message):
        apply_corrections(sample(), corrections)


def test_coverage_reports_problems_on_a_wrong_mix():
    assert coverage_problems(sample())


def test_review_sheet_renders_every_case():
    sheet = render(sample(), "Title", "note", "source.py")
    assert sheet.count("| 00") == 3 and "Title" in sheet


@pytest.mark.skipif(not CANDIDATES.exists(), reason="candidates not delivered yet")
def test_candidates_are_valid_labels():
    from crisisgraph_ml.triage_eval import load_cases
    from crisisgraph_ml.triage_eval_v3_dev import validate_cases

    validate_cases(load_cases(CANDIDATES))
