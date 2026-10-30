# Qwen3-4B SFT-V4 evaluation

Date: 2026-10-30. Spec: `data/triage-sft-v4-spec.md` (revision 2026-10-28).
Training: `qwen3-4b-qlora-sft-v4-training-receipt.md`.

## Setup

All runs use the same serving settings as the V3 final runs: llama.cpp build
11429 on CPU, Qwen3-4B-Instruct-2507 Q4_K_M, context 4,096, one slot,
temperature 0, at most 768 tokens, prompt v3.2, profile-last schema, and the
server adapter check on. Every result is scored with `urgency-rules-v5`
(saved outputs re-scored; no model call depends on the rules). The models are:

- **Base**: no adapter.
- **SFT-V3**: `adapter-f16.gguf` `bcec4e21…` (V2 targets, prompt v2.1.1 in training).
- **SFT-V4**: `adapter-f16.gguf` `f31100c9…` (V3 targets, prompt v3.2 in training).

Runs: dev (42, one check, no prompt changes), **`triage-eval-v3.1.0` (headline,
blind, 120)**, the V2 carry-over (120), and `triage-eval-v3.0.1` (120,
previously seen). Base and SFT-V3 carry-over and v3.0.1 outputs are the saved
V3 final runs; everything else was run for this report. Raw outputs are in
`ml/runs/` (not committed).

The v3.1.0 headline is also reported on the **92 cases without a close
training neighbour**: the 28 test cases that the leak audit found resembling
a training record (kept as medium overlap, or dropped) are excluded
(`data/triage-sft-v4-leak-audit.json`, `sensitivity_exclude`).

## Headline: v3.1.0 blind test

| Measure | Base | SFT-V3 | **SFT-V4** | Gate |
|---|---:|---:|---:|---|
| Usable outputs | 120 | 120 | 120 | — |
| Critical under-triage (of 57 CRITICAL) | 12 | 8 | **7** | ≤ 1 ❌ all |
| UNCONSCIOUS recall | 62% | 69% | **77%** | ≥ 90% ❌ all |
| BREATHING_DIFFICULTY recall | 100% | 91% | 91% | ≥ 90% |
| CARDIAC / BLEEDING recall | 100 / 100% | 100 / 100% | 100 / 100% | ≥ 90% |
| Remote-caller miss | 7.7% | **0%** | 7.7% | ≤ 10% ✅ all |
| Urgency accuracy (4 tiers) | 64% | 66% | **70%** | — |
| Over-triage | 20% | 22% | **17%** | — |
| V2 fields exact | 40 | 56 | **81** | — |
| V2 review recall | 0.89 | **0.94** | 0.91 | — |
| Cases with invented facts | 39 | 40 | **31** | — |
| Latency p50 (CPU, waived) | 20.1 s | 23.3 s | 18.2 s | — |

**Sensitivity check (92 cases without a close training neighbour):**
critical under-triage base 8, SFT-V3 5, SFT-V4 5; UNCONSCIOUS recall 75% /
88% / 88%; V2 exact 32 / 44 / 63; invented-fact cases 27 / 30 / 22. The
ordering is the same and SFT-V4 does not gain more on the cases that have a
training neighbour, so the measured overlap did not inflate the result.

**By language (V2 exact / critical under-triage):**

| | Base | SFT-V3 | SFT-V4 |
|---|---|---|---|
| English (58) | 27 / 0 | 33 / 0 | 40 / 1 |
| Malayalam (32) | 9 / 5 | 14 / 3 | **24 / 1** |
| Manglish (30) | 4 / 7 | 9 / 5 | 17 / **5** |

## V2 carry-over and v3.0.1

| Set | Measure | Base | SFT-V3 | **SFT-V4** | Gate |
|---|---|---:|---:|---:|---|
| Carry-over (120) | V2 exact | 71 | 91 ❌ | **106** ✅ | ≥ 96 |
| | Review recall | 46/68 | 58/68 ❌ | **61/68** ✅ | ≥ 60/68 |
| | Invented-fact cases | 5 | 13 | **2** | — |
| v3.0.1 (120, seen) | Critical under-triage | 7 | 5 | **3** | ≤ 1 ❌ |
| | V2 exact | 64 | 83 | **108** | — |
| | Review recall | 0.94 | 0.99 | **1.00** | — |
| | Remote-caller miss | 8.3% | 8.3% | **0%** | ≤ 10% |
| | Invented-fact cases | 31 | 33 | **21** | — |
| Dev (42) | Critical under-triage | — | 0 | 0 | — |
| | Invented-fact cases | — | 15 | **8** | — |

SFT-V4 **recovers the V2 regression**: it passes the carry-over gate that
SFT-V3 failed, and exceeds the SFT-V2 figures the gate was set from (98 exact,
60/68).

## Where SFT-V4 still fails

Six of SFT-V4's seven critical under-triage cases on v3.1.0 are also missed by
SFT-V3 (011, 023, 044, 049, 060, 107), and five are Manglish. Each is a
single missed extraction, not a rules problem:

| Case | Lang | Missed fact |
|---|---|---|
| 011 | manglish | UNCONSCIOUS read as INJURY only |
| 023 | manglish | UNCONSCIOUS missed (bedridden elderly) |
| 029 | en | BREATHING_DIFFICULTY missed |
| 044 | ml | snakebite (`other_conditions`) missed |
| 049 | manglish | "swept away by the current" missed |
| 060 | manglish | chest-deep water missed |
| 107 | manglish | terrace stranding and roof-level water missed |

## Gap closed relative to the base model

No frontier reference was run on v3.1.0 (optional in the spec; the Gemini dev
check was not repeated, billing being out of scope). The gap is therefore
measured towards a perfect score: share of the base model's error removed by
SFT-V4.

| Measure | Base | SFT-V4 | Perfect | Gap closed |
|---|---:|---:|---:|---:|
| Critical under-triage | 12 | 7 | 0 | 42% |
| UNCONSCIOUS recall | 62% | 77% | 100% | 40% |
| V2 exact (v3.1.0) | 40 | 81 | 120 | 51% |
| Invented-fact cases | 39 | 31 | 0 | 21% |
| Carry-over V2 exact | 71 | 106 | 120 | 71% |

## Conclusion

SFT-V4 is the best local model on every dispatch-field measure, invents fewer
facts, and closes the V2 regression; on the blind set it also has the lowest
critical under-triage and the highest urgency accuracy. It **does not pass
the critical-triage gates** (7 under-triaged cases against ≤ 1; UNCONSCIOUS
recall 77% against 90%), and neither does any other model tested. The
remaining misses are concentrated in Manglish medical and situation phrasing
and are shared with SFT-V3, which points to a limit of the 4B base with this
amount of data rather than to an SFT-V4 regression.

Consequences for the system: SFT-V4 is the recommended local adapter for
development and M3b integration, but the extraction must stay a candidate,
not an authority, for critical triage. The deterministic rules, the
`UNKNOWN_REVIEW_REQUIRED` path and dispatcher review stay as designed. A
missed critical fact reaches a person, it is not silently dispatched as low
priority. That decision belongs to the owner.

Limitations: 57 CRITICAL cases and 13 UNCONSCIOUS cases give wide intervals
(one case moves UNCONSCIOUS recall by about 8 points); the training and test
texts were written by models of the same family from the same guidelines
(about 7% close matches at the convergence base rate, handled by the
sensitivity check); latency is CPU-only and waived.

Possible next steps, not started: a targeted Manglish batch for the shared
failure phrasings (written zero-context, then a new blind set, since v3.1.0
has now been seen); a larger base (Qwen3-8B) on the same data; and the
optional frontier reference on an inputs-only copy of v3.1.0.
