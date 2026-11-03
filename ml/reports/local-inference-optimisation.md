# Local inference optimisation: Qwen3-4B + SFT-V4 on the laptop CPU

Status: **complete** (2026-11-02 to 2026-11-03). Raw benchmark data and logs are kept locally and not published; the scripts in `ml/scripts/local-inference/` reproduce every table.

**Summary so far:**
- A warm extraction takes about 16–19 s. Almost all of it is generating about 110 JSON tokens at about 7–8 tokens/s.
- The system prompt is already cached.
- Device, thread and llama.cpp runtime options do not speed up the real request: at the real context length (about 2,400 cached tokens), **attention over the cached system prompt** costs roughly half the generation speed on the CPU, and every setting lands at about 7–8 tokens/s.
- The adapter costs about 3–10% and the JSON grammar nothing.
- Changing device (iGPU to CPU) changes some greedy answers, so it is not a speed-only setting.
- **In the real pipeline (Finding 6),** extraction takes about 20 s, not the 60–70 s quoted earlier. The only slow report was the first after a server restart (54 s, close to the 60 s timeout). A start-up warm-up in the triage role removed it: the slowest report went from 54 s to 21.5 s.
- **Speculative decoding (Finding 7) does not help on this laptop.** A Qwen3-0.6B draft is accepted 55–86% of the time, but a pass that checks several tokens costs so much here that every draft configuration is slower. The one faster variant (`ngram-mod`, up to +37%) changed one answer, so it is not adopted without a blind-set accuracy run.
- The remaining local lever is shorter attention (a shorter system prompt, an ML change with a blind-set accuracy run). Speculative decoding moves to the T4 study, where GPU batching is cheap.

## Why

CrisisGraph is designed so that a small model can run locally, for resilience when the field site's network is poor. Extraction is the slowest step between an SOS and a vehicle:

- the matcher commits a vehicle in under a second once a task is queued;
- a local extraction takes many seconds.

Making local extraction faster without changing its answers directly shortens time to assignment.

## What was measured

| Item | Value |
|---|---|
| Base model | Qwen3-4B-Instruct-2507, Q4_K_M GGUF, `sha256 8cdb57cbb880d313736a9bc4e3d3d2485f145b5e19cf33783746e753e82641fc`, the file used by the SFT-V3/V4 evaluations |
| Adapter | SFT-V4, `adapter-f16.gguf`, `sha256 f31100c925b3b2f4540f481be0f9338d5024ca931c0368265f6b16eb5f520fbc` (applied at run time with `--lora`) |
| Prompt and schema | `ml/prompts/triage-extraction-v3.2.txt` (`9368283c…`), `schemas/triage-extraction-v3.json` (`c9b3ed10…`), exactly what the Rust extractor sends |
| llama.cpp | distro package `llama-cpp 0.6.0-1.1`, build 11429, commit `d812350493`, built with the **Vulkan** backend |
| Machine | Intel Core i5-13420H, 4 performance cores (logical CPUs 0–7, with hyperthreads, 4.6 GHz) and 4 efficiency cores (CPUs 8–11, 3.4 GHz); AVX2, AVX-VNNI, FMA, F16C; no AVX-512 |
| Other hardware and software | 16 GB RAM, Intel iGPU (i915), Linux 7.2.11 (CachyOS) |
| Power | **mains power, performance power profile** (see "Measurement pitfall" below) |
| Background load | Neo4j (`java`, about 8% CPU) and a browser, both recorded; otherwise idle |

**Tools** (all in `ml/scripts/local-inference/`):

- `measure.py` sends real extraction requests to `llama-server`, optionally interleaved with the landmark-refinement call as the pipeline does. It prints llama.cpp's own per-request `timings`: cached tokens, new prompt tokens, prompt time, generated tokens and generation time.
- `matrix.sh` runs `llama-bench` over device, thread and pinning configurations: 5 repetitions each of generation (`tg64`) and prompt reading (`pp512`). It samples the performance-core clocks during each run and keeps the raw CSV per configuration.

## Finding 1: where the time of one extraction goes

The warm request is the normal case. The system prompt is already cached:

| Phase | Tokens | Time |
|---|---|---|
| System prompt (cached, reused) | about 2,350 | about 0 s, not recomputed |
| New prompt tokens (the SOS message) | 30–120 | about 1.7 s |
| **Generating the JSON** | **about 105–116** | **about 15–17 s (about 85% of the request)** |
| Total, warm | | about 17–20 s |
| **Total, first request after a server restart (cold)** | 2,389 prompt tokens | **about 58 s** (40.6 s of it reading the system prompt) |
| Landmark-refinement call (second, short prompt) | about 140 | about 2.7 s |

- **Prompt caching already works.** llama.cpp reuses the cached system prompt for every extraction, even after a landmark-refinement call with a different system prompt. The hypothesis that the two prompts evict each other from the single slot was **tested and rejected**.
- **The remaining cost is generation:** about 110 tokens of JSON, at about 7 tokens/s on the current default.
- **The cold start (about 58 s)** happens once per server start and can be removed with a warm-up request.
- **Two suspected costs turned out to be small:** applying the adapter at run time (about 7.0 tokens/s without it, about 6.7 with it) and the JSON-schema grammar (6.5 vs 6.6 tokens/s with and without it).

*The absolute times in this table were measured on battery power (see below). The proportions and the cache behaviour don't depend on power; the end-to-end times will be re-measured on mains power with the best configuration.*

## Finding 2: device and threads (mains power, 5 repetitions each)

Raw data (kept locally, not published): the summary TSV and one llama-bench CSV per configuration.

**Generation (`tg64`, tokens/s)**, the phase that dominates each request:

| Configuration | Tokens/s | vs. current default |
|---|---:|---:|
| **Integrated GPU via Vulkan (the package's default, used by llama-server today)** | **8.86** ± 0.01 | baseline |
| CPU, 4 threads | 11.41 ± 0.11 | +29% |
| CPU, 4 threads (repeat) | 11.49 ± 0.07 | +30% |
| **CPU, 4 threads, one per performance core** (`taskset -c 0,2,4,6`) | **11.62** ± 0.38 | **+31%** |
| CPU, 4 threads on CPUs 0–7 | 10.83 ± 0.06 | +22% |
| CPU, 6 threads | 10.45 ± 0.54 | +18% |
| CPU, 8 threads on performance cores 0–7 | 10.41 ± 0.06 | +17% |
| CPU, 8 threads | 9.75 ± 0.13 | +10% |
| CPU, 12 threads (all cores) | 8.95 ± 0.07 | +1% |

**Prompt reading (`pp512`, tokens/s):** integrated GPU 110.5; CPU with 4 or 6 threads 117.3–117.5 (the best); CPU with 8 or 12 threads, or pinned variants, 107.6–107.8.

**What this shows:**

1. **The current default is the slowest reasonable choice.** The CPU alone with 4 threads generates about 30% faster than the integrated GPU through Vulkan. The repeated run (11.41 vs 11.49) shows the difference is reproducible.
2. **More threads are slower.** Generation mostly waits for weights to arrive from RAM. Four performance cores already keep memory busy, so extra threads add synchronisation overhead. After each step every thread waits for the slowest one, and the efficiency cores and shared hyperthreads are slower.
3. **The best result is one thread per physical performance core.** It is within the spread of plain 4 threads, so pinning is a small, optional gain. Putting 4 threads on CPUs 0–7 without choosing the cores was worse (10.83), probably because two threads sometimes share one physical core.
4. **Memory bandwidth.** At 11.6 tokens/s the model reads about 2.5 GB of weights per token, about **29 GB/s**. That is likely below what this machine's memory can deliver, so some headroom may remain for build and kernel options (step 2 below).
5. **Caveat:** with `-ngl 0` this Vulkan build may still offload large prompt batches to the iGPU ("op offload"), so the CPU prompt-reading numbers may not be purely CPU. A strictly CPU-only run (`-dev none`) comes next.

**Practical consequence: superseded.** This table suggested that `-ngl 0 --threads 4` would speed up the server. The real server request (Finding 4) showed it does not: llama-bench's `tg64` generates into an empty context, unlike the real request (Finding 5).

## Finding 3: llama.cpp runtime options (pass 2, mains power, 5 repetitions)

Base: CPU, 4 threads, one per performance core (`taskset -c 0,2,4,6 … -ngl 0 -t 4`). Raw data kept locally.

| Option | Prompt pp512 (tokens/s) | Generation tg64 (tokens/s) |
|---|---:|---:|
| base | 117.0 ± 0.8 | 11.96 ± 0.23 |
| `-nopo 1` (no op offload to the iGPU) | 69.7 ± 0.6 | **16.11** ± 0.07 |
| `-dev none` (no device at all) | 73.0 ± 0.2 | 14.97 ± 0.14 |
| `--repack 0` | 116.6 | 12.04 |
| `-fa on` / `-fa off` | 117.6 / 113.3 | 11.93 / 11.52 |
| `--poll 0` / `--poll 100` | 117.3 / 117.4 | 11.89 / 11.98 |
| `-C 0x55 --cpu-strict 1` | 117.5 | 11.44 ± 0.65 |
| `--prio 2` | 117.4 | 11.98 |
| `-ub 256` / `-ub 1024 -b 2048` | 110.6 / 117.4 | 11.90 / 11.72 |
| base, repeated at the end | 117.4 | 11.79 |

- **Hidden op offload.** Even with `-ngl 0`, this Vulkan build sends some operations to the iGPU by default. That helps prompt reading but slows single-token generation. Turning it off gave +35% in this test, but only at an empty context; see Finding 5.
- **No measurable effect:** repacking, polling, priority and strict pinning. Flash attention is already on through `auto`. The default prompt batch of 512 is best.
- **Build facts found along the way:**
  - the package's CPU backend is already the Alder/Raptor Lake variant (`libggml-cpu-alderlake.so`, with AVX2 and AVX-VNNI), so a "native" rebuild for CPU instructions is unlikely to gain much;
  - the package is built **with assertions enabled** (`warning: asserts enabled, performance may be affected`). A release build without them is the remaining build-level option to test.

## Finding 4: the real server request (mains power)

`ml/scripts/local-inference/server_compare.sh` sends the same 9 requests to each configuration: a cold extraction, then warm extractions with refinement calls in between. Raw output kept locally.

| Configuration | Warm extraction (wall time) | Generation in the server | Cold first request |
|---|---|---|---|
| **Default** (iGPU, 8 threads) | **15.5–19.5 s** | 7.0–7.3 tokens/s | 49.9 s |
| CPU, 4 threads, `--no-op-offload` | 16.2–19.0 s | 7.3–7.6 tokens/s | 68.6 s |
| CPU, 4 threads, `-dev none` | 16.1–18.6 s | 7.1–7.4 tokens/s | 64.9 s |

- **No configuration is faster for warm requests.** The CPU configurations have a slower cold start, as their slower prompt reading predicts.
- **Answers are not identical across devices.** Each answer's content is fingerprinted (SHA-256, first 10 characters):
  - within each configuration, the same message always gives the same answer;
  - between the iGPU and CPU configurations, 2 of 5 messages produced a different JSON answer.

  The two code paths compute slightly different numbers, and at temperature 0 a tiny difference can flip a token. **A device change is therefore an answer-changing setting** and needs a blind-set accuracy run before adoption.

## Finding 5: why the server is slower than llama-bench

`ml/scripts/local-inference/diagnose_generation.sh` isolates the three differences between llama-bench's `tg64` and the real request. Raw output kept locally.

**Context depth (llama-bench tg64, 3 repetitions):**

| Configuration | Empty context | 2,400 tokens in context | Change |
|---|---:|---:|---:|
| iGPU (Vulkan) | 8.79 ± 0.11 | 8.09 ± 0.01 | −8% |
| CPU, op offload | 11.84 ± 0.08 | 6.82 ± 0.03 | −42% |
| CPU, `--no-op-offload` | 16.29 ± 0.09 | 7.87 ± 0.02 | −52% |

**Adapter and grammar (server, 4 extractions each):**

| Server | Generation |
|---|---|
| CPU with `--lora` / without | 7.2–7.6 / 7.2–8.0 tokens/s (adapter about 3–5%) |
| iGPU with `--lora` / without | 7.1–7.3 / 7.9–8.0 tokens/s (adapter about 10%) |
| CPU with `--lora`, no JSON-schema grammar | 7.3–7.5 tokens/s (grammar costs nothing; answers identical to those with the grammar) |

- **The cause is attention over the cached context.** Every generated token attends to all about 2,400 cached tokens. On the CPU this halves generation speed, and the iGPU handles it far better. At the real depth, all configurations run at about 7–8 tokens/s, which matches the server.
- **The default iGPU configuration stays.** It is as fast at the real depth, has the faster cold start, and is the configuration the SFT-V4 accuracy numbers were measured with.
- **The fine-tuned model produced schema-valid JSON for all 4 messages without the grammar.** The grammar is kept as a free safety net.

## Finding 6: the real pipeline, end to end (mains power)

**Where the 60–70 s figure came from.** It was never measured in the pipeline. It first appeared in the run instructions on 2026-10-31, next to "≈35 tokens/s prompt evaluation". At that speed, reading the 2,400-token system prompt alone takes about 68 s. So it most likely describes a request that read the whole prompt each time, from an older setup.

**The run.** `ml/scripts/local-inference/pipeline_run.sh` starts a fresh `llama-server` (default iGPU configuration, one slot), the six roles from the release build, and a `surge` run of 13 reports over 10 minutes (one every 45 s, below capacity, so reports do not queue). Synthetic reports only, `.env` settings unchanged, including `INFERENCE_TIMEOUT_SECS=60`. It samples memory and charger state every 5 s. It ran twice: before and after the warm-up below. Raw output kept locally. Both runs were cleaned up with `surge cleanup`.

| | Without warm-up | With warm-up |
|---|---:|---:|
| Model time per report, median | 19.8 s | 19.1 s |
| Model time per report, slowest | **53.8 s** (the first, cold request) | **21.5 s** |
| Triage time per report, median / slowest | 21.5 s / 54.0 s | 19.8 s / 25.3 s |
| Time to assignment, median / slowest | 20.4 s / 55.5 s | 20.2 s / 25.9 s |
| Matcher queue wait, median | 0.6 s | — (under 1 s) |
| Failed extractions, dead letters, verification failures | 0 / 0 / 0 | 0 / 0 / 0 |
| Lowest available memory during the run | 2.1 GB | 3.8 GB |

The llama.cpp log of the first run shows the split:

- **First request:** 2,387 prompt tokens in 38.4 s, then 106 tokens generated in 15.4 s.
- **Every later extraction:** 40–130 new prompt tokens (1.1–3.7 s) and 106–119 generated tokens at 6.3–7.2 tokens/s (15–18.5 s).
- **Landmark-refinement calls:** about 1.3–3.4 s each.

**What this shows:**

- **A warm extraction in the pipeline costs the same as in isolation** (Finding 1). Memory pressure, prompt eviction and the pipeline itself add nothing measurable. The matcher commits in under a second.
- **The cold first request was the only slow one.** At 54 s it was close to the 60 s client timeout. Under memory pressure or on battery it would have timed out, and the report would have gone to review.

**Fix: a start-up warm-up** (`TriageExtractor::warm_up`, called by the triage role). For a self-hosted backend (llama.cpp or vLLM), the role sends one request with the real system prompt and schema and `max_tokens: 1` before it reads any report. This request has its own 300 s timeout. The answer is discarded. `/readyz` reports `model_warm` once the warm-up has finished. If the warm-up fails, the role starts anyway with a warning, since extraction failures already go to review. Hosted endpoints (Gemini) skip it, because the call would only spend quota.

In the second run the warm-up took 40.1 s (2,363 prompt tokens). The first report after it reused the cached prompt: 40 new tokens in 1.6 s. **The slowest report fell from 54 s to 21.5 s.**

Generation, about 110 tokens at 6.3–7.2 tokens/s, is now the whole remaining cost. It is the target of speculative decoding (Finding 7).

## Finding 7: speculative decoding (mains power)

`ml/scripts/local-inference/spec_compare.sh` (target on the iGPU, the default) and `spec_compare_cpu.sh` (target on 4 performance-core threads, `--no-op-offload`) send the same 6 extractions to each configuration (the first one is cold) and fingerprint every answer. `measure.py` now also prints llama.cpp's draft counts. Raw output kept locally.

- **Draft model:** Qwen3-0.6B Q8_0 (`Qwen/Qwen3-0.6B-GGUF`, `sha256 9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`, same tokenizer as the 4B, no adapter), stored in the git-ignored `ml/model-cache/drafts/`.
- **N-gram speculation:** needs no draft model. It guesses from text already in the context.

**Warm extractions (requests 2–6):**

| Target | Speculation | Generation (tokens/s) | Accepted draft tokens | Answers vs. its own baseline |
|---|---|---:|---:|---|
| **iGPU** | none (baseline) | **6.7–6.8** | — | — |
| iGPU | draft, up to 3 tokens | 5.6–6.4 | 72–86% | identical |
| iGPU | draft, up to 8 | 2.8–3.3 | 55–74% | identical |
| iGPU | draft, up to 16 | 2.3–3.5 | 38–59% | identical |
| iGPU | `ngram-simple` | 4.6–5.1 | 8–15% | identical |
| iGPU | `ngram-mod` | 6.4 → **9.3** (rising per request) | 29–45% | **1 of 5 changed** |
| CPU | none (baseline) | 6.0–6.3 | — | — (2 of 5 differ from the iGPU, as in Finding 4) |
| CPU | draft on the iGPU, up to 3 | 4.8–5.3 | 72–86% | identical |
| CPU | draft on the iGPU, up to 8 | 3.3–4.2 | 55–74% | identical |
| CPU | draft on the CPU, up to 3 | 4.5–5.1 | 72–86% | identical |

**What this shows:**

- **The draft model predicts the fine-tuned JSON well.** It is not fine-tuned, yet with up to 8 tokens the target accepts about 6.5 tokens per verification pass.
- **Every draft-model configuration is still slower, on both devices.** A pass that checks several tokens costs far more here than one single-token step. On the iGPU, a pass over about 7 tokens takes about 2 s, against about 150 ms for one token. Two causes are likely:
  - the Vulkan build takes a slower route for small multi-token batches (compare the hidden op offload in Finding 3);
  - attention over the 2,400 cached tokens is roughly half of each step (Finding 5), and that part grows with every token in the pass instead of being shared.
- **`ngram-mod` is the only variant that sped up.** It learns from earlier answers and reached 9.3 tokens/s by the sixth request, about 37% faster. **But it changed one answer.** Request 4 produced different JSON (103 tokens instead of 105): the same answer the CPU baseline gives for that message.
  - Greedy speculative decoding is lossless only in exact arithmetic. Checking several tokens in one batch computes slightly different numbers, and at temperature 0 that can flip a token on a borderline message.

**Decision so far:** no speculative configuration is adopted.

- The draft-model variants are slower everywhere on this hardware.
- `ngram-mod` changes answers, so it could only be adopted after a full blind-set accuracy run against the SFT-V4 gates. Its gain also depends on a stream of similar requests warming it up.
- The idea is not dead for the T4. A GPU verifies a batch of tokens at nearly the cost of one, which is where speculative decoding normally pays off, so the T4 serving study tests it there (vLLM n-gram speculation is already in plan ML-S1).

## Measurement pitfall: battery power

The first round of measurements was taken **on battery**. On battery the CPU's power limits change from minute to minute, so the same setting gave 8.6 tokens/s at one moment and 3.4–5.3 a few minutes later. Those numbers are discarded. Every number in Finding 2 was measured on mains power in the performance profile, with the charger state checked before and after the run.

## Next steps (to be added to this report)

Done: pass 2 (Finding 3), the end-to-end server request (Finding 4), and the cause of slow generation (Finding 5).

Order revised on 2026-11-03: the steps that address the real pipeline come first.

1. ~~The real pipeline, end to end~~: done (Finding 6), with the start-up warm-up.
2. ~~Speculative decoding~~: done (Finding 7). Not adopted. The draft model is slower on both devices; `ngram-mod` is faster but changed one answer.
3. **Write up the local study**, then move to the vLLM study on a Kaggle T4.

Lower priority, possibly skipped:

- **KV-cache quantization** at `-d 2400` (`-ctk/-ctv` q8_0, q4_0). The cache at 2,400 tokens is about 350 MB, against about 2.5 GB of weights read per token, so the expected gain is a few percent, and it changes the arithmetic (answers must be checked). At most one short check.
- **Forced-token share** of the JSON output: a measurement only, since llama-server has no schema-aware ("jump-forward") decoding to act on it.
- **A shorter system prompt** (an ML change: a new prompt version, with a full blind-set accuracy run against the SFT-V4 gates).
- **Optional release build** of llama.cpp **without assertions**, compared with the package build. The CPU-specific code is already present (Finding 3).
- **Changes that alter the answers** (other quantizations, a merged adapter) only with a blind-set accuracy run against the SFT-V4 gates.
