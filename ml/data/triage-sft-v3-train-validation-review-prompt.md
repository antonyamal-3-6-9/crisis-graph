# CrisisGraph Triage SFT V3 additions — first-pass review prompt

Use `triage-annotation-guidelines.md` and
`triage-sft-v3-augmentation-spec.md` to review the supplied V3 candidate file.
This is an AI-assisted first-pass semantic and language review, not a human,
domain-expert, or independent review.

## Training-augmentation review

When reviewing `triage-sft-v3-train-augmentation-candidates.jsonl`, review all
60 records in one continuous task. You may process records 1–30 and 31–60 as
internal phases, but continue automatically without asking for confirmation.
The ranges refer to one-based JSONL order. Do not sample or skip records.

## Validation-extension review

Review `triage-sft-v3-validation-extension-candidates.jsonl` separately from the
training file. Review all 24 records. Do not use validation records to propose
changes to individual training labels, and never move validation examples into
training.

## Checks for every record

- Derive the target only from the input.
- Preserve unrelated explicit facts when one field is unresolved.
- Use `null` for missing, approximate, contradictory, or unavailable scalar facts.
- Do not infer an asset from medical condition, mobility, flooding, terrain, or
  likely operational need.
- Evaluate hazard road, status, and duration independently. Do not infer an
  unnamed road from the victim location or infer duration from a clock time.
- Distinguish unresolved contradictions from explicit corrections where a final
  value clearly replaces an earlier value.
- Check `needs_human_review`, uncertainty reasons, and the documented
  `0.95`/`0.55` confidence labels.
- Check English clarity, Malayalam grammar and case-marker normalization, and
  Manglish fluency.
- Identify translations, repeated sentence templates, or semantic near-duplicates
  across the supplied records.
- Confirm that each target conforms to the extraction schema.

## Required output

Return a Markdown report with:

1. the filename and total records reviewed;
2. `first_pass_reviewed` and `needs_revision` counts;
3. one row for every record ID, in file order, with verdict and concise reason;
4. exact input or target replacements for every requested revision;
5. systematic policy, fluency, template, or coverage issues;
6. confirmation that every unique ID was reviewed exactly once; and
7. `PASS`, `PASS AFTER LISTED CORRECTIONS`, or `FAIL`.

Do not silently rewrite records and do not mark anything `reviewed`. If an output
limit interrupts the review, state the last completed one-based record position
and ID and the exact remaining range; do not claim completion.

Use separate report filenames:

- `triage-sft-v3-train-augmentation-first-pass-review-report.md`
- `triage-sft-v3-validation-extension-first-pass-review-report.md`
