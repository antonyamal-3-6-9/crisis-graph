import json
from collections import Counter

from crisisgraph_ml.triage_eval import load_cases
from crisisgraph_ml.triage_eval_v3_candidates import (
    CONTRACT_FILES,
    DEFAULT_OUTPUT,
    FROZEN,
    MANIFEST,
    build_cases,
    sha256,
)
from crisisgraph_ml.triage_eval_v3_dev import build_cases as build_dev_cases
from crisisgraph_ml.triage_eval_v3_dev import validate_cases

CRITICAL_CONDITIONS = ("UNCONSCIOUS", "BREATHING_DIFFICULTY", "CARDIAC", "BLEEDING")


def profiles(cases):
    return [item["expected"]["victim_profile"] for item in cases]


def test_candidates_are_valid_v3_labels():
    cases = build_cases()
    validate_cases(cases)
    assert len(cases) == 120


def test_file_matches_generator():
    assert load_cases(DEFAULT_OUTPUT) == build_cases()


def test_each_critical_condition_has_at_least_ten_cases():
    counts = Counter(m for p in profiles(build_cases()) for m in p["medical_conditions"])
    for condition in CRITICAL_CONDITIONS:
        assert counts[condition] >= 10, (condition, counts[condition])


def test_coverage_of_callers_tiers_relations_and_languages():
    cases = build_cases()
    assert sum(p["reporter_on_scene"] is False for p in profiles(cases)) >= 10
    assert {p["reporter_relation"] for p in profiles(cases)} == {
        "SELF", "HOUSEHOLD", "THIRD_PARTY", "UNDER_REVIEW", None
    }
    assert {item["expected_urgency"] for item in cases} == {
        "CRITICAL", "URGENT", "STABLE", "UNKNOWN"
    }
    languages = Counter(item["language"] for item in cases)
    assert languages["ml"] >= 20 and languages["manglish"] >= 20


def test_no_overlap_with_dev_set():
    dev = build_dev_cases()
    dev_inputs = {item["input"].casefold() for item in dev}
    dev_locations = {
        (item["expected"]["victim_location"] or "").casefold() for item in dev
    } - {""}
    for item in build_cases():
        assert item["input"].casefold() not in dev_inputs, item["id"]
        location = (item["expected"]["victim_location"] or "").casefold()
        assert location not in dev_locations, (item["id"], location)


def test_frozen_set_matches_manifest_checksum():
    manifest = json.loads(MANIFEST.read_text())
    assert manifest["frozen_benchmark"] is True
    assert sha256(FROZEN) == manifest["sha256"]
    assert len(load_cases(FROZEN)) == manifest["record_count"] == 120


def test_generator_still_reproduces_frozen_set():
    # Any later label edit must become a new dataset version, not a silent change.
    assert load_cases(FROZEN) == build_cases()


def test_contract_files_unchanged_since_freeze():
    manifest = json.loads(MANIFEST.read_text())
    for name, path in CONTRACT_FILES.items():
        assert sha256(path) == manifest["contract_sha256"][name], name
