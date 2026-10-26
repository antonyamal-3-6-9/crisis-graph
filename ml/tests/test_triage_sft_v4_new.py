from crisisgraph_ml.triage_eval import load_cases
from crisisgraph_ml.triage_sft_v4_convert import validate_records
from crisisgraph_ml.triage_sft_v4_new import DEFAULT_OUTPUT, build_records, separation_problems


def test_new_records_are_valid_targets():
    records = build_records()
    validate_records(records)
    assert len({r["id"] for r in records}) == len(records)


def test_new_records_are_separated_from_every_evaluation_set():
    assert separation_problems(build_records()) == []


def test_file_matches_generator():
    assert load_cases(DEFAULT_OUTPUT) == build_records()


def test_review_decision_follows_the_v2_rule():
    for record in build_records():
        target = record["target"]
        incomplete = (
            target["victim_location"] is None
            or target["headcount"] is None
            or target["required_asset"] is None
            or any(v is None for hazard in target["hazards"] for v in hazard.values())
        )
        assert target["needs_human_review"] is incomplete, record["id"]
        assert bool(target["uncertainty_reasons"]) is incomplete, record["id"]
