# Prompt V2.1.1 cross-model comparison

Date: 2026-09-11

## Hypothesis

Prompt V2.1 had strong extraction behavior but poor raw human-review recall. A
small, targeted rewrite of only its review rule may recover review behavior
without the additional instructions and examples in V2.2 overwhelming a 4B
model.

## Prompt change

V2.1.1 preserves V2.1's structure and wording except for its human-review
paragraph. The replacement expresses review as a mandatory computation over
null, ambiguous, contradictory, and incomplete-hazard fields. It also states
that confidence cannot override the result and ties uncertainty reasons to the
computed boolean.

| Prompt | Words | Bytes |
|---|---:|---:|
| V2.1 | 314 | 2,078 |
| V2.1.1 | 348 | 2,303 |
| V2.2 | 451 | 3,722 |

V2.1.1 is 10.8% longer than V2.1 by word count and 22.8% shorter than V2.2. It
adds no headings or examples.

## Fixed conditions

- Runtime: llama.cpp build 10809
- Quantization: Q4_K_M
- CPU threads: 8
- Context per slot: 2,048 tokens
- Concurrency: 1
- Schema: `triage-extraction-v2`
- Dataset: `triage-eval-v1.jsonl` (12 synthetic development cases)
- Temperature: 0.0
- Maximum output: 512 tokens

These cases influenced prompt design. This is a development comparison, not an
unbiased test of generalization or production reliability.

## Qwen prompt comparison

| Metric | V2.1 | V2.1.1 | V2.2 |
|---|---:|---:|---:|
| Schema validity | 100.0% | 100.0% | 100.0% |
| Critical-field accuracy | 88.3% | 88.3% | **90.0%** |
| Critical exact match | 58.3% | **66.7%** | 50.0% |
| Human-review recall | 42.9% | 85.7% | **100.0%** |
| Raw review-decision accuracy | 66.7% | **83.3%** | **83.3%** |
| Cases with unsupported facts | 8.3% | **0.0%** | 16.7% |
| Mean service latency | 17.4 s | 17.8 s | 13.8 s |

V2.1.1 fixed three of V2.1's four review false negatives. It still returned
`needs_human_review=false` when `required_asset` was null in
`missing-asset-001`. It also rejected all valid facts in the prompt-injection
case and escalated it, which is fail-closed but loses useful extraction.

## Gemma prompt comparison

| Metric | V2.1.1 | V2.2 |
|---|---:|---:|
| Schema validity | 100.0% | 100.0% |
| Critical-field accuracy | 75.0% | **83.3%** |
| Critical exact match | 16.7% | **50.0%** |
| Human-review recall | 100.0% | 100.0% |
| Raw review-decision accuracy | 66.7% | **75.0%** |
| Cases with unsupported facts | **33.3%** | 41.7% |
| Mean service latency | **24.8 s** | 28.9 s |

Gemma followed the fail-closed review rule but over-requested review on four
complete cases. Removing V2.2's examples also caused substantial asset
classification regressions: Gemma frequently selected `AMBULANCE` or
`EVAC_TRUCK` when another asset was requested. The shorter prompt reduced
observed latency but did not preserve semantic quality.

## Conclusion

The hypothesis is partially supported for Qwen and rejected for Gemma. A
compact, precise review rule is better than V2.1 for Qwen, but it still cannot
guarantee raw review recall. Gemma benefits from the additional scaffolding in
V2.2, especially its examples, despite the larger prompt.

Prompt V2.1.1 is the best Qwen development prompt for exact match and avoidance
of counted unsupported facts. Prompt V2.2 remains the best raw fail-closed Qwen
prompt because it reached 100% review recall. Neither result changes the system
requirement: deterministic application logic must force review for null
critical fields, incomplete hazards, invalid output, or inference failure.

Do not add another rule for the single remaining Qwen case using this same
12-case set. The next reliable step is an untouched evaluation split, followed
by reporting both raw model review recall and effective post-policy recall.
