# Qwen3-4B QLoRA V3: frozen regression comparison

Completed 2026-09-22. V3 is the strongest adapter measured on the exposed
120-case regression suite, but it is **not promoted unconditionally** because it
fails the predeclared 20/20 clear-case gate. It improves aggregate extraction and
review recall while introducing two false escalations on clear requests. A blind
holdout is still required before any generalization or deployment claim.

## Controlled execution

All adapter results use the same frozen cases and labels, prompt V2.1.1, JSON
schema, Qwen3-4B-Instruct-2507 Q4_K_M base, llama.cpp build 10809 at commit
`5266f24da7`, eight threads, a 2,048-token context, one slot, temperature zero,
and a 512-token output limit. V3 used its separately converted 504-tensor F16
adapter at default scale. The checkpointed run completed all 120 cases with no
inference errors. Exact hashes and execution settings are recorded in
`qwen3-4b-qlora-sft-v3-evaluation-config.json`.

V3 starts from the untouched base model and combines the 240 V1 training records
with 60 targeted records, for 300 total. Its two epochs therefore create 25%
more example exposures than V1. This is a controlled system comparison, not a
strict one-variable or data-only experiment.

## Overall results

| Metric | Base | V1 | V2 | V3 |
|---|---:|---:|---:|---:|
| Schema validity | 100.0% | 100.0% | 100.0% | 100.0% |
| Critical-field accuracy | 85.2% | 92.7% | 91.5% | **94.0%** |
| Exact match on five critical fields | 55.0% (66/120) | 77.5% (93/120) | 70.0% (84/120) | **81.7% (98/120)** |
| Raw human-review recall | 66.2% (45/68) | 82.4% (56/68) | 76.5% (52/68) | **88.2% (60/68)** |
| Raw human-review precision | 86.5% | **98.2%** | 96.3% | 95.2% |
| Effective recall after null/schema guard | 86.8% (59/68) | 82.4% (56/68) | 85.3% (58/68) | **88.2% (60/68)** |
| Cases with unsupported facts | 16.7% (20/120) | 17.5% (21/120) | 16.7% (20/120) | **14.2% (17/120)** |
| Inference errors | 0 | 0 | 0 | 0 |
| Mean latency | 15.45 s | 11.59 s | 11.33 s | 11.59 s |
| P50 latency | 14.12 s | 10.33 s | 10.41 s | 10.31 s |
| P95 latency | 22.35 s | 16.19 s | 15.81 s | 15.32 s |

Latency was observed across separate CPU runs and is not a controlled throughput
benchmark. Exact match covers victim location, headcount, requested asset,
hazards, and the review decision. The unsupported-fact metric is the existing
evaluator heuristic, not a complete hallucination audit.

## V1 versus V3 by category

| Category | Cases | Exact match V1 -> V3 | Review recall V1 -> V3 |
|---|---:|---:|---:|
| Clear | 20 | **100.0% -> 90.0%** | N/A |
| Missing information | 20 | 85.0% -> 80.0% | 90.0% -> 90.0% |
| Contradictory | 15 | **40.0% -> 73.3%** | **53.3% -> 80.0%** |
| Hazard | 20 | 80.0% -> 80.0% | 90.9% -> 90.9% |
| Malayalam | 15 | **60.0% -> 73.3%** | 100.0% -> 100.0% |
| Manglish | 15 | 80.0% -> 80.0% | 71.4% -> 71.4% |
| Adversarial | 8 | **75.0% -> 87.5%** | 100.0% -> 100.0% |
| Irrelevant | 7 | 100.0% -> 100.0% | 100.0% -> 100.0% |

V3's field accuracies are location 90.0%, headcount 97.5%, asset 95.8%, hazards
95.8%, and review decision 90.8%.

## Review-decision audit

V3 has eight review false negatives:

- `v2-missing-information-011`
- `v2-missing-information-012`
- `v2-contradictory-005`
- `v2-contradictory-007`
- `v2-contradictory-008`
- `v2-hazard-019`
- `v2-multilingual-manglish-007`
- `v2-multilingual-manglish-008`

The model fills an omitted asset or location, selects one side of a contradiction,
or assigns a definite hazard status. Those outputs are structurally complete, so
the deterministic null/schema guard cannot recover any of them.

V3 also has three review false positives:

- `v2-clear-002`: drops the explicitly requested rescue boat.
- `v2-clear-007`: drops the explicitly requested evacuation truck.
- `v2-adversarial-005`: converts non-incident flooding text into an incomplete
  hazard and escalates it.

Relative to V1, V3 repairs four review false negatives, all contradictions, and
introduces no new review false negatives. Across full exact match it fixes ten
V1 failures and regresses five, giving the net improvement from 93 to 98 cases.

## Predeclared decision gates

| Gate | V3 result | Decision |
|---|---:|---|
| Schema validity and inference errors | 100%; 0 errors | Pass |
| Raw and effective recall above V1's 56/68 | 60/68; 60/68 | Pass |
| Hazard review recall at least 10/11 | 10/11 | Pass |
| Clear exact match 20/20 | 18/20 | **Fail** |
| Malayalam and Manglish exact floors | 11/15; 12/15 | Pass |
| Overall exact match at least 93/120 | 98/120 | Pass |
| Unsupported facts no more than 21/120 | 17/120 | Pass |
| Contradiction recall above 8/15 | 12/15 | Pass |

The documented 99% review-recall target effectively requires 68/68 on this
suite. V3 reaches 60/68, so it does not meet that target.

## Decision and next experiment

V3 demonstrates that targeted augmentation can repair contradiction abstention
without repeating V2's hazard and Malayalam regressions. It is therefore a useful
candidate, but the clear-case regression means the predeclared rule does not
permit proceeding directly to a blind promotion test as though every gate passed.

The completed audit is in `qwen3-4b-qlora-sft-v3-false-positive-audit.md`. It
found a narrow counterexample imbalance and a fake-hazard injection coverage
gap, without claiming either as a proven causal explanation. A separate 12-case
development pilot is now pending semantic/language review. Do not train on the
exposed evaluation cases or rewrite their labels. If a later adapter restores
the clear gate to 20/20 while preserving V3's gains, freeze a new independently
reviewed blind holdout. Until then, retain V1 as the conservative comparison
baseline and treat V3 as the leading experimental candidate.

## Ignored artifacts

- Raw result: `ml/runs/qwen3-4b-q4-k-m-lora-sft-v3-c1-prompt-v2.1.1-eval-v2.0.0.json`
- Per-case checkpoint and identity: same basename with `.checkpoint.jsonl` and
  `.identity.json`
- Imported PEFT adapter: `ml/artifacts/qwen3-4b-triage-sft-v3/adapter/`
- Converted adapter: `ml/artifacts/qwen3-4b-triage-sft-v3/adapter-f16.gguf`

Weights and raw run files remain ignored; this report contains the tracked,
reviewable conclusion.
