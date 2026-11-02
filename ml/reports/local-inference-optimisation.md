# Local inference optimisation: Qwen3-4B + SFT-V4 on the laptop CPU

Status: **in progress** (2026-11-02). This report records the measurements so far. Later steps are listed at the end and will be added here.

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

**Practical consequence:** running `llama-server` with `-ngl 0 --threads 4` (optionally `taskset -c 0,2,4,6`) should cut warm generation from about 15–17 s to about 11–12 s, without changing the model or its answers. This will be confirmed with the real server request.

## Measurement pitfall: battery power

The first round of measurements was taken **on battery**. On battery the CPU's power limits change from minute to minute, so the same setting gave 8.6 tokens/s at one moment and 3.4–5.3 a few minutes later. Those numbers are discarded. Every number in Finding 2 was measured on mains power in the performance profile, with the charger state checked before and after the run.

## Next steps (to be added to this report)

1. **Pass 2 (llama.cpp options):** a strictly CPU-only run (`-dev none`), weight repacking on/off, flash attention, thread polling, strict pinning and priority, batch sizes for the cold start.
2. **End-to-end server request** with the best configuration against the current default, on mains power.
3. **Forced-token share:** how many of the about 110 generated tokens are dictated by the JSON schema (field names, quotes, punctuation). That share is the potential of schema-aware ("jump-forward") decoding.
4. **Speculative decoding** with a small draft model (Qwen3-0.6B): speed, acceptance rate, identical answers.
5. **Optional native build** of llama.cpp for this CPU (`GGML_NATIVE`, no Vulkan, BLAS), mainly for the cold start.
6. **Changes that alter the answers** (other quantizations, a merged adapter) only with a blind-set accuracy run against the SFT-V4 gates.
