# Local inference optimisation: Qwen3-4B + SFT-V4 on the laptop CPU

Status: **in progress** (2026-11-02 to 2026-11-03). This report records the measurements so far. Later steps are listed at the end and will be added here.

**Summary so far:**
- A warm extraction takes about 16–19 s. Almost all of it is generating about 110 JSON tokens at about 7–8 tokens/s.
- The system prompt is already cached.
- Device, thread and llama.cpp runtime options do not speed up the real request: at the real context length (about 2,400 cached tokens), **attention over the cached system prompt** costs roughly half the generation speed on the CPU, and every setting lands at about 7–8 tokens/s.
- The adapter costs about 3–10% and the JSON grammar nothing.
- Changing device (iGPU to CPU) changes some greedy answers, so it is not a speed-only setting.
- The remaining levers are cheaper or shorter attention (KV-cache quantization, a shorter prompt) and fewer passes per token (speculative or schema-aware decoding).

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

Raw data: `local-inference/matrix-ac-2026-11-02.tsv`, plus one llama-bench CSV per configuration in `local-inference/matrix-ac-2026-11-02-raw/`.

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

Base: CPU, 4 threads, one per performance core (`taskset -c 0,2,4,6 … -ngl 0 -t 4`). Raw data: `local-inference/pass2-2026-11-02.tsv` and `pass2-2026-11-02-raw/`.

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

`ml/scripts/local-inference/server_compare.sh` sends the same 9 requests to each configuration: a cold extraction, then warm extractions with refinement calls in between. Output: `local-inference/server-compare-2026-11-02/`.

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

`ml/scripts/local-inference/diagnose_generation.sh` isolates the three differences between llama-bench's `tg64` and the real request. Output: `local-inference/diagnose-2026-11-03/`.

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

## Measurement pitfall: battery power

The first round of measurements was taken **on battery**. On battery the CPU's power limits change from minute to minute, so the same setting gave 8.6 tokens/s at one moment and 3.4–5.3 a few minutes later. Those numbers are discarded. Every number in Finding 2 was measured on mains power in the performance profile, with the charger state checked before and after the run.

## Next steps (to be added to this report)

Done: pass 2 (Finding 3), the end-to-end server request (Finding 4), and the cause of slow generation (Finding 5).

Order revised on 2026-11-03: the steps that address the real pipeline come first.

1. **The real pipeline, end to end (mains power).** In the running CrisisGraph pipeline, an extraction was quoted at about 60–70 s, while the isolated warm request takes about 17–20 s (Finding 1). Measure SOS to assignment with the per-request timings, and attribute the gap: cold starts, memory pressure with all roles running, earlier battery runs, or the pipeline's own retries and waits. If cold starts explain it, add a warm-up request and measure again.
2. **Speculative decoding** with a small draft model (Qwen3-0.6B): speed, acceptance rate, identical answers. Each verification pass checks several draft tokens against the same cached context, so this also spreads the attention cost identified in Finding 5. It is the only remaining lever aimed directly at generation.
3. **Write up the local study**, then move to the vLLM study on a Kaggle T4.

Lower priority, possibly skipped:

- **KV-cache quantization** at `-d 2400` (`-ctk/-ctv` q8_0, q4_0). The cache at 2,400 tokens is about 350 MB, against about 2.5 GB of weights read per token, so the expected gain is a few percent, and it changes the arithmetic (answers must be checked). At most one short check.
- **Forced-token share** of the JSON output: a measurement only, since llama-server has no schema-aware ("jump-forward") decoding to act on it.
- **A shorter system prompt** (an ML change: a new prompt version, with a full blind-set accuracy run against the SFT-V4 gates).
- **Optional release build** of llama.cpp **without assertions**, compared with the package build. The CPU-specific code is already present (Finding 3).
- **Changes that alter the answers** (other quantizations, a merged adapter) only with a blind-set accuracy run against the SFT-V4 gates.
