import asyncio
import json
from pathlib import Path

import httpx

from crisisgraph_ml import serving_bench as bench

ML_ROOT = Path(__file__).resolve().parents[1]
DATASET = ML_ROOT / "data" / "triage-eval-v3.1.0.jsonl"
PROMPT = ML_ROOT / "prompts" / "triage-extraction-v3.2.txt"


def sse(chunks: list[dict]) -> bytes:
    lines = [f"data: {json.dumps(c)}\n\n" for c in chunks] + ["data: [DONE]\n\n"]
    return "".join(lines).encode()


def streaming_server(answer_for, seen: list[dict] | None = None, fail_ids: set[str] = frozenset()):
    """A fake OpenAI-compatible server that streams an answer in three pieces."""

    def handler(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        if seen is not None:
            seen.append(body)
        message = body["messages"][1]["content"]
        if message in fail_ids:
            return httpx.Response(500, text="boom")
        answer = answer_for(message)
        pieces = [answer[:3], answer[3:6], answer[6:]]
        chunks = [{"choices": [{"delta": {"content": p}}]} for p in pieces]
        chunks.append({"choices": [], "usage": {"completion_tokens": 3}})
        return httpx.Response(200, content=sse(chunks), headers={"content-type": "text/event-stream"})

    return httpx.MockTransport(handler)


def args_for(tmp_path: Path, *extra: str):
    return bench.parse_args([
        "--model", "sftv4", "--dataset", str(DATASET), "--prompt", str(PROMPT),
        "--limit", "5", "--output", str(tmp_path / "report.json"), *extra,
    ])


def test_percentile_is_nearest_rank():
    assert bench.percentile([], 50) is None
    assert bench.percentile([1.0, 2.0, 3.0, 4.0], 50) == 2.0
    assert bench.percentile([1.0, 2.0, 3.0, 4.0], 95) == 4.0
    assert bench.percentile([5.0], 99) == 5.0


def test_bench_sends_the_real_request_streamed_and_reports_each_level(tmp_path):
    seen: list[dict] = []
    transport = streaming_server(lambda m: '{"answer": "' + m[:10] + '"}', seen)
    args = args_for(tmp_path, "--concurrency", "1,2", "--requests-per-level", "6", "--no-warmup")
    report = asyncio.run(bench.run(args, transport))

    assert [level["concurrency"] for level in report["levels"]] == [1, 2]
    for level in report["levels"]:
        assert level["requests"] == 6 and level["failed"] == 0
        assert level["output_tokens"]["p50"] == 3.0
        assert level["ttft_s"]["n"] == 6 and level["tpot_s"]["n"] == 6
    # The real extraction request, with streaming and the vLLM schema parameter.
    first = seen[0]
    assert first["stream"] is True and first["stream_options"] == {"include_usage": True}
    assert first["messages"][0]["content"] == PROMPT.read_text(encoding="utf-8").strip()
    assert "structured_outputs" in first and first["temperature"] == 0.0
    # Serial answers are fingerprinted per case (5 cases, cycled).
    assert len(report["fingerprints"]) == 5
    assert json.loads((tmp_path / "report.json").read_text())["levels"][0]["concurrency"] == 1


def test_failed_requests_are_counted_not_raised(tmp_path):
    first_case = json.loads(DATASET.read_text(encoding="utf-8").splitlines()[0])
    transport = streaming_server(lambda m: '{"ok": true}', fail_ids={first_case["input"]})
    args = args_for(tmp_path, "--concurrency", "1", "--requests-per-level", "5", "--no-warmup")
    level = asyncio.run(bench.run(args, transport))["levels"][0]
    assert level["failed"] == 1 and level["requests"] == 5


def test_changed_answers_are_flagged_against_a_reference(tmp_path):
    reference = asyncio.run(bench.run(
        args_for(tmp_path, "--concurrency", "1", "--no-warmup"),
        streaming_server(lambda m: '{"same": 1}')))
    ref_path = tmp_path / "reference.json"
    ref_path.write_text(json.dumps(reference))

    unchanged = asyncio.run(bench.run(
        args_for(tmp_path, "--concurrency", "4", "--no-warmup", "--reference", str(ref_path)),
        streaming_server(lambda m: '{"same": 1}')))
    assert unchanged["levels"][0]["answers_changed_vs_reference"] == []

    changed = asyncio.run(bench.run(
        args_for(tmp_path, "--concurrency", "4", "--no-warmup", "--reference", str(ref_path)),
        streaming_server(lambda m: '{"same": 2}')))
    assert len(changed["levels"][0]["answers_changed_vs_reference"]) == 5
