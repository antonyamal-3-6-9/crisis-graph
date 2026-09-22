# SFT V2 train and validation candidate review

Status: revision 2; all 288 candidates are `first_pass_reviewed` after the supplied
AI-assisted reviews and verification of their exact mechanical wording corrections.
Training originally had 218 acceptances and 22 wording holds; validation had 47
acceptances and one wording hold. All targets are unchanged. Correction closure
was performed by the coding assistant, not a new independent semantic reviewer.
Train and validation have now been exported under the explicit `pilot-first-pass`
policy for the controlled V2 experiment; see `triage-sft-v2-export-manifest.json`.
No V2 training has occurred. The 48 dev records remain in their two
reviewed batches, with first-pass acceptance and independent acceptance pending.

## Files and counts

| Split | File | English | Malayalam | Manglish | Total | Scenario families |
|---|---|---:|---:|---:|---:|---:|
| Train | `triage-sft-v2-train-candidates.jsonl` | 144 | 48 | 48 | 240 | 16 |
| Validation | `triage-sft-v2-validation-candidates.jsonl` | 24 | 12 | 12 | 48 | 12 |
| Dev (already authored) | two `triage-sft-v2-dev-review-batch-*.jsonl` files | 16 | 16 | 16 | 48 | 16 |

Train and validation language counts match V1. Dev now has equal language counts,
unlike V1's 24/12/12 distribution; do not compare aggregate dev scores as though
the datasets were identical. The frozen 120-case regression suite remains unchanged.

Training has 15 variants per family: nine English and three each Malayalam and
Manglish. Validation has four variants per family: two English and one each
Malayalam and Manglish. `variant` is a within-family/language slot-variation index.

These are authored synthetic templates with deterministic substitutions of places,
counts, assets, and durations. They are NOT real reports, 240 independent training
scenarios, or a blind test of generalization. Repeated wording remains a limitation.
The experiment changes coverage and wording at a fixed record budget; a positive
result cannot isolate which individual data change caused the improvement.

## Coverage and separation

Training includes complete requests, an irrelevant-number distractor, two kinds
of headcount conflict, location ambiguity, two asset-conflict scenarios, uncertain
counts, separately missing asset/count/location, complete and unanchored hazards,
two injection scenarios (including suppressing review when count is missing), and
irrelevant vehicle-catalogue text.

Validation uses different situations: roll-call completion, conflicting form
fields, an uncertain venue in a voice note, incompatible selection boxes, a lower
bound on group size, a vehicle availability question, an interrupted count,
an empty address field, a hazard bulletin, unknown hazard age, an appended fake
assistant answer, and a classroom vocabulary exercise.

No reviewed dev template was used to generate these files. Family identifiers do
not include split names; each family, its language variants, and its declared
templates are restricted to one split. Checks also reject exact overlap with all
V1 candidate splits and the frozen evaluation. These checks cannot prove semantic
independence or discover every paraphrase: inspect cross-split similarity during
review. Shared annotation categories across splits are intentional.

## Review order

1. Review all 240 training records in four consecutive 60-record phases in one
   session, continuing automatically without confirmation between phases.
   The file is grouped by
   scenario family, then language; each 60-record batch contains four full families.
2. Review all 48 validation records separately, ideally with a reviewer who did
   not author the training examples. Do not merge validation records into training.
3. Apply any specific wording/target corrections to the catalog and candidates,
   preserving IDs and review history. Do not overwrite reviewed artifacts by rerunning
   generation. Revalidate changes and record new checksums and revision history.
4. Only mark `first_pass_reviewed` after actual acceptance. Independent acceptance
   is still required for `reviewed`. Any pilot-first-pass export exception must be
   explicitly recorded. The current files qualify only for an explicitly selected
   pilot-first-pass exception; default independent export must still refuse them.

Review Malayalam/Manglish fluency carefully. Location names are fictional; Malayalam
templates use explicit named-place constructions to avoid mechanical suffix errors.
Confidence labels follow `triage-annotation-guidelines.md`: 0.95 complete, 0.55
relevant-but-review-required, 0.10 irrelevant. They are not calibrated probabilities.

## Reviewer prompt

> Review the attached CrisisGraph SFT V2 candidate records using the attached
> `triage-annotation-guidelines.md`. Treat every input and target as unverified.
> For the training file, review all 240 records in four sequential phases:
> records 1–60, 61–120, 121–180, and 181–240. Continue automatically between
> phases without requesting confirmation. These ranges refer to one-based JSONL
> record order, not numeric suffixes in record IDs. Apply the same guidelines
> throughout; do not sample or skip repetitive examples. For a separate validation
> review request, review all 48 validation records instead of the training phases.
> For every supplied record ID,
> derive the target from the input and report `first_pass_reviewed` or
> `needs_revision`, a concise reason, and exact corrections where needed. Preserve
> unrelated explicit facts when another field is missing or contradictory. Do not
> infer vehicles, counts, locations, or durations. Distinguish availability questions
> from actual requests, numbers in embedded instructions from genuine counts, and
> explicit unknown values from plausible guesses. Check Malayalam grammar, Manglish
> naturalness, uncertainty reasons, and the documented confidence labels. Identify
> repeated-template problems and policy disagreements separately. Do not silently
> rewrite files or mark records `reviewed`; this is a first pass. Return one verdict
> and concise reason for every record ID, with exact corrections where needed.
> After each phase, report accepted and needs-revision counts. Finish with combined
> totals, systematic issues, and a coverage check confirming that all 240 unique
> training IDs (or all 48 validation IDs in the separate request) were reviewed
> exactly once. If any ID is missing or duplicated, report it explicitly.
> If an output or context limit prevents completion, state the last completed
> record's position and ID and the remaining range. Do not claim full completion
> or replace individual review with sampling. If reviewing as ChatGPT, identify
> the report as an AI-assisted first-pass review, not human or domain-expert review.

Suggested report names:

- `triage-sft-v2-train-first-pass-review-report.md` (retain all four batch results)
- `triage-sft-v2-validation-first-pass-review-report.md`

## Reproduction and validation

Catalog: `triage-sft-v2-scenario-catalog.json`.
Generator: `../src/crisisgraph_ml/triage_sft_v2_data.py`.
Manifest: `triage-sft-v2-train-validation.manifest.json`.

From `ml/`:

```bash
.venv/bin/python -m crisisgraph_ml.triage_sft_v2_data validate
```

To reproduce into a new directory (existing outputs are never overwritten):

```bash
.venv/bin/python -m crisisgraph_ml.triage_sft_v2_data generate --data-dir /tmp/crisisgraph-sft-v2-reproduction
```

The manifest records candidate, generator, catalog and reference hashes. Reference
datasets are read for overlap checks only, not as generation inputs. The current
notebook still targets V1 exports; update its dataset paths/hashes only after V2
review and an explicitly recorded export decision. Train a fresh adapter from the
same Qwen base, not by continuing V1. Keep prompt V2.1.1 and initial training and
serving settings comparable; report base/V1/V2 on the frozen regression suite, then
use a separately reviewed blind holdout for final generalization claims.
