# Qwen3 4B versus Gemma 3 4B

Date: 2026-09-11

## Question

Does changing the model family from Qwen3 4B to Gemma 3 4B improve CrisisGraph
triage extraction when the prompt and evaluation contract remain fixed?

## Fixed conditions

- Runtime: llama.cpp build 10809
- Quantization: Q4_K_M
- CPU threads: 8
- Context per slot: 2,048 tokens
- Concurrency: 1
- Prompt: `triage-extraction-v2.2`
- Schema: `triage-extraction-v2`
- Dataset: `triage-eval-v1.jsonl` (12 synthetic development cases)
- Temperature: 0.0
- Maximum output: 512 tokens

The Qwen result influenced development of Prompt V2.2, so this is not an
unbiased model benchmark. It is an initial compatibility and development-set
comparison.

## Models

| Label | Runtime model | Parameters reported by llama.cpp |
|---|---|---:|
| Qwen | `Qwen/Qwen3-4B-Instruct-2507` | approximately 4B |
| Gemma | `google/gemma-3-4b-it` | 3.88B |

## Results

| Metric | Qwen3 4B | Gemma 3 4B |
|---|---:|---:|
| Schema validity | 100.0% | 100.0% |
| Critical-field accuracy | **90.0%** | 83.3% |
| Critical exact match | 50.0% | 50.0% |
| Human-review recall | 100.0% | 100.0% |
| Cases with unsupported facts | **16.7%** | 41.7% |
| Unsupported facts | **2** | 6 |
| Mean service latency | **13.8 s** | 28.9 s |
| P50 service latency | **13.7 s** | 26.0 s |
| P95 service latency | **19.9 s** | 38.4 s |

### Per-field accuracy

| Field | Qwen3 4B | Gemma 3 4B |
|---|---:|---:|
| Victim location | **91.7%** | 83.3% |
| Headcount | 91.7% | 91.7% |
| Required asset | 91.7% | **100.0%** |
| Hazards | **91.7%** | 66.7% |
| Raw review decision | **83.3%** | 75.0% |

## Gemma failure pattern

Gemma's dominant failure was treating general emergency language as evidence of
a road hazard. It created hazards for otherwise complete reports containing
phrases such as "trapped" and also created an `OPEN` hazard where no road status
was stated. This caused both unsupported facts and unnecessary human review.

Other errors included:

- returning `stranded` as a location when no location was given;
- choosing five from a contradictory "four ... maybe five" headcount;
- retaining a Malayalam relational location suffix; and
- normalizing `road near Bank Junction` to `near Bank Junction`, which failed
  the strict expected-value comparison.

Gemma correctly classified the requested asset in all 12 development cases and
never missed a required human-review case.

## Interpretation

Qwen remains the stronger triage candidate under the current shared prompt,
mainly because hazard extraction is the safety-sensitive center of this task.
Gemma is nevertheless compatible with the llama.cpp JSON Schema path and is a
useful cross-family control.

The latency figures are observational, not a controlled performance benchmark.
The model runs were not randomized or repeated, and generated response length,
cold state, prefix caching, and CPU conditions can affect service latency.

## Next decision

Do not tune the shared prompt to Gemma's six known failures yet. First expand
the dataset and reserve an untouched test split. Then rerun both models with the
same frozen prompt. If Gemma remains interesting, a separate Gemma-specific
prompt can be evaluated as a second experiment rather than mixed into the
model-only comparison.
