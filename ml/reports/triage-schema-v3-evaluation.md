# Triage schema V3: final evaluation

Completed 2026-10-26. **Neither candidate passes the predeclared gates.** The
SFT-V3 adapter is the stronger candidate overall, but 4 of the critical
under-triage cases are a gap in the urgency rules that no extraction model
can close, so the safety gate cannot be met without a rules change. The
story of how we got here, including every dev iteration, is in
`triage-schema-v3-experiment-log.md`.

## Setup

| | |
|---|---|
| Model | Qwen3-4B-Instruct-2507 Q4_K_M |
| Candidates | **A** base model; **B** base + SFT-V3 adapter (`adapter-f16.gguf`, sha256 `bcec4e21…`, trained on V2 outputs only) |
| Prompt | `prompts/triage-extraction-v3.2.txt` (frozen after dev) |
| Schema | `schemas/triage-extraction-v3.json`, `victim_profile` generated last |
| Serving | llama.cpp build 11429, CPU, 8 threads, 1 slot, context 4096, temperature 0, max 768 output tokens, JSON-schema constrained |
| Test set | `triage-eval-v3.0.1.jsonl`, 120 cases, frozen, owner-reviewed (56 gold CRITICAL) |
| V2 regression set | `triage-eval-v3-v2-carryover.jsonl`: the 120 frozen V2 inputs with V3 labels |
| Runner | `crisisgraph-triage-eval-v3`, serial, checkpointed; the adapter on the server is checked against `--adapter` |

The V2 baseline (98/120 exact, 60/68 review recall) was measured with the
SFT-V3 adapter, prompt v2.1.1, the V2 schema, llama.cpp build 10809 and a
2048 context, so the regression comparison changes prompt, schema and
serving settings together. Run 1 was interrupted at case 73 and resumed from
its checkpoint with an identical run identity (temperature 0).

## Gates

Fixed before any results were seen. Latency waived by the owner for local CPU
serving (2026-10-25); it is reported but does not count.

| Gate | A: base | B: SFT-V3 adapter |
|---|---|---|
| Critical under-triage <= 1 of 56 | ❌ 8 | ❌ 9 |
| Recall UNCONSCIOUS >= 90% | ✅ 13/13 | ✅ 13/13 |
| Recall BREATHING_DIFFICULTY >= 90% | ✅ 10/10 | ❌ 8/10 |
| Recall CARDIAC >= 90% | ✅ 10/10 | ✅ 10/10 |
| Recall BLEEDING >= 90% | ✅ 9/10 | ✅ 9/10 |
| Remote-caller miss <= 10% | ✅ 1/12 | ✅ 1/12 |
| V2 exact match >= 96/120 | ❌ 71 | ❌ 91 |
| V2 review recall >= 60/68 | ❌ 46/68 | ❌ 58/68 |
| p95 latency (waived) | 21.4 s | 21.8 s |
| **Passed** | **No** | **No** |

Recall intervals are wide at these counts (9/10 has a 95% Wilson interval of
about 0.60–0.98).

## Results

| Measure (test set unless noted) | A: base | B: adapter |
|---|---|---|
| Urgency matches gold | 91/120 | 89/120 |
| Under-triage, any tier | 9 | 11 |
| Over-triage | 11 | 12 |
| `suggested_urgency` raised a result | 2 | 0 |
| Reporter relation / on scene | 70% / 84% | 80% / 89% |
| Water level / trend | 90% / 99% | 91% / 99% |
| INJURY recall | 5/15 | 3/15 |
| ELDERLY / BEDRIDDEN recall | 14/21 · 2/2 | 14/21 · 0/2 |
| Cases with invented profile facts (test / V2 set) | 31 / 5 | 33 / 13 |
| V2 fields on the test messages, exact | 64/120 | 83/120 |
| V2 set: review false negatives / false positives | 22 / 0 | 10 / 2 |
| Latency p50 / p95 | 16.4 / 21.4 s | 17.9 / 21.8 s |

By language, critical under-triage was 6/36 English, 2–3/12 Manglish and 0/8
Malayalam for both candidates.

## Critical under-triage cases

| Case | Situation | A | B | Cause |
|---|---|---|---|---|
| 053 | snakebite | ✗ | ✗ | **Rules gap**: `other_conditions` reach only URGENT |
| 055 | person swept away, holding a tree | ✗ | ✗ | **Rules gap** |
| 057 | hypothermia with confusion | ✓ (raised) | ✗ | **Rules gap**; A's suggestion rescued it |
| 085 | family on a roof, no flood stated | ✗ | ✗ | **Rules gap** (strict roof rule) |
| 028 | "chora nilkunnilla", remote caller (Manglish) | ✗ | ✗ | Bleeding not extracted |
| 067 | "ground floor completely under water" | ✗ | ✗ | ABOVE_HEAD not extracted |
| 068 | "veedu muzhuvan vellathil", on the terrace | ✗ | ✗ | ROOF not extracted |
| 065 | elderly, waist level, "koodikondirikkunnu" | ✓ | ✗ | Rising not extracted (B) |
| 070 | bedridden father, chest-deep water | ✓ | ✗ | "bedridden" written into `other_conditions` (B) |
| 064 | "3 to 4 feet", rising | ✗ | ✓ | Range read as WAIST, not CHEST (A) |
| 076 | "about 2 metres" | ✗ | ✓ | Read as WAIST, not ABOVE_HEAD (A) |

The **oracle** (gold labels through the rules) under-triages 053, 055, 057
and 085: with perfect extraction, critical under-triage would still be 4,
above the gate of 1. The raise-only `suggested_urgency` was meant to cover
these; it raised a result 2 times in 120 cases for A (rescuing 057) and never
for B.

## Findings

1. **The urgency rules have a hole.** Life-threatening situations outside the
   categories (snakebite, drowning risk, hypothermia, stranded on a roof) can
   reach only URGENT, and the model's raise-only suggestion almost never
   fires. This is a rules decision, independent of the model.
2. **Water described by the building, not the body, is missed by both
   models** ("ground floor under water", "the whole house is under water"),
   although the prompt names it. This is a training-data gap.
3. **The adapter keeps most of its V2 strength but not all of it.** Asking for
   an output it never learned cost 7 exact matches and 2 missed reviews on
   the V2 set. The base model is far worse on V2 (71/120, 22 missed reviews),
   which rules it out for dispatch regardless of its profile results.
4. **The base model is slightly better on the profile** (BREATHING 10/10,
   BEDRIDDEN 2/2, two useful suggestions), the adapter better on the
   reporter fields and on everything V2. Neither is reliable on INJURY next
   to a more serious condition (3–5/15).
5. **Invented facts are common** (about a quarter of test cases), mostly the
   reporter relation and CHILD/ELDERLY from family words, and on the V2 set
   "critical patient" became UNCONSCIOUS or CARDIAC.
6. **Generation order matters**: placing `victim_profile` after the review
   decision was the largest single improvement during development.

## Limitations

- Synthetic data, owner-reviewed, not certified by a medical or
  emergency-services expert.
- Some scenario types appear in both the dev and test sets with different
  wording and places; the prompt was tuned on dev for five rounds.
- Small counts per condition; differences of one or two cases are within noise.
- Latency is CPU-only; the deployment latency budget belongs to GPU serving.
- The V2 baseline used different serving settings and llama.cpp build.

## Decision options (owner)

1. **Rules change first, for either model.** For example, make any
   `other_conditions` entry CRITICAL-for-review (over-triage, but a human
   sees it at once), or add categories for the recurring life threats
   (envenomation, in water / drowning risk, hypothermia, stranded at height).
   Needs a rules version (`urgency-rules-v2`) and re-scoring; the existing
   run outputs can be re-scored without new model runs.
2. **SFT-V4 from the measured failures**, starting from the untouched base
   model, with V3 targets for every field: building-referenced water, ranges
   and metres, Malayalam/Manglish medical phrasing, groups only when stated,
   injury next to serious conditions, and V2 review decisions. Evaluate on a
   **new blind test set**, since v3.0.1 has now been seen.
3. **Integrate the adapter (B) into M3b now as an experimental candidate**,
   with the measured limits documented and the deterministic boundary
   failing closed, as was done for V2. Not recommended until option 1 is
   decided, because the under-triage gap is structural.

Recommended: **1, then 2.** Option 1 is cheap and removes a floor that no
model can get below; option 2 targets the specific extraction failures
measured here.
