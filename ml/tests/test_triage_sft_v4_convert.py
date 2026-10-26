import json

from crisisgraph_ml.triage_eval import load_cases
from crisisgraph_ml.triage_sft_v4_convert import (
    DEFAULT_OUTPUT,
    PROFILES,
    build_records,
    validate_records,
    verified_exports,
)


def test_conversion_is_valid_and_reproducible():
    records = build_records()
    validate_records(records)
    assert len(records) == 372
    assert load_cases(DEFAULT_OUTPUT) == records


def test_messages_and_v2_labels_are_unchanged():
    exports = verified_exports()
    sources = exports["train"] + exports["validation"]
    for record, source in zip(build_records(), sources, strict=True):
        _, user, assistant = source["messages"]
        assert record["input"] == user["content"]
        v2 = json.loads(assistant["content"])
        target = {k: v for k, v in record["target"].items()
                  if k not in ("schema_version", "victim_profile")}
        assert target == {k: v for k, v in v2.items() if k != "schema_version"}


def test_only_listed_records_have_profile_facts():
    for record in build_records():
        key = record["id"].removeprefix("sft-v4-conv-")
        profile = {k: v for k, v in record["target"]["victim_profile"].items()
                   if k != "suggested_urgency"}
        has_facts = any(v not in (None, []) for v in profile.values())
        assert has_facts == (key in PROFILES), key
