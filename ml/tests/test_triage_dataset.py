from collections import Counter

from crisisgraph_ml.generate_triage_eval import EXPECTED_COUNTS, build_cases, validate_cases
from crisisgraph_ml.triage_eval import DEFAULT_SCHEMA, load_json
from crisisgraph_ml.validate_triage_data import require_review_status


def test_generated_candidate_distribution_and_contract():
    cases = build_cases()
    validate_cases(cases, load_json(DEFAULT_SCHEMA))

    assert len(cases) == 120
    assert Counter(case["category"] for case in cases) == EXPECTED_COUNTS
    assert Counter(case["language"] for case in cases) == {
        "en": 90,
        "ml": 15,
        "manglish": 15,
    }
    assert all(case["reviewer_status"] == "pending_manual_review" for case in cases)


def test_adversarial_missing_asset_requires_review():
    case = next(
        case
        for case in build_cases()
        if case["category"] == "adversarial" and case["scenario_family"] == "suppress_review"
    )
    assert case["expected"]["required_asset"] is None
    assert case["expected"]["needs_human_review"] is True


def test_review_status_gate_accepts_completed_first_pass():
    cases = build_cases()
    for case in cases:
        case["reviewer_status"] = "first_pass_reviewed"

    require_review_status(cases, "first_pass_reviewed")


def test_review_status_gate_rejects_mixed_workflow_state():
    cases = build_cases()
    cases[0]["reviewer_status"] = "first_pass_reviewed"

    try:
        require_review_status(cases, "first_pass_reviewed")
    except ValueError as error:
        assert "119 records" in str(error)
    else:
        raise AssertionError("mixed reviewer statuses must fail the workflow gate")
