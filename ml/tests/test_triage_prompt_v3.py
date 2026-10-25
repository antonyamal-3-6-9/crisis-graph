import json
import re

from jsonschema import Draft202012Validator

from crisisgraph_ml.triage_eval import ML_ROOT, load_cases, load_json

PROMPT = ML_ROOT / "prompts" / "triage-extraction-v3.0.txt"
SCHEMA = ML_ROOT.parent / "schemas" / "triage-extraction-v3.json"
EVAL_SETS = [
    "triage-eval-v3-dev-candidates.jsonl",
    "triage-eval-v3.0.1.jsonl",
    "triage-eval-v3-v2-carryover.jsonl",
]


def examples():
    text = PROMPT.read_text(encoding="utf-8")
    reports = re.findall(r'^Report: "(.*)"$', text, flags=re.MULTILINE)
    outputs = [json.loads(line) for line in text.splitlines() if line.startswith("{")]
    return reports, outputs


def test_prompt_examples_are_valid_v3_outputs():
    reports, outputs = examples()
    assert len(reports) == len(outputs) == 3
    validator = Draft202012Validator(load_json(SCHEMA))
    for output in outputs:
        assert not list(validator.iter_errors(output))


def test_prompt_examples_do_not_leak_evaluation_cases():
    reports, outputs = examples()
    cases = [case for name in EVAL_SETS for case in load_cases(ML_ROOT / "data" / name)]
    inputs = {case["input"].casefold() for case in cases}
    test_locations = {
        (case["expected"]["victim_location"] or "").casefold()
        for case in load_cases(ML_ROOT / "data" / "triage-eval-v3.0.1.jsonl")
    }
    for report, output in zip(reports, outputs):
        assert report.casefold() not in inputs
        assert output["victim_location"].casefold() not in test_locations
