from jsonschema import Draft202012Validator

from crisisgraph_ml.triage_eval import (
    DEFAULT_DATASET,
    DEFAULT_PROMPT,
    DEFAULT_SCHEMA,
    load_cases,
    load_json,
    request_payload,
    score_case,
    summarize,
    unsupported_fact_count,
)

SCHEMA = {
    "type": "object",
    "required": [
        "schema_version",
        "victim_location",
        "headcount",
        "required_asset",
        "hazards",
        "confidence_score",
        "needs_human_review",
        "uncertainty_reasons",
    ],
}


def complete_output(**overrides):
    output = {
        "schema_version": "triage-extraction-v2",
        "victim_location": None,
        "headcount": None,
        "required_asset": None,
        "hazards": [],
        "confidence_score": 0.2,
        "needs_human_review": True,
        "uncertainty_reasons": ["Missing information"],
    }
    output.update(overrides)
    return output


def test_llama_cpp_payload_uses_response_format():
    payload = request_payload("llama-cpp", "model", "prompt", SCHEMA, "help")
    assert payload["response_format"]["type"] == "json_schema"
    assert payload["response_format"]["json_schema"]["schema"] == SCHEMA
    assert payload["response_format"]["json_schema"]["strict"] is True
    assert "structured_outputs" not in payload


def test_vllm_payload_uses_structured_outputs():
    payload = request_payload("vllm", "model", "prompt", SCHEMA, "help")
    assert payload["structured_outputs"]["json"] == SCHEMA
    assert "response_format" not in payload


def test_invented_missing_value_is_counted():
    expected = complete_output()
    actual = complete_output(headcount=1, required_asset="EVAC_TRUCK")
    assert unsupported_fact_count(expected, actual) == 2


def test_correct_case_scores_exact_match():
    actual = complete_output()
    case = {
        "id": "case-1",
        "category": "missing_information",
        "expected": {
            field: actual[field]
            for field in (
                "victim_location",
                "headcount",
                "required_asset",
                "hazards",
                "needs_human_review",
            )
        },
    }
    result = score_case(case, actual, Draft202012Validator(SCHEMA))
    assert result.schema_valid
    assert result.exact_match
    assert result.unsupported_fact_count == 0


def test_repository_evaluation_assets_are_loadable():
    cases = load_cases(DEFAULT_DATASET)
    schema = load_json(DEFAULT_SCHEMA)
    Draft202012Validator.check_schema(schema)
    assert len(cases) == 12
    assert DEFAULT_PROMPT.read_text(encoding="utf-8").strip()
    assert schema["properties"]["headcount"]["anyOf"][1] == {"type": "null"}


def test_summary_contains_field_and_unsupported_fact_metrics():
    actual = complete_output(headcount=1)
    case = {
        "id": "case-1",
        "category": "missing_information",
        "expected": {
            "victim_location": None,
            "headcount": None,
            "required_asset": None,
            "hazards": [],
            "needs_human_review": True,
        },
    }
    result = score_case(case, actual, Draft202012Validator(SCHEMA))
    report = summarize([result], "model", "llama-cpp")
    assert report["field_accuracy"]["headcount"] == 0.0
    assert report["unsupported_fact_case_rate"] == 1.0
