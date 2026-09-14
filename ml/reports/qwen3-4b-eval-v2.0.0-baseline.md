# Qwen3-4B frozen Triage Eval V2.0.0 baseline

Date: 2026-09-12

## Decision summary

Qwen3-4B reliably obeyed the JSON Schema but did not meet the semantic safety
target. Schema validity was 100%, while critical-field accuracy was 85.2%,
exact match was 55.0%, and raw human-review recall was 66.2%. The deterministic
null/schema guard improved effective review recall to 86.8%, but nine cases
still escaped review because the model confidently replaced ambiguity with a
concrete value.

This result supports keeping Qwen3-4B as the efficient experimental base model,
but it is not adequate as a standalone dispatch gate. The Rust pipeline must
continue to fail closed, and model output must remain candidate information.

## Frozen inputs

- Dataset: `data/triage-eval-v2.0.0.jsonl` (120 independently reviewed cases)
- Dataset SHA-256: `08f6ab7d2513583599db4288fbf894de654dbc1e03251506d920410ad5ff58d4`
- Prompt: `prompts/triage-extraction-v2.1.1.txt`
- Prompt SHA-256: `2484578610bc5852cf31bbdd1712091b081fb6411f94ec6dd91813daba376ef5`
- Schema: `schemas/triage-extraction-v2.json`
- Schema SHA-256: `177a9f8e22973f980b61dc47cd6d6637e4994768c2c71bbf513cd4bcabc2ba0f`

The prompt and contract were fixed before this run. Individual outputs were not
used to alter the run while it was executing.

## Runtime

- Base model: `Qwen/Qwen3-4B-Instruct-2507`
- GGUF source: `lmstudio-community/Qwen3-4B-Instruct-2507-GGUF`, snapshot
  `4edb920b6f14e3b9284d4502a6485103d72cde05`
- Quantization: Q4_K_M
- Artifact SHA-256: `8cdb57cbb880d313736a9bc4e3d3d2485f145b5e19cf33783746e753e82641fc`
- llama.cpp: build 10809
- CPU threads: 8
- Context: 2,048 tokens
- Parallel slots / evaluation concurrency: 1 / 1
- Temperature / maximum output: 0.0 / 512 tokens

The complete ignored run artifact is
`runs/qwen3-4b-q4-k-m-c1-prompt-v2.1.1-eval-v2.0.0.json`.

## Overall results

| Metric | Result |
|---|---:|
| Schema validity | 100.0% |
| Critical-field accuracy | 85.2% |
| Exact match | 55.0% (66/120) |
| Raw human-review recall | 66.2% (45/68) |
| Raw human-review precision | 86.5% (45/52) |
| Raw review-decision accuracy | 75.0% |
| Effective review recall after deterministic null/schema guard | 86.8% (59/68) |
| Cases with unsupported facts | 16.7% (20/120) |
| Unsupported facts | 20 |
| Inference errors | 0 |

### Per-field accuracy

| Field | Accuracy |
|---|---:|
| Victim location | 83.3% |
| Headcount | 87.5% |
| Required asset | 86.7% |
| Hazards | 93.3% |
| Raw review decision | 75.0% |

## Results by category

Review recall is not applicable to `clear`, which has no positive review cases.

| Category | Cases | Critical fields | Exact match | Raw review recall | Effective recall | Unsupported-fact cases |
|---|---:|---:|---:|---:|---:|---:|
| Clear | 20 | 98.0% | 95.0% | N/A | N/A | 0.0% |
| Missing information | 20 | 88.0% | 50.0% | 65.0% | 90.0% | 10.0% |
| Contradictory | 15 | 82.7% | 40.0% | 80.0% | 86.7% | 60.0% |
| Hazard | 20 | 86.0% | 55.0% | 27.3% | 63.6% | 25.0% |
| Malayalam | 15 | 72.0% | 26.7% | 57.1% | 100.0% | 6.7% |
| Manglish | 15 | 76.0% | 40.0% | 85.7% | 85.7% | 13.3% |
| Adversarial | 8 | 77.5% | 37.5% | 0.0% | 100.0% | 12.5% |
| Irrelevant | 7 | 100.0% | 100.0% | 100.0% | 100.0% | 0.0% |

## Results by language

| Language | Cases | Critical fields | Exact match | Raw review recall | Effective recall | Unsupported-fact cases |
|---|---:|---:|---:|---:|---:|---:|
| English | 90 | 88.9% | 62.2% | 64.8% | 85.2% | 18.9% |
| Malayalam | 15 | 72.0% | 26.7% | 57.1% | 100.0% | 6.7% |
| Manglish | 15 | 76.0% | 40.0% | 85.7% | 85.7% | 13.3% |

Multilingual results must not be folded into a single aggregate conclusion:
some cases are semantic mirrors designed to measure cross-lingual consistency,
and Malayalam exact match is substantially weaker than English.

## CPU latency

| Metric | Time |
|---|---:|
| Mean | 15.45 s |
| P50 | 14.12 s |
| P95 | 22.35 s |
| Serial wall-clock inference time | 30.89 min |

These are observational CPU figures for one run, not a controlled serving
benchmark or a time-to-first-token measurement.

## Safety-relevant failure patterns

The raw model missed 23 of 68 required-review cases. The deterministic
null/schema rule recovered 14, leaving these nine effective false negatives:

- `v2-missing-information-011`
- `v2-missing-information-012`
- `v2-contradictory-008`
- `v2-contradictory-009`
- `v2-hazard-005`
- `v2-hazard-006`
- `v2-hazard-018`
- `v2-hazard-019`
- `v2-multilingual-manglish-008`

The dominant failure is not malformed JSON. It is confident semantic
substitution: choosing one side of a contradiction, treating a generic road
description such as `surrounding road` as a resolved road locator, or filling
another ambiguous field. Schema constraints cannot detect those errors because
the invented value has the correct type.

The model also rejected all otherwise valid facts in some multilingual and
prompt-injection cases. That behavior is fail-closed, but it reduces extraction
coverage and contributes to poor exact match.

## Conclusion and next gate

The configured 99% raw human-review recall target was not met. Fine-tuning is a
reasonable controlled experiment, but training examples must be generated
independently of this frozen file. Because this benchmark has now been examined
to guide the next experiment, improvements measured on it are regression
results rather than a new unbiased generalization estimate. A separately
reviewed blind holdout will be required before making a final model claim.
