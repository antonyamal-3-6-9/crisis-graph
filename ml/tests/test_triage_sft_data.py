from pathlib import Path

import pytest

from crisisgraph_ml.triage_eval import DEFAULT_SCHEMA, load_json
from crisisgraph_ml.triage_sft_data import (
    DEFAULT_FROZEN_EVAL,
    LOCATIONS,
    MALAYALAM_LOCATIVES,
    SPLIT_LANGUAGE_COUNTS,
    build_split,
    export_messages,
    validate_splits,
)


def test_sft_splits_are_valid_and_do_not_duplicate_frozen_eval():
    datasets = {split: build_split(split) for split in SPLIT_LANGUAGE_COUNTS}

    validate_splits(datasets, load_json(DEFAULT_SCHEMA), load_jsonl(DEFAULT_FROZEN_EVAL))

    assert len(datasets["train"]) == 240
    assert len(datasets["validation"]) == 48
    assert len(datasets["dev"]) == 48
    assert all(
        case["reviewer_status"] == "pending_manual_review"
        for cases in datasets.values()
        for case in cases
    )


def load_jsonl(path: Path):
    from crisisgraph_ml.triage_eval import load_cases

    return load_cases(path)


def write_candidate(tmp_path, status):
    candidate = tmp_path / "candidate.jsonl"
    case = build_split("dev")[0]
    case["reviewer_status"] = status
    candidate.write_text(__import__("json").dumps(case) + "\n", encoding="utf-8")
    return candidate, case


def test_strict_export_refuses_first_pass_candidates(tmp_path):
    candidate, _ = write_candidate(tmp_path, "first_pass_reviewed")

    with pytest.raises(ValueError, match="review policy 'independent'"):
        export_messages(candidate, Path("prompts/triage-extraction-v2.1.1.txt"), tmp_path / "sft.jsonl")


def test_pilot_export_accepts_first_pass_candidates(tmp_path):
    candidate, case = write_candidate(tmp_path, "first_pass_reviewed")
    output = tmp_path / "sft.jsonl"

    count = export_messages(
        candidate,
        Path("prompts/triage-extraction-v2.1.1.txt"),
        output,
        review_policy="pilot-first-pass",
    )

    assert count == 1
    row = load_jsonl(output)[0]
    assert [message["role"] for message in row["messages"]] == [
        "system",
        "user",
        "assistant",
    ]
    assert row["messages"][1]["content"] == case["input"]
    assert __import__("json").loads(row["messages"][2]["content"]) == case["target"]


@pytest.mark.parametrize("status", ["pending_manual_review", "needs_revision"])
def test_pilot_export_rejects_unaccepted_candidates(tmp_path, status):
    candidate, _ = write_candidate(tmp_path, status)

    with pytest.raises(ValueError, match="review policy 'pilot-first-pass'"):
        export_messages(
            candidate,
            Path("prompts/triage-extraction-v2.1.1.txt"),
            tmp_path / "sft.jsonl",
            review_policy="pilot-first-pass",
        )


def test_reviewed_template_issues_do_not_regress():
    for split in SPLIT_LANGUAGE_COUNTS:
        cases = build_split(split)

        contradictory = [
            case for case in cases if case["category"].startswith("contradictory_")
        ]
        assert all("then corrected" not in case["input"] for case in contradictory)
        assert all("aadyam" not in case["input"] for case in contradictory)
        assert all("ആദ്യം" not in case["input"] for case in contradictory)

        malayalam_floods = [
            case
            for case in cases
            if case["language"] == "ml" and "FLOODED" in str(case["target"]["hazards"])
        ]
        assert all("വെള്ളത്തിലായിരിക്കും" not in case["input"] for case in malayalam_floods)

        manglish_floods = [
            case
            for case in cases
            if case["language"] == "manglish"
            and "FLOODED" in str(case["target"]["hazards"])
        ]
        assert all("hours aayi flooded aanu" in case["input"] for case in manglish_floods)


def test_malayalam_locations_use_explicit_natural_inflections():
    expected_locations = {
        location
        for split_locations in LOCATIONS.values()
        for location in split_locations["ml"]
    }
    assert set(MALAYALAM_LOCATIVES) == expected_locations
    assert all("-" not in form for form in MALAYALAM_LOCATIVES.values())

    malayalam_cases = [
        case
        for split in SPLIT_LANGUAGE_COUNTS
        for case in build_split(split)
        if case["language"] == "ml"
    ]
    assert all("-ൽ" not in case["input"] for case in malayalam_cases)
    assert all("-യിൽ" not in case["input"] for case in malayalam_cases)
    assert all("-ലെ" not in case["input"] for case in malayalam_cases)
    assert all("-ലേക്ക്" not in case["input"] for case in malayalam_cases)
