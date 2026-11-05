# Serving Qwen3-4B + SFT-V4 on one T4 with vLLM (study ML-S1)

Status: **complete** (2026-11-03 to 2026-11-05). Plan ML-S1, approved 2026-11-02. The notebooks are tracked:

- `ml/notebooks/crisisgraph-sft-v4-serving-kaggle.ipynb`: sessions 1 and 1b, variant B;
- `ml/notebooks/crisisgraph-sft-v4-serving-kaggle-session2.ipynb`: session 2, B with guidance, and variant A.

Aggregate results are in `qwen3-4b-sft-v4-serving-t4-results.json`. Per-case outputs and server logs are kept locally and not published, because they contain the blind test set.

**Summary:**
- **vLLM 0.11.0 serves the SFT-V4 model on one free Kaggle T4** (FP16 base with the LoRA adapter applied at run time). Its accuracy on the blind v3.1.0 set is as good as or better than the laptop on four of the five parity gates:

  | | T4 | Laptop |
  |---|---|---|
  | V2 exact | 85 | 81 |
  | Critical under-triage | 6 | 7 |
  | Review recall | 0.925 | 0.906 |
  | UNCONSCIOUS recall | 84.6% | 77% |

- **It misses the invented-fact gate by one case:** 35 against the limit of 34 (local 31). By the rule declared before the study, this is reported as a missed gate, and the threshold is not moved. The result repeated exactly in a second run.
- **The grammar library limits throughput, not the GPU.** vLLM's `auto` choice (xgrammar) levels off at about 12 extractions/min. With **guidance** (`--structured-outputs-config '{"backend":"guidance"}'`), the server gives **identical outputs on all 120 cases** and reaches **39 extractions/min at 32 in flight**. Single-request latency improves too, from p50/p95 15.0/22.4 s to **12.4/14.9 s**.
- **On the laptop CPU** one extraction takes about 16–20 s, one at a time: about 3 per minute. One T4 with guidance handles about 13 times as many.
- **Not done:**
  - Variant A (llama.cpp CUDA) failed to build on Kaggle's current image.
  - Variants C and D were dropped once the bottleneck was found.
  - The CrisisGraph surge through the T4 (Run C) was not run.

  So the cause of the invented-fact gap is **likely, not proven**, to be the different weight format.

## What was measured

| Item | Value |
|---|---|
| GPU | 1 × Tesla T4, 15,360 MiB, compute capability 7.5, driver 580.178.04. The session had two T4s; `CUDA_VISIBLE_DEVICES=0` used one. |
| Host | Kaggle notebook, Intel Xeon @ 2.00 GHz, 31.3 GB RAM |
| Serving stack | vLLM 0.11.0 (own Python 3.12 venv), torch 2.8.0+cu128, transformers 4.57.1 (pinned: transformers 5 breaks vLLM 0.11's tokenizer use), peft 0.21.2 |
| Base model | `Qwen/Qwen3-4B-Instruct-2507` at revision `cdbee75f17c01a7cc42f958dc650907174af0554` (the SFT-V4 training revision), FP16 |
| Adapter | SFT-V4 PEFT adapter, applied at run time (`--enable-lora --max-lora-rank 16`) |
| Prompt and schema | `ml/prompts/triage-extraction-v3.2.txt` (`9368283c…`), `schemas/triage-extraction-v3.json` (`c9b3ed10…`), sent exactly as the Rust extractor sends them (`structured_outputs.json`) |
| Server flags | `--dtype half --max-model-len 4096 --gpu-memory-utilization 0.90 --seed 0 --generation-config vllm`, prefix caching on (default), `VLLM_ATTENTION_BACKEND=TRITON_ATTN` |
| Accuracy runs | blind `triage-eval-v3.1.0` (120 cases), serial, temperature 0, max 768 tokens, scored with urgency-rules-v5 exactly as in `qwen3-4b-qlora-sft-v4-evaluation.md`, plus the 92 cases without a close training neighbour |
| Speed runs | `crisisgraph_ml.serving_bench`: streamed real extraction requests, 64 per concurrency level, replaying the test prompts in order. It measures time to first token, end-to-end latency, requests/s and output tokens/s, and fingerprints every answer to detect changes. |
| Bundle | private Kaggle dataset from `ml/scripts/package_kaggle_dataset.sh` at commit `950eb3f`, with every file checked against `SHA256SUMS` in the notebook |

The local reference is the SFT-V4 evaluation: llama.cpp on the laptop CPU, the Q4_K_M base and `adapter-f16.gguf`.

## Finding 1: accuracy, B against the local reference

| Measure | Gate | Local (llama.cpp, Q4_K_M) | **B (vLLM, FP16 + LoRA)** |
|---|---|---:|---:|
| Usable outputs | = 120 | 120 | **120** ✅ |
| Critical under-triage (of 57 CRITICAL) | ≤ 8 | 7 | **6** ✅ |
| V2 fields exact | ≥ 78 | 81 | **85** ✅ |
| Cases with invented facts | ≤ 34 | 31 | **35** ❌ |
| V2 review recall | ≥ 0.89 | 0.906 | **0.925** ✅ |
| UNCONSCIOUS recall | — | 77% | **84.6%** |
| BREATHING_DIFFICULTY / CARDIAC / BLEEDING recall | — | 91 / 100 / 100% | 91 / 100 / 100% |
| Remote-caller miss | — | 7.7% | 7.7% |
| Latency p50 / p95 (serial) | — | 18.2 s (CPU) | 15.1 / 22.1 s |

**On the 92 cases without a close training neighbour:**

| | Local | B |
|---|---|---|
| V2 exact | 63 | 68 |
| Critical under-triage | 5 | 4 |
| Invented-fact cases | 22 | 26 |

The pattern is the same as on all 120: better on every measure except invented facts.

**Repeatability:** B was evaluated twice, in separate Kaggle sessions on 2026-11-04 and 2026-11-05. Both runs gave the same headline, including the same six under-triaged cases, so the result is not run-to-run noise.

**Where the extra invented facts come from:**
- **The T4 and the laptop give identical outputs on 61 of 120 cases.** That's expected, because the weights differ: FP16 with the adapter applied at run time, against a 4-bit quantised base with a GGUF adapter.
- **The flagged cases overlap but are not the same:**
  - 27 are flagged by both;
  - 8 only by B;
  - 4 only locally.
- **The kinds of invented fact are the same mix in both runs:** an extra medical condition or vulnerable group, or a profile field such as "reporter on scene" filled in where the label leaves it empty.
- **A swing of ±4 cases is about one standard deviation** for a count near 33 of 120.
- **Three of B's new cases add a medical condition the message doesn't support** (bleeding or unconscious). These raise urgency (over-triage): the safer direction, but still dispatcher time.

The planned check, variant A (llama.cpp CUDA with the same 4-bit files on the T4), would have separated the weight format from the engine. It did not build (see Deviations).

## Finding 2: throughput is limited by the grammar library

**The full sweep for B** (vLLM `auto` grammar backend, which chose xgrammar):

| Concurrency | Extractions/min | Output tokens/s | End-to-end p50 |
|---:|---:|---:|---:|
| 1 | 3.8 | 7.8 | 14.9 s |
| 2 | 5.8 | 12.0 | 20.3 s |
| 4 | 8.0 | 16.5 | 28.1 s |
| 8 | 9.8 | 20.3 | 45.9 s |
| 16 | 10.8 | 22.4 | 83.8 s |
| 32 | 11.5 | 23.8 | 164.0 s |

**It levels off at about 11–12 extractions/min while latency keeps rising.** The KV cache was not the limit: about 17% was used, since every request shares the cached system prompt, and time to first token stayed low.

**Diagnostic at 16 in flight** (32 requests, same server; the outputs without the grammar were not scored):

| | Extractions/min |
|---|---:|
| Adapter + grammar (as served) | 12.2 |
| Adapter, **no grammar** | **30.2** |
| Base model, no grammar | 36.0 |
| Base model + grammar | 7.7 |

- **Removing the grammar gives about 2.5× the throughput.** Removing the adapter adds only about 20% on top. So the ceiling is the grammar work, not the GPU.
- **The base model with the grammar is slower than the adapter with the grammar.** This is not explained. The base model's output length is similar (about 135 against 123 tokens per request).

**Grammar libraries compared** (speed settings, 64 requests per level):

| Setting | c1 p50 / p95 | c8 | c32 | c32 p50 | Answers changed vs B |
|---|---|---:|---:|---:|---|
| xgrammar (as `auto`) | 15.0 / 22.4 s | 10.0/min | 12.2/min | 151 s | 1 case at c8 |
| **guidance** | **12.3 / 14.9 s** | **22.9/min** | **39.5/min** | **45 s** | 1 case at c1 and c8 |
| no prefix cache | 18.5 / 25.8 s | 6.2/min | 5.8/min | 315 s | 1 case at all levels |

- **The one case that changes is the same case in every setting,** including xgrammar itself at c8. It is a borderline case whose answer changes with batching, not an effect of any setting.
- **Parity check:** 20 cases at 16 in flight on the B server gave answers identical to the serial run.
- **Prefix caching is essential.** Without it every request re-reads the 2,380-token system prompt: time to first token reaches 220 s at c32, and throughput halves.

**Attention backend:** on the T4 (no FlashAttention 2, which needs compute capability 8.0), vLLM 0.11 picks FlexAttention by default.

| Backend | Base model, single request |
|---|---|
| FlexAttention (default) | 9.1 tokens/s |
| **Triton** (`VLLM_ATTENTION_BACKEND=TRITON_ATTN`) | **19.4 tokens/s** |
| FlashInfer | not installed with vLLM 0.11 |

All results in this report use Triton.

## Finding 3: B with guidance, full accuracy run

Session 2 served B with `--structured-outputs-config '{"backend":"guidance"}'` and ran the full blind set and a speed run on the same server.

- **Accuracy: the outputs are identical to B on all 120 cases,** so every measure is the same as B in Finding 1, including the one-case miss on invented facts. The grammar library changes speed only.
- **Speed:**

| Concurrency | Extractions/min | Output tokens/s | End-to-end p50 / p95 | Time to first token p50 |
|---:|---:|---:|---|---:|
| 1 | 4.7 | 9.8 | 12.4 / 14.9 s | 0.15 s |
| 8 | 22.5 | 46.8 | 20.1 / 24.2 s | 0.35 s |
| 32 | **39.2** | 81.2 | 45.5 / 54.9 s | 0.87 s |

These repeat session 1b's guidance speeds (39.5/min at c32) to within 1%.

**What is left at one request at a time:** with guidance, a single request runs at about the speed of the adapter without any grammar: 9.8 tokens/s, against 9.6 in the session 1 single-request split. The adapter applied at run time is now the main cost for one request: the base model without the adapter reached 19.4 tokens/s. Merging the adapter into the weights (variant C) is therefore the next lever for **latency**. It was dropped here because the throughput question was already answered (see Deviations).

## Recommendation

**For a GPU deployment of SFT-V4 on a T4-class card:**
- Serve with vLLM, the adapter at run time, prefix caching on, Triton attention, and the **guidance** grammar library:

  ```bash
  VLLM_ATTENTION_BACKEND=TRITON_ATTN vllm serve Qwen/Qwen3-4B-Instruct-2507 \
    --revision cdbee75f17c01a7cc42f958dc650907174af0554 --dtype half --max-model-len 4096 \
    --enable-lora --lora-modules sftv4=<adapter> --max-lora-rank 16 \
    --generation-config vllm --structured-outputs-config '{"backend":"guidance"}'
  ```

  CrisisGraph needs no code change for this: set `INFERENCE_BACKEND=vllm` and `INFERENCE_MODEL=sftv4`. The schema sent to vLLM already omits `uniqueItems`, which vLLM's grammar libraries reject (`950eb3f`). Outputs are still validated against the full schema.
- **Size the triage workers to the latency you need:**

  | Requests in flight | Extractions/min | p95 latency | Use |
  |---|---|---|---|
  | about 8 | about 22 | 24 s | about the laptop's single-request latency, at 7× the throughput |
  | 32 | about 39 | 55 s | surge throughput |

  Set `STREAM_CLAIM_IDLE_MS` to about 3× the measured p99 for the concurrency chosen.
- **The accuracy caveat stays:** this configuration misses the invented-fact gate by one case. Invented profile facts can raise urgency, never lower it. Urgency is derived in Rust (urgency-rules-v5), and the model's suggestion can only raise it. Dispatcher review of the victim profile is still required, as it is for the local model.

**Cost, as an assumption:** at an example on-demand price of **USD 0.35 per T4-hour** (an assumption, not a quote), the configuration above costs about:

| Load | Extractions/hour | Cost per 1,000 extractions |
|---|---|---|
| Fully loaded at 32 in flight | 2,352 | USD 0.15 |
| One request at a time | 282 | USD 1.24 |

The Kaggle GPU used here is free, with a weekly quota, and is only suitable for time-boxed sessions, not hosting.

## Deviations from the plan

- **Variant A (llama.cpp CUDA reference) did not run.** Building llama.cpp `b11429` with `-DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=75` failed at the CMake configuration step on Kaggle's current image, which ships CUDA 13.0. The notebook kept the exit status but not CMake's message, so the cause is unknown. Without A, the invented-fact gap is attributed to the weight format only as the likely explanation.
- **Variants C (merged weights), D (published AWQ) and E (own quantisation) were dropped** after the diagnostic showed the adapter costs only about 20% of throughput. C remains the next step for single-request latency (Finding 3).
- **Two speed settings from the plan were not measured:** a maximum batch size and n-gram speculative decoding. The session budget went to the grammar comparison, which decided the result.
- **The CrisisGraph surge through the T4 (plan step 4, Run C) was not run.** `docs/load-test-results.md` has no T4 run.
- **No accuracy-against-throughput chart:** both measured configurations have identical outputs, so a trade-off chart would show one accuracy point.
- **Session 1's result files were lost** when the Kaggle session ended before the download. The sweep table in Finding 2 comes from the numbers the notebook printed. Session 1b reran the accuracy run and the reference level unattended (Save & Run All); the later results come from its files.
- **The bundle used in session 1 was built before the `uniqueItems` fix.** The fix was applied in that session by redefining the function in the notebook, then rebuilt into the bundle (`950eb3f`) for sessions 1b and 2.
- **Privacy note:** the blind set was only sent to the model served inside the private notebook. During the analysis on 2026-11-05, one case's labels and model output were displayed to the AI coding assistant (a hosted model) while the result files were inspected. After that, the analysis used case IDs and label names only. The set is not used for training, and one case does not change any decision in this report. It is recorded here for completeness.
