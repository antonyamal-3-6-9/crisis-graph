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


def test_every_new_record_uses_a_zero_context_rewrite():
    # The module's own texts were drafted after exposure to the evaluation sets.
    from crisisgraph_ml.triage_sft_v4_new import build_records

    assert all(record.get("text_source") == "zero-context rewrite" for record in build_records())


def test_leak_audit_drops_are_applied():
    import json

    from crisisgraph_ml.triage_sft_v4_new import LEAK_AUDIT, build_records

    dropped = json.loads(LEAK_AUDIT.read_text(encoding="utf-8"))["dropped"]
    assert len(dropped) == 15
    assert not {record["id"] for record in build_records()} & set(dropped)
