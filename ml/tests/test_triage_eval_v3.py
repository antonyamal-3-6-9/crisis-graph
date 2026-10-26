import asyncio
import json

import httpx
import pytest

from crisisgraph_ml.triage_eval import load_cases
from crisisgraph_ml.triage_eval_v3 import (
    EMPTY_PROFILE,
    check_gates,
    headline,
    parse_args,
    run,
    wilson_interval,
)
from crisisgraph_ml.triage_eval_v3_dev import DEFAULT_OUTPUT as DEV_SET
from crisisgraph_ml.triage_eval_v3_dev import as_model_output


def model_server(transform=lambda case, output: output, adapters=()):
    """Fake OpenAI-compatible server answering with the gold labels, optionally altered."""
    cases = {case["input"]: case for case in load_cases(DEV_SET)}

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/lora-adapters":
            return httpx.Response(
                200, json=[{"id": i, "path": f"artifacts/{a}", "scale": 1.0}
                           for i, a in enumerate(adapters)]
            )
        message = json.loads(request.content)["messages"][-1]["content"]
        case = cases[message]
        output = transform(case, as_model_output(case["expected"]))
        return httpx.Response(
            200, json={"choices": [{"message": {"content": json.dumps(output)}}]}
        )

    return httpx.MockTransport(handler)


@pytest.fixture
def args(tmp_path):
    prompt = tmp_path / "prompt.txt"
    prompt.write_text("test prompt")
    # The gap tests below are about urgency-rules-v1; v2 is checked separately.
    return parse_args(
        ["--dataset", str(DEV_SET), "--prompt", str(prompt), "--output", str(tmp_path / "run.json"),
         "--rules", "urgency-rules-v1"]
    )


def test_gold_answers_score_perfectly(args):
    report = asyncio.run(run(args, model_server()))
    profile = report["profile"]
    assert profile["usable_outputs"] == 42
    assert report["v2_fields"]["exact_match_accuracy"] == 1.0
    for condition in ("UNCONSCIOUS", "BREATHING_DIFFICULTY", "CARDIAC", "BLEEDING"):
        assert profile["medical_conditions"]["per_label"][condition]["recall"]["value"] == 1.0
    assert profile["remote_caller_miss"]["count"] == 0
    assert profile["invented_fact_count"] == 0
    # Without suggested_urgency, only the rule table's known gaps under-triage.
    assert set(profile["urgency"]["critical_under_triage_cases"]) == {
        "v3-dev-011", "v3-dev-014"
    }
    assert profile["urgency"]["oracle_under_triage_cases"] == ["v3-dev-011", "v3-dev-014"]


def test_suggestion_closes_rule_gaps(args):
    def suggest(case, output):
        if case["expected_urgency"] == "CRITICAL":
            output["victim_profile"]["suggested_urgency"] = "CRITICAL"
        return output

    profile = asyncio.run(run(args, model_server(suggest)))["profile"]
    assert profile["urgency"]["critical_under_triage_cases"] == []
    assert profile["urgency"]["rule_only_critical_under_triage"]["count"] == 2
    assert profile["urgency"]["suggestion_raised"] == 2


def test_missed_conditions_and_remote_callers_are_caught(args):
    def degrade(case, output):
        profile = output["victim_profile"]
        profile["medical_conditions"] = [
            m for m in profile["medical_conditions"] if m != "UNCONSCIOUS"
        ]
        if profile["reporter_on_scene"] is False:
            profile["reporter_on_scene"] = True
        if case["id"] == "v3-dev-028":
            profile["vulnerable_groups"] = ["ELDERLY"]
        return output

    profile = asyncio.run(run(args, model_server(degrade)))["profile"]
    assert profile["medical_conditions"]["per_label"]["UNCONSCIOUS"]["recall"]["value"] == 0.0
    assert {"v3-dev-001", "v3-dev-005", "v3-dev-034"} <= set(
        profile["urgency"]["critical_under_triage_cases"]
    )
    assert profile["remote_caller_miss"]["value"] == 1.0
    assert profile["invented_fact_cases"]["count"] == 1


def test_unusable_output_counts_as_missed_facts(args):
    def broken(case, output):
        return {"not": "the schema"}

    profile = asyncio.run(run(args, model_server(broken)))["profile"]
    assert profile["usable_outputs"] == 0
    assert profile["urgency"]["critical_under_triage"]["value"] == 1.0


def test_resume_reuses_checkpoint_and_rejects_changed_identity(args, tmp_path):
    calls = {"n": 0}
    inner = model_server()

    def counting(request):
        if request.url.path == "/lora-adapters":
            return inner.handle_request(request)
        calls["n"] += 1
        if calls["n"] > 5:
            raise httpx.ConnectError("server stopped")
        return inner.handle_request(request)

    asyncio.run(run(args, httpx.MockTransport(counting)))
    checkpoint = args.output.with_suffix(".checkpoint.jsonl")
    assert len(checkpoint.read_text().splitlines()) == 42  # errors are recorded too

    # Keep the first five answers, drop the failures, resume with a working server.
    lines = checkpoint.read_text().splitlines()[:5]
    checkpoint.write_text("\n".join(lines) + "\n")
    calls["n"] = 0
    report = asyncio.run(run(args, model_server()))
    assert report["profile"]["usable_outputs"] == 42

    args.prompt.write_text("a different prompt")
    with pytest.raises(ValueError, match="identity changed"):
        asyncio.run(run(args, model_server()))


def test_gates_pass_on_gold_answers(args, tmp_path):
    report = asyncio.run(run(args, model_server()))
    gates = check_gates(report, report, baseline_exact_match=42, baseline_review_recall=1.0,
                        baseline_p95_ms=1000.0)
    # Gold answers without suggestions leave two rule-gap under-triage cases (> 1).
    assert gates["checks"]["critical_under_triage"] is False
    assert all(v for k, v in gates["checks"].items() if k != "critical_under_triage")
    assert headline(report)["usable_outputs"] == 42


def test_wilson_interval_bounds():
    assert wilson_interval(0, 0) == (0.0, 0.0)
    low, high = wilson_interval(9, 10)
    assert 0.55 < low < 0.6 and 0.98 < high <= 1.0


def test_empty_profile_covers_schema_fields():
    from crisisgraph_ml.triage_eval import load_json
    from crisisgraph_ml.triage_eval_v3 import DEFAULT_SCHEMA

    schema = load_json(DEFAULT_SCHEMA)
    assert set(EMPTY_PROFILE) == set(schema["properties"]["victim_profile"]["required"])


def test_declared_adapter_must_match_the_server(args):
    with pytest.raises(ValueError, match="server applies"):
        asyncio.run(run(args, model_server(adapters=["adapter-f16.gguf"])))

    args.adapter = "artifacts/qwen3-4b-triage-sft-v3/adapter-f16.gguf"
    with pytest.raises(ValueError, match="server applies none"):
        asyncio.run(run(args, model_server()))

    report = asyncio.run(run(args, model_server(adapters=["adapter-f16.gguf"])))
    assert report["identity"]["server_adapters"] == ["adapter-f16.gguf"]


def test_waived_check_is_reported_but_not_counted(args):
    report = asyncio.run(run(args, model_server()))
    gates = check_gates(report, report, baseline_exact_match=42, baseline_review_recall=1.0,
                        baseline_p95_ms=0.001, waived=frozenset({"latency_p95",
                                                                 "critical_under_triage"}))
    assert gates["checks"]["latency_p95"] is False
    assert gates["passed"] is True
    with pytest.raises(ValueError, match="unknown waived"):
        check_gates(report, report, 42, 1.0, 1.0, waived=frozenset({"latency"}))


def test_rescore_under_v2_closes_the_other_condition_gap(args):
    from crisisgraph_ml.triage_eval_v3 import rescore

    report = asyncio.run(run(args, model_server()))
    assert report["urgency_rules"] == "urgency-rules-v1"
    v2 = rescore(report, DEV_SET, "urgency-rules-v2")
    assert v2["urgency_rules"] == "urgency-rules-v2"
    assert v2["profile"]["urgency"]["critical_under_triage_cases"] == []
    assert v2["profile"]["urgency"]["oracle_under_triage_cases"] == []
    # Same model outputs, so the V2 field scores are identical.
    assert v2["v2_fields"] == report["v2_fields"]


def test_rescore_rejects_a_different_dataset(args, tmp_path):
    from crisisgraph_ml.triage_eval_v3 import rescore

    report = asyncio.run(run(args, model_server()))
    other = tmp_path / "other.jsonl"
    other.write_text(DEV_SET.read_text() + "\n")
    with pytest.raises(ValueError, match="dataset differs"):
        rescore(report, other, "urgency-rules-v2")


def test_transient_api_errors_are_retried_not_recorded(args, monkeypatch):
    import crisisgraph_ml.triage_eval_v3 as module

    async def no_sleep(_):
        return None

    monkeypatch.setattr(module.asyncio, "sleep", no_sleep)
    inner = model_server()
    calls = {"n": 0}

    def flaky(request):
        if request.url.path != "/lora-adapters":
            calls["n"] += 1
            if calls["n"] % 3 == 1:
                return httpx.Response(503, json={"error": "high demand"})
        return inner.handle_request(request)

    report = asyncio.run(run(args, httpx.MockTransport(flaky)))
    assert report["profile"]["usable_outputs"] == 42
    assert report["v2_fields"]["error_count"] == 0


def test_persistent_api_errors_stop_the_run_for_resume(args, monkeypatch):
    import crisisgraph_ml.triage_eval_v3 as module

    async def no_sleep(_):
        return None

    monkeypatch.setattr(module.asyncio, "sleep", no_sleep)
    inner = model_server()

    def down(request):
        if request.url.path == "/lora-adapters":
            return inner.handle_request(request)
        return httpx.Response(429, json={"error": "quota"})

    with pytest.raises(SystemExit, match="resume"):
        asyncio.run(run(args, httpx.MockTransport(down)))
    assert not args.output.with_suffix(".checkpoint.jsonl").exists() or not (
        args.output.with_suffix(".checkpoint.jsonl").read_text().strip()
    )


def test_api_key_is_sent_but_not_recorded(args, monkeypatch):
    monkeypatch.setenv("TEST_API_KEY", "secret-value")
    args.api_key_env = "TEST_API_KEY"
    seen = []
    inner = model_server()

    def capture(request):
        seen.append(request.headers.get("authorization"))
        return inner.handle_request(request)

    report = asyncio.run(run(args, httpx.MockTransport(capture)))
    assert "Bearer secret-value" in seen
    assert "secret-value" not in json.dumps(report)
