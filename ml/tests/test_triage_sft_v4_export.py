import json

import pytest

from crisisgraph_ml.triage_sft_v4_export import (
    export,
    leak_problems,
    load_records,
    target_problems,
)


def test_sources_meet_spec_targets_and_are_separated():
    splits = load_records()
    assert target_problems(splits["train"]) == []
    assert leak_problems(splits) == []


def test_export_writes_prompted_rows_and_refuses_to_overwrite(tmp_path):
    manifest = export(tmp_path, tmp_path / "manifest.json")
    train = [json.loads(line) for line in (tmp_path / "triage-sft-v4-train.jsonl").open()]
    assert len(train) == manifest["exports"]["train"]["records"]
    system, _, assistant = train[0]["messages"]
    assert system["role"] == "system" and "victim_profile" in system["content"]
    assert list(json.loads(assistant["content"]))[-1] == "victim_profile"
    with pytest.raises(FileExistsError):
        export(tmp_path, tmp_path / "manifest.json")


def test_targets_fail_closed_when_too_few_records():
    assert target_problems(load_records()["train"][:100])
