# Triage prompt V2.2 comparison

Date: 2026-09-11

## Experiment

V2.2 tests whether a small instruction-tuned model benefits from a more explicit
decision procedure. Compared with V2.1, it adds:

- a narrowly defined extractor role and explicit non-authorities;
- an ordered field-by-field extraction procedure;
- field-specific null and hallucination rules;
- a mandatory decision table for `needs_human_review`;
- cross-field output invariants; and
- three short examples covering complete input, a missing asset, and an
  incomplete hazard.

## Fixed evaluation conditions

- Model: `Qwen/Qwen3-4B-Instruct-2507`
- Quantization: GGUF Q4_K_M
- Runtime: llama.cpp build 10809
- CPU threads: 8
- Context per slot: 2,048 tokens
- Concurrency: 1
- Schema: `triage-extraction-v2`
- Dataset: `triage-eval-v1.jsonl` (12 synthetic development cases)

This dataset has now influenced the prompt, so it is a development set. Results
measure iteration progress, not generalization or production reliability.

## Results

| Metric | Prompt V2.0 | Prompt V2.1 | Prompt V2.2 |
|---|---:|---:|---:|
| Schema validity | 100.0% | 100.0% | 100.0% |
| Critical-field accuracy | 76.7% | 88.3% | 90.0% |
| Critical exact match | 41.7% | 58.3% | 50.0% |
| Human-review recall | 100.0% | 42.9% | 100.0% |
| Cases with unsupported facts | 33.3% | 8.3% | 16.7% |
| Mean service latency | 27.8 s | 17.4 s | 13.8 s |
| P50 service latency | 26.3 s | 17.0 s | 13.7 s |
| P95 service latency | 36.6 s | 20.8 s | 19.9 s |

V2.2 field accuracy was 91.7% for location, headcount, asset, and hazards,
and 83.3% for the raw model review decision.

## Interpretation

The mandatory review decision table fixed V2.1's safety-critical false
negatives: all seven cases expected to require review were flagged. V2.2 also
achieved the highest aggregate critical-field accuracy of the three prompts.

The improvement has trade-offs. The model became over-cautious and incorrectly
requested review for two complete reports. It also inferred an ambulance from
the generic phrase "We need help" and selected one value from a contradictory
headcount. Those are unsupported facts even though the model still requested
human review. Exact match consequently fell below V2.1.

The lower latency must not be attributed to the prompt. V2.2 is longer than the
other prompts, and these runs were not randomized repeated performance trials.
Warm model state, prompt-prefix caching, output length, and normal CPU variance
can all change latency. Accuracy and latency experiments should be separated.

## Decision

V2.2 is the best development prompt so far for fail-closed behavior, but it is
not sufficient as the only safety mechanism. The Rust boundary must still
derive an effective review decision deterministically from null fields,
incomplete hazards, schema failures, and model errors. A model can add a review
requirement; it cannot remove one.

Before further prompt tuning, create a larger untouched test split. Otherwise,
each rule added for these 12 known cases increases the risk of overfitting the
evaluation rather than improving real SOS extraction.
