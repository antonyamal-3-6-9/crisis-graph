"""Serving benchmark for triage extraction (plan ML-S1, step 3).

Replays real extraction requests (the system prompt, schema and messages of
a triage dataset) against an OpenAI-compatible server at fixed concurrency
levels, with streaming, and records per request:

- time to first token (TTFT) and time per output token (TPOT),
- end-to-end latency and output tokens,
- a fingerprint of the answer, so speed settings can be checked for
  changed answers against a serial run.

Per concurrency level it reports p50/p95/p99 latencies, requests/s and output
tokens/s; with ``--gpu-sample`` it also samples GPU memory with nvidia-smi.

This measures speed only. Accuracy is measured by ``triage_eval_v3`` on the
same server, serially. Example::

    crisisgraph-serving-bench --base-url http://127.0.0.1:8000/v1 --backend vllm \\
        --model sftv4 --dataset ml/data/triage-eval-v3.1.0.jsonl \\
        --prompt ml/prompts/triage-extraction-v3.2.txt --concurrency 1,4,16 \\
        --requests-per-level 48 --output runs/bench-B.json --gpu-sample
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import math
import platform
import shutil
import subprocess
import threading
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Self

import httpx

from crisisgraph_ml.triage_eval import load_cases, load_json, request_payload
from crisisgraph_ml.triage_eval_v3 import DEFAULT_SCHEMA


@dataclass
class RequestResult:
    level: int
    index: int
    case_id: str
    ok: bool
    error: str | None
    ttft_s: float | None
    e2e_s: float
    output_tokens: int | None
    tpot_s: float | None
    answer_sha256: str | None


def percentile(values: list[float], q: float) -> float | None:
    """Nearest-rank percentile (q in 0..100); None for no values."""
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, math.ceil(q / 100 * len(ordered)))
    return ordered[rank - 1]


def spread(values: list[float]) -> dict[str, float | None]:
    return {"n": len(values), "p50": percentile(values, 50), "p95": percentile(values, 95),
            "p99": percentile(values, 99), "max": max(values) if values else None}


async def one_request(client: httpx.AsyncClient, url: str, payload: dict[str, Any],
                      level: int, index: int, case_id: str) -> RequestResult:
    """Send one streaming request and time it."""
    body = {**payload, "stream": True, "stream_options": {"include_usage": True}}
    started = time.perf_counter()
    first: float | None = None
    parts: list[str] = []
    tokens: int | None = None
    try:
        async with client.stream("POST", url, json=body) as response:
            if response.status_code != 200:
                text = (await response.aread()).decode(errors="replace")[:300]
                raise RuntimeError(f"HTTP {response.status_code}: {text}")
            async for line in response.aiter_lines():
                if not line.startswith("data:"):
                    continue
                data = line[5:].strip()
                if data == "[DONE]":
                    break
                chunk = json.loads(data)
                usage = chunk.get("usage")
                if usage and usage.get("completion_tokens") is not None:
                    tokens = usage["completion_tokens"]
                for choice in chunk.get("choices") or []:
                    delta = (choice.get("delta") or {}).get("content")
                    if delta:
                        if first is None:
                            first = time.perf_counter()
                        parts.append(delta)
    except (httpx.HTTPError, RuntimeError, ValueError) as error:  # recorded: one failure must not stop a level
        return RequestResult(level, index, case_id, False, f"{type(error).__name__}: {error}"[:300],
                             None, time.perf_counter() - started, None, None, None)
    ended = time.perf_counter()
    ttft = None if first is None else first - started
    tpot = None
    if ttft is not None and tokens and tokens > 1:
        tpot = (ended - first) / (tokens - 1)
    answer = "".join(parts)
    return RequestResult(level, index, case_id, True, None, ttft, ended - started, tokens, tpot,
                         hashlib.sha256(answer.encode()).hexdigest())


async def run_level(client: httpx.AsyncClient, url: str, payloads: list[tuple[str, dict]],
                    level: int, count: int) -> tuple[list[RequestResult], float]:
    """Keep ``level`` requests in flight until ``count`` have been sent.

    Request i uses case ``i % len(payloads)``, so every level replays the
    dataset in the same order.
    """
    queue: asyncio.Queue[int] = asyncio.Queue()
    for i in range(count):
        queue.put_nowait(i)
    results: list[RequestResult] = []

    async def worker() -> None:
        while True:
            try:
                i = queue.get_nowait()
            except asyncio.QueueEmpty:
                return
            case_id, payload = payloads[i % len(payloads)]
            results.append(await one_request(client, url, payload, level, i, case_id))

    started = time.perf_counter()
    await asyncio.gather(*(worker() for _ in range(level)))
    return sorted(results, key=lambda r: r.index), time.perf_counter() - started


def summarize_level(results: list[RequestResult], wall_s: float) -> dict[str, Any]:
    ok = [r for r in results if r.ok]
    out_tokens = sum(r.output_tokens or 0 for r in ok)
    return {
        "requests": len(results),
        "failed": len(results) - len(ok),
        "wall_s": round(wall_s, 3),
        "requests_per_s": round(len(ok) / wall_s, 4) if wall_s > 0 else None,
        "output_tokens_per_s": round(out_tokens / wall_s, 2) if wall_s > 0 else None,
        "ttft_s": spread([r.ttft_s for r in ok if r.ttft_s is not None]),
        "tpot_s": spread([r.tpot_s for r in ok if r.tpot_s is not None]),
        "e2e_s": spread([r.e2e_s for r in ok]),
        "output_tokens": spread([float(r.output_tokens) for r in ok if r.output_tokens is not None]),
    }


def answer_changes(reference: dict[str, str], results: list[RequestResult]) -> list[str]:
    """Case ids whose answer differs from the reference fingerprints."""
    return sorted({r.case_id for r in results
                   if r.ok and r.case_id in reference and r.answer_sha256 != reference[r.case_id]})


class GpuSampler:
    """Samples used GPU memory (MiB) with nvidia-smi in a background thread."""

    def __init__(self, interval_s: float = 1.0) -> None:
        self.interval_s = interval_s
        self.samples: list[list[int]] = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def _run(self) -> None:
        while not self._stop.is_set():
            try:
                out = subprocess.run(
                    ["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"],
                    capture_output=True, text=True, timeout=10, check=True).stdout
                self.samples.append([int(v) for v in out.split()])
            except (OSError, subprocess.SubprocessError, ValueError):
                pass
            self._stop.wait(self.interval_s)

    def __enter__(self) -> Self:
        if shutil.which("nvidia-smi"):
            self._thread.start()
        return self

    def __exit__(self, *_: object) -> None:
        self._stop.set()
        if self._thread.is_alive():
            self._thread.join(timeout=15)

    def peak_mib(self) -> list[int] | None:
        if not self.samples:
            return None
        return [max(sample[g] for sample in self.samples) for g in range(len(self.samples[0]))]


def build_payloads(args: argparse.Namespace) -> list[tuple[str, dict[str, Any]]]:
    cases = load_cases(args.dataset)
    if args.limit:
        cases = cases[: args.limit]
    prompt = args.prompt.read_text(encoding="utf-8").strip()
    schema = load_json(args.schema)
    return [(case["id"], request_payload(args.backend, args.model, prompt, schema, case["input"],
                                         args.max_tokens)) for case in cases]


async def run(args: argparse.Namespace, transport: httpx.AsyncBaseTransport | None = None
              ) -> dict[str, Any]:
    payloads = build_payloads(args)
    url = f"{args.base_url.rstrip('/')}/chat/completions"
    headers = {}
    if args.api_key_env:
        import os
        key = os.environ.get(args.api_key_env)
        if not key:
            raise SystemExit(f"environment variable {args.api_key_env} is not set")
        headers["Authorization"] = f"Bearer {key}"
    reference = load_json(args.reference)["fingerprints"] if args.reference else None

    levels: list[dict[str, Any]] = []
    fingerprints: dict[str, str] = {}
    limits = httpx.Limits(max_connections=max(args.concurrency) + 4)
    async with httpx.AsyncClient(timeout=httpx.Timeout(args.timeout), transport=transport,
                                 headers=headers, limits=limits) as client:
        if args.warmup:
            await one_request(client, url, payloads[0][1], 0, -1, payloads[0][0])
        for level in args.concurrency:
            count = args.requests_per_level or max(2 * level, len(payloads))
            sampler = GpuSampler() if args.gpu_sample else None  # one thread per level
            if sampler:
                with sampler:
                    results, wall = await run_level(client, url, payloads, level, count)
            else:
                results, wall = await run_level(client, url, payloads, level, count)
            summary = {"concurrency": level, **summarize_level(results, wall)}
            if sampler:
                summary["gpu_memory_peak_mib"] = sampler.peak_mib()
            if level == 1:
                fingerprints.update({r.case_id: r.answer_sha256 for r in results
                                     if r.ok and r.answer_sha256})
            if reference is not None:
                summary["answers_changed_vs_reference"] = answer_changes(reference, results)
            levels.append(summary)
            print(json.dumps({k: summary[k] for k in ("concurrency", "requests", "failed",
                                                      "requests_per_s", "output_tokens_per_s")}
                             | {"e2e_p50": summary["e2e_s"]["p50"], "ttft_p50": summary["ttft_s"]["p50"]}),
                  flush=True)
            if args.raw:
                with args.raw.open("a", encoding="utf-8") as handle:
                    for r in results:
                        handle.write(json.dumps(asdict(r)) + "\n")

    report = {
        "label": args.label,
        "identity": {
            "base_url": args.base_url, "backend": args.backend, "model": args.model,
            "dataset": args.dataset.name, "dataset_cases": len(payloads),
            "prompt": args.prompt.name,
            "prompt_sha256": hashlib.sha256(args.prompt.read_bytes()).hexdigest(),
            "max_tokens": args.max_tokens, "server_notes": args.server_notes,
            "client_host": platform.node(), "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        },
        "levels": levels,
        "fingerprints": fingerprints,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Serving benchmark for triage extraction")
    parser.add_argument("--base-url", default="http://127.0.0.1:8000/v1")
    parser.add_argument("--backend", choices=("llama-cpp", "vllm", "openai"), default="vllm")
    parser.add_argument("--model", required=True, help="served model or LoRA name")
    parser.add_argument("--api-key-env", default=None)
    parser.add_argument("--dataset", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=0, help="use only the first N cases")
    parser.add_argument("--prompt", type=Path, required=True)
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    parser.add_argument("--max-tokens", type=int, default=768)
    parser.add_argument("--concurrency", type=lambda s: [int(v) for v in s.split(",")],
                        default=[1, 2, 4, 8, 16, 32])
    parser.add_argument("--requests-per-level", type=int, default=0,
                        help="default: max(2 x concurrency, dataset cases)")
    parser.add_argument("--timeout", type=float, default=600.0)
    parser.add_argument("--no-warmup", dest="warmup", action="store_false",
                        help="skip the untimed first request")
    parser.add_argument("--reference", type=Path, default=None,
                        help="earlier report: flag cases whose answer changed")
    parser.add_argument("--gpu-sample", action="store_true", help="sample GPU memory (nvidia-smi)")
    parser.add_argument("--label", default="")
    parser.add_argument("--server-notes", default="", help="server command or settings, recorded")
    parser.add_argument("--raw", type=Path, default=None, help="append per-request JSONL here")
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args(argv)


def main() -> None:
    asyncio.run(run(parse_args()))


if __name__ == "__main__":
    main()
