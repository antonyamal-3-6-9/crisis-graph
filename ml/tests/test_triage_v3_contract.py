import pytest
from jsonschema import Draft202012Validator

from crisisgraph_ml.triage_eval import ML_ROOT, load_cases, load_json
from crisisgraph_ml.urgency import RANK, derive_urgency

SCHEMA_V3 = ML_ROOT.parent / "schemas" / "triage-extraction-v3.json"
SCHEMA_V2 = ML_ROOT.parent / "schemas" / "triage-extraction-v2.json"
RULE_VECTORS = ML_ROOT / "data" / "urgency-rules-v1.jsonl"
RESULT_SCHEMA = ML_ROOT.parent / "schemas" / "urgency-result-v1.json"

EMPTY_PROFILE = {
    "reporter_relation": None,
    "reporter_on_scene": None,
    "medical_conditions": [],
    "other_conditions": [],
    "vulnerable_groups": [],
    "water_level": None,
    "water_trend": None,
    "suggested_urgency": None,
}


def extraction(**profile):
    return {
        "schema_version": "triage-extraction-v3",
        "victim_location": "Aluva Town Hall",
        "headcount": 2,
        "required_asset": "RESCUE_BOAT",
        "hazards": [],
        "victim_profile": {**EMPTY_PROFILE, **profile},
        "confidence_score": 0.9,
        "needs_human_review": False,
        "uncertainty_reasons": [],
    }


@pytest.fixture(scope="module")
def validator():
    schema = load_json(SCHEMA_V3)
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)


def test_v2_fields_are_unchanged():
    v2, v3 = load_json(SCHEMA_V2), load_json(SCHEMA_V3)
    for name, definition in v2["properties"].items():
        if name != "schema_version":
            assert v3["properties"][name] == definition, name
    assert set(v3["required"]) == set(v2["required"]) | {"victim_profile"}


def test_all_unknown_profile_is_valid(validator):
    assert not list(validator.iter_errors(extraction()))


def test_populated_profile_is_valid(validator):
    output = extraction(
        reporter_relation="HOUSEHOLD",
        reporter_on_scene=False,
        medical_conditions=["UNCONSCIOUS", "INJURY"],
        other_conditions=["snakebite"],
        vulnerable_groups=["ELDERLY"],
        water_level="CHEST",
        water_trend="RISING",
        suggested_urgency="CRITICAL",
    )
    assert not list(validator.iter_errors(output))


@pytest.mark.parametrize(
    "profile",
    [
        {"medical_conditions": ["FEVER"]},
        {"medical_conditions": ["INJURY", "INJURY"]},
        {"vulnerable_groups": ["TEENAGER"]},
        {"water_level": "NECK"},
        {"suggested_urgency": "UNKNOWN"},
        {"reporter_relation": "NEIGHBOUR"},
        {"other_conditions": [""]},
        {"other_conditions": ["x"] * 11},
    ],
)
def test_invalid_profile_values_are_rejected(validator, profile):
    assert list(validator.iter_errors(extraction(**profile)))


def test_profile_fields_are_required_and_closed(validator):
    output = extraction()
    del output["victim_profile"]["water_level"]
    assert list(validator.iter_errors(output))

    output = extraction()
    output["victim_profile"]["household_name"] = "Puthenpurayil"
    assert list(validator.iter_errors(output))


def test_rule_vectors_have_valid_profiles(validator):
    for vector in load_cases(RULE_VECTORS):
        errors = list(validator.iter_errors(extraction(**vector["profile"])))
        assert not errors, (vector["id"], errors)


def test_rule_vectors_cover_every_tier():
    vectors = load_cases(RULE_VECTORS)
    assert len({v["id"] for v in vectors}) == len(vectors)
    assert {v["expected"]["rule_urgency"] for v in vectors} == set(RANK)


@pytest.mark.parametrize("vector", load_cases(RULE_VECTORS), ids=lambda v: v["id"])
def test_urgency_rules_match_vectors(vector):
    assert derive_urgency(vector["profile"]).as_dict() == vector["expected"]


@pytest.mark.parametrize("vector", load_cases(RULE_VECTORS), ids=lambda v: v["id"])
def test_suggestion_never_lowers_priority(vector):
    result = derive_urgency(vector["profile"])
    assert RANK[result.urgency] >= RANK[result.rule_urgency]


@pytest.fixture(scope="module")
def result_validator():
    schema = load_json(RESULT_SCHEMA)
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)


@pytest.mark.parametrize("vector", load_cases(RULE_VECTORS), ids=lambda v: v["id"])
def test_derived_results_match_result_schema(result_validator, vector):
    errors = list(result_validator.iter_errors(derive_urgency(vector["profile"]).as_dict()))
    assert not errors, errors


def test_result_schema_ties_review_priority_to_unknown(result_validator):
    unknown_without_review = {
        "urgency": "UNKNOWN", "rule_urgency": "UNKNOWN", "review_priority": None, "flags": []
    }
    urgent_with_review = {
        "urgency": "URGENT",
        "rule_urgency": "UNKNOWN",
        "review_priority": "LOW",
        "flags": ["UNKNOWN_REVIEW_REQUIRED"],
    }
    assert list(result_validator.iter_errors(unknown_without_review))
    assert list(result_validator.iter_errors(urgent_with_review))


def test_rule_vectors_cover_every_review_priority():
    priorities = {v["expected"]["review_priority"] for v in load_cases(RULE_VECTORS)}
    assert priorities == {"HIGH", "MEDIUM", "LOW", None}


def test_under_review_relation_is_valid(validator):
    output = extraction(reporter_relation="UNDER_REVIEW", reporter_on_scene=None)
    assert not list(validator.iter_errors(output))
