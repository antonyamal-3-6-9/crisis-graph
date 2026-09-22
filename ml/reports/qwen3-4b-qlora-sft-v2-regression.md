# Qwen3-4B QLoRA V2: frozen regression comparison

Completed 2026-09-21. Decision: do not promote V2 over V1 as the default extractor.
V2 partly repairs contradiction handling but regresses on hazards, Malayalam,
and aggregate extraction. Neither adapter meets the documented 99% review-recall
target. A larger 1,000-record run is not justified by this result alone.

## Controlled inputs and execution

All three runs use the same frozen 120 cases, expected labels, V2.1.1 prompt,
JSON schema, Qwen3-4B-Instruct-2507 Q4_K_M base, llama.cpp build 10809/commit
5266f24da7, eight threads, 2,048-token context, one slot, temperature 0 and a
512-token output limit. Base/V1 figures are from their saved runs, not newly
rerun inference. V2 used the separately loaded F16 adapter at default scale.
Detailed hashes and settings are in `qwen3-4b-qlora-sft-v2-evaluation-config.json`.

The initial V2 attempt was interrupted without a final result. This completed run
restarted from case 1 using `ml/evals/triage/run_checkpointed.py`, which calls the
same request/scoring functions serially and saves each completed response. It
completed without further interruption or inference errors. All 120 unique IDs
and expected labels match the base/V1 reports, and all metrics were recomputed
from saved case results. No prompt, model, schema, or scoring changes were made
in response to intermediate results.

The QLoRA training manifest confirms the same base revision and seven recorded
package versions as V1. Both training sets have 240 records with the same language
counts. Training content and validation content differ; the comparison does not
isolate an individual annotation category as the cause of a change. V1's saved
manifest does not record warmup steps; V2 records six. Do not claim every effective
training setting was independently verified identical from the manifests alone.

## Overall results

| Metric | Base | V1 adapter | V2 adapter |
|---|---:|---:|---:|
| Schema validity | 100.0% | 100.0% | 100.0% |
| Critical-field accuracy | 85.2% | 92.7% | 91.5% |
| Exact match on five critical fields | 55.0% (66/120) | 77.5% (93/120) | 70.0% (84/120) |
| Raw human-review recall | 66.2% (45/68) | 82.4% (56/68) | 76.5% (52/68) |
| Raw human-review precision | 86.5% | 98.2% | 96.3% |
| Effective review recall after null/schema guard | 86.8% (59/68) | 82.4% (56/68) | 85.3% (58/68) |
| Effective missed escalations | 9 | 12 | 10 |
| Cases with unsupported facts | 16.7% (20/120) | 17.5% (21/120) | 16.7% (20/120) |
| Inference errors | 0 | 0 | 0 |
| Mean response latency | 15.45 s | 11.59 s | 11.33 s |
| P50 response latency | 14.12 s | 10.33 s | 10.41 s |
| P95 response latency | 22.35 s | 16.19 s | 15.81 s |

Latency is observational across separate CPU runs, not a controlled speed
benchmark. Exact match concerns location, headcount, asset, hazards and review
decision; it does not assert exact equality of confidence or uncertainty prose.
Unsupported-fact count is the existing evaluator's heuristic, not a complete
human hallucination audit. V2 has 16 raw review false negatives and two false
positives; the guard recovers six of those false negatives.

## V1 versus V2 by category

| Category | Cases | Exact match V1 -> V2 | Raw review recall V1 -> V2 | Effective recall V1 -> V2 |
|---|---:|---:|---:|---:|
| Clear | 20 | 100.0% -> 100.0% | N/A | N/A |
| Missing information | 20 | 85.0% -> 80.0% | 90.0% -> 90.0% | 90.0% -> 90.0% |
| Contradictory | 15 | 40.0% -> 60.0% | 53.3% -> 73.3% | 53.3% -> 73.3% |
| Hazard | 20 | 80.0% -> 55.0% | 90.9% -> 45.5% | 90.9% -> 72.7% |
| Malayalam | 15 | 60.0% -> 33.3% | 100.0% -> 71.4% | 100.0% -> 100.0% |
| Manglish | 15 | 80.0% -> 80.0% | 71.4% -> 71.4% | 71.4% -> 85.7% |
| Adversarial | 8 | 75.0% -> 50.0% | 100.0% -> 100.0% | 100.0% -> 100.0% |
| Irrelevant | 7 | 100.0% -> 100.0% | 100.0% -> 100.0% | 100.0% -> 100.0% |

V2's per-field accuracies are location 85.8%, headcount 95.8%, asset 97.5%,
hazards 93.3%, and review decision 85.0%.

## Effective V2 misses

Ten required-review cases still escape the null/schema guard:

- `v2-missing-information-011`
- `v2-missing-information-012`
- `v2-contradictory-005`
- `v2-contradictory-007`
- `v2-contradictory-008`
- `v2-contradictory-010`
- `v2-hazard-006`
- `v2-hazard-018`
- `v2-hazard-019`
- `v2-multilingual-manglish-008`

Relative to V1, V2 fixes effective misses `v2-contradictory-002`,
`v2-contradictory-011`, `v2-contradictory-015`, and
`v2-multilingual-manglish-007`, but introduces effective misses
`v2-hazard-006` and `v2-hazard-018`. Net improvement: two fewer effective misses.

Observed failure mechanisms include selecting one side of a count/location
conflict, inventing a definite road status, and retaining `nearby road` as though
it identified a road. All ten effective misses have concrete output fields and
`needs_human_review=false`, so structural checks alone cannot rescue them.

Malayalam failures include malformed/overextended location strings, an incorrect
headcount, and review=false despite missing hazard duration. Some string mismatches
are morphological or spelling failures rather than wholly invented locations, but
they remain failures under the unchanged lexical-preservation contract.

## Interpretation and next decision

The measured result supports a limited learning signal: revised synthetic data
improved the targeted contradiction behavior. It does not support replacing V1
with V2 overall, nor does it establish that increasing the same templates to 1,000
records will help. V2 training includes no hazard-missing-duration category; that
case exists only in its validation set. Its catalog also lacks training cases with
unknown hazard status. These are concrete coverage gaps worth auditing alongside
the heavy template repetition, not proven causal explanations of every regression.

Keep both adapters as experiment artifacts. Before more GPU training, compare
training coverage against the annotation policy, preserve useful V1 coverage,
and evaluate candidate changes on dev. Do not copy these exposed regression cases
into SFT data. A new independently reviewed blind holdout is needed before any
final generalization claim; current results are regression evidence only.

## Artifacts

- Completed raw result: `ml/runs/qwen3-4b-q4-k-m-lora-sft-v2-c1-prompt-v2.1.1-eval-v2.0.0.json`
- Per-case checkpoints: same basename with `.checkpoint.jsonl`
- Adapter: `ml/artifacts/qwen3-4b-triage-sft-v2/adapter-f16.gguf`
- Training receipt: `ml/reports/qwen3-4b-qlora-sft-v2-training-receipt.md`

Raw results and weights remain ignored; this report contains the reviewable conclusions.
