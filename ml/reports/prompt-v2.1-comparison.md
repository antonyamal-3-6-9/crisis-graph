# Triage prompt V2.1 comparison

Date: 2026-09-10

## Fixed evaluation conditions

- Model: `Qwen/Qwen3-4B-Instruct-2507`
- Quantization: GGUF Q4_K_M
- Runtime: llama.cpp build 10809
- CPU threads: 8
- Context per slot: 2,048 tokens
- Concurrency: 1
- Schema: `triage-extraction-v2`
- Dataset: `triage-eval-v1.jsonl` (12 synthetic development cases)

The dataset is a development seed set, not an untouched test set. These results
must not be presented as production reliability evidence.

## Results

| Metric | Prompt V2.0 | Prompt V2.1 |
|---|---:|---:|
| Schema validity | 100.0% | 100.0% |
| Critical-field accuracy | 76.7% | 88.3% |
| Critical exact match | 41.7% | 58.3% |
| Human-review recall | 100.0% | 42.9% |
| Cases with unsupported facts | 33.3% | 8.3% |
| Mean service latency | 27.8 s | 17.4 s |
| P50 service latency | 26.3 s | 17.0 s |
| P95 service latency | 36.6 s | 20.8 s |

## Interpretation

V2.1 eliminated most empty all-null hazard objects, preserved headcount and asset
facts more reliably, and produced shorter responses. However, it frequently
returned `needs_human_review=false` while one or more critical fields were null.
That regression is safety-critical and prevents V2.1 from being selected as-is.

The experiment shows that `needs_human_review` should not be trusted as an
independent model decision. A deterministic post-validation policy must force
review whenever a dispatch-critical field is null, a reported hazard is
incomplete, schema validation fails, or extraction fails. The model-provided
value can remain an additional reason to escalate but must never suppress the
deterministic decision.

## Next experiment

Implement and test the deterministic review policy before another prompt
iteration. Report both raw model review recall and effective post-policy review
recall so model behavior is not hidden by the safety guard.
