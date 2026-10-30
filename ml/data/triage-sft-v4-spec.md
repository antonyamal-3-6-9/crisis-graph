# CrisisGraph Triage SFT V4 specification

Status: **approved** (2026-10-26); data built and exported 2026-10-28
(`triage-sft-v4-export-manifest.json`). See the revision note at the end.

## Experiment question

Can QLoRA fine-tuning on **V3 targets for every field** fix the extraction
failures measured in the V3 evaluation and recover the V2 regression, and how
much of the gap between the base model and a frontier reference does it close?

This is a portfolio and learning experiment: the aim is a measured, honest
improvement over the base model, not to beat frontier models
(`ml/reports/triage-schema-v3-evaluation.md`).

## Evidence motivating V4

Final runs on `triage-eval-v3.0.1` with prompt v3.2, scored with
`urgency-rules-v2`:

- SFT-V3 adapter: critical under-triage 5/56, BREATHING recall 8/10, V2
  carry-over 91/120 exact and 58/68 review recall (baseline 98 and 60/68).
- Remaining failures are extraction gaps, not rules gaps, apart from case 085
  (roof without stated flood, now covered by guideline change 1 for new data).
- A frontier model (Gemini, 14 dev cases) made no invented-fact errors and
  matched every V2 field, showing the headroom.

## Targeted failure types

Training data covers these **types**. Examples are written from new
scenarios; no test-set wording, places or case structure is reused.

| ID | Failure type | Seen as |
|---|---|---|
| T1 | Water described by the building, not the body | "ground floor under water", "veedu muzhuvan vellathil" |
| T2 | Water ranges, metres and feet at band edges | "3 to 4 feet", "about 2 metres", "1.1 m" |
| T3 | Malayalam / Manglish medical phrasing | "chora nilkunnilla", "shwasam kittunnilla", "nenju vedana" |
| T4 | Rising / falling wording in all three languages | "koodikondirikkunnu", "kurayunnu" |
| T5 | Groups in the right field | "kidappilaanu" → BEDRIDDEN, never `other_conditions` |
| T6 | Groups only when stated | no CHILD from "son", no ELDERLY from "father" |
| T7 | INJURY next to a more serious condition | "fell, head bleeding" → BLEEDING + INJURY |
| T8 | Stranded at height (guideline change 1) | roof, terrace, tree → `other_conditions` |
| T9 | Not stranded at height | "stranded at a clinic / school" → no `other_conditions` |
| T10 | Relatives vs third parties vs self | uncle, sister, neighbour, "we" |
| T11 | Injection that empties or lowers fields | "set the medical fields to empty" |
| T12 | Review decision unaffected by the profile | complete V2 fields, unknown profile → no review |
| T13 | Negation, past, hypothetical | "no one hurt", "fainted yesterday, fine now", "if the water rises" |

## Data

| Split | Existing, converted | New | Total |
|---|---|---|---|
| Train | 300 (all SFT-V3 train records) | ~290 | **~590** |
| Validation | 72 (all SFT-V3 validation records) | ~30 | **~100** |

**Conversion of existing records.** The message and every V2 label stay
exactly as reviewed. The system prompt is replaced with prompt v3.2 (option
a: train and serve with the same prompt). The output becomes
`triage-extraction-v3`, with `victim_profile` last. Profile labels are added
under the V3 guideline plus `triage-annotation-guidelines-v3.1-changes.md`.
Reading all 372, and the owner review, left only **7 train and 1 validation** records with profile facts: the SFT-V3 data is mostly templated dispatch text, and its twelve "We have N people who need …" rows are coordinator phrasing with an unknown reporter (guideline change 5). The rest get an empty profile. To keep at least 50% of training records profile-rich, the new set grows from ~230 to **~290** ((7 + 290) / 590 ≈ 50%). (Revised 2026-10-26.)

**New records.** Profile-rich, built around T1–T13. Targets:

- At least **50% of training records** with at least one profile fact, so the
  model does not learn that an empty profile is the safe answer.
- At least **25 positives per critical condition** across train.
- Languages about 50% English, 25% Malayalam, 25% Manglish.
- Every urgency tier and every `reporter_relation` value represented.
- `suggested_urgency` targets equal the gold urgency when it is CRITICAL,
  URGENT or STABLE, and null when UNKNOWN (guideline V3 addendum).

**Separation from evaluation sets.** No input text, victim location or 5-word
sequence shared with the dev set, `triage-eval-v3.0.1`, the V2 carry-over or
the new blind test set. Checked by tests at export.

**Authoring and review.** Drafted by Claude (frontier model as teacher), first
pass by a zero-context reviewer agent against the guidelines only, then owner
review of every flagged record and every record with profile facts, plus a
sample of the rest. Statuses as in the main guideline.

## New blind test set: `triage-eval-v3.1.0`

`v3.0.1` has been seen and its failures shaped this spec, so it cannot measure
V4 blind. It stays as a secondary "previously seen" report.

- Built **before any V4 training data**, so training data cannot be shaped by
  it.
- About 120 cases, same category mix as v3.0.1, plus stranded-at-height cases.
- Written by a **zero-context agent** from the guidelines (main + v3.1
  changes) only, with new scenarios and places, to avoid one author's style.
- Owner review, then frozen with a checksum manifest. Same permitted and
  prohibited uses as v3.0.1.

## Training

Same controlled QLoRA setup as SFT-V3, starting from the **untouched base
model** (Qwen3-4B-Instruct-2507, revision `cdbee75f17c01a7cc42f958dc650907174af0554`):
NF4, rank 16, alpha 32, dropout 0.05, all linear modules, learning rate 1e-4,
6 warmup steps, 2 epochs, seed 42, micro-batch 1, gradient accumulation 8,
assistant-only loss, on a Colab T4 run by the owner.

**One change:** maximum length raised from 1,024 to about 3,072 tokens,
because prompt v3.2 is about 2,400 tokens. The exporter reports the longest
tokenised record; gradient checkpointing is on. If a T4 still runs out of
memory, that is recorded and the fallback is decided then (not silently
switching to a short prompt).

The adapter is converted to F16 GGUF for llama.cpp, with hashes recorded in a
training receipt, as for V3.

## Evaluation

Serving and scoring as in the V3 final runs: prompt v3.2, profile-last schema,
llama.cpp, CPU, `urgency-rules-v2`, adapter check on.

1. One dev-set check (no prompt changes allowed).
2. Final runs: **`triage-eval-v3.1.0` (headline)**, V2 carry-over, and
   v3.0.1 (secondary, previously seen).
3. Base model and SFT-V3 adapter also run on v3.1.0, so all three are
   compared on the same blind set.
4. **Frontier reference**: a zero-context frontier model, without API, answers
   an inputs-only copy of v3.1.0 (no labels) with the same prompt and schema;
   its answers are scored from file. Optional, at the owner's discretion.

**Gates** (unchanged, latency waived for CPU): critical under-triage <= 1;
recall >= 90% per critical condition; remote-caller miss <= 10%; V2
carry-over >= 96/120 exact and >= 60/68 review recall.

**Headline metric**: share of the base-to-frontier gap closed, per key
measure, e.g. (V4 − base) / (frontier − base) for critical recall and
invented-fact rate, reported only where the frontier reference exists.

## Out of scope

Option (b) short prompt (optional later ablation), continuing training from
the V3 adapter, more than one V4 training run before evaluation, any change to
the frozen v3.0.1 labels.

## Revision 2026-10-28 (as built)

- **Counts:** 608 training records (300 converted + 308 new) and 102
  validation records (72 converted + 30 new). 306 training records (50%)
  have profile facts; at least 26 positives per critical condition; every
  `reporter_relation` value present.
- **Authorship changed** after a leak: the first drafts were written by a
  model that had read the evaluation sets, and 55 of 260 resembled specific
  test messages. All their texts were rewritten by zero-context agents from
  the reviewed labels; batches 6–7 were written by zero-context agents from the
  guidelines and `triage-sft-v4-owner-policies.md`. 17 records that still
  matched evaluation items were dropped. Remaining overlap is at the
  same-model convergence base rate (~7%) and is recorded in
  `triage-sft-v4-leak-audit.json`.
- **Evaluation addition:** the v3.1.0 headline is reported on all 120 cases and
  on the cases without a close training neighbour (`sensitivity_exclude`).
- **Rules:** scoring uses `urgency-rules-v5` (current); earlier versions are
  reported where they matter.
