from crisisgraph_ml.triage_eval import load_cases
from crisisgraph_ml.triage_eval_v3_dev import validate_cases
from crisisgraph_ml.triage_eval_v3_v2_carryover import DEFAULT_OUTPUT, build_cases, load_v2


def test_carryover_is_valid_and_complete():
    cases = build_cases()
    validate_cases(cases)
    assert len(cases) == 120


def test_v2_labels_are_unchanged():
    for v2, v3 in zip(load_v2(), build_cases(), strict=True):
        assert v3["source_id"] == v2["id"]
        assert v3["input"] == v2["input"]
        expected = {k: v for k, v in v3["expected"].items() if k != "victim_profile"}
        assert expected == v2["expected"]


def test_file_matches_generator():
    assert load_cases(DEFAULT_OUTPUT) == build_cases()
