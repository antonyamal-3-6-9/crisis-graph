from collections import Counter

from crisisgraph_ml.triage_eval import load_cases
from crisisgraph_ml.triage_eval_v3_dev import DEFAULT_OUTPUT, build_cases, validate_cases


def test_dev_candidates_are_valid_v3_labels():
    validate_cases(build_cases())


def test_dev_file_matches_generator():
    assert load_cases(DEFAULT_OUTPUT) == build_cases()


def test_dev_set_covers_every_critical_condition_and_language():
    cases = build_cases()
    medical = Counter(
        condition for item in cases for condition in item["expected"]["victim_profile"]["medical_conditions"]
    )
    for condition in ("UNCONSCIOUS", "BREATHING_DIFFICULTY", "CARDIAC", "BLEEDING"):
        assert medical[condition] >= 2, condition
    assert {item["language"] for item in cases} == {"en", "ml", "manglish"}
    assert {item["expected_urgency"] for item in cases} == {"CRITICAL", "URGENT", "STABLE", "UNKNOWN"}
