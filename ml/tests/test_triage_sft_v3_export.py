import json

from crisisgraph_ml.triage_sft_v3_export import export, load_and_validate_sources


def test_v3_sources_have_expected_combined_counts():
    datasets = load_and_validate_sources()
    assert len(datasets["train"]) == 300
    assert len(datasets["validation"]) == 72


def test_v3_export_has_chat_messages_and_excludes_candidate_metadata(tmp_path):
    outputs = export(tmp_path)
    assert sum(1 for _ in outputs["train"].open()) == 300
    assert sum(1 for _ in outputs["validation"].open()) == 72
    first = json.loads(outputs["train"].read_text().splitlines()[0])
    assert list(first) == ["messages"]
    assert [message["role"] for message in first["messages"]] == [
        "system",
        "user",
        "assistant",
    ]
    json.loads(first["messages"][2]["content"])
