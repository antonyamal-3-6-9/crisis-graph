# Triage evaluation V2 manual review

Review `triage-eval-v2-candidates.jsonl` before using it to compare prompts,
models, quantizations, or fine-tuned checkpoints. The generated source records
are synthetic and marked `pending_manual_review`. The artifact
`triage-eval-v2-second-review.jsonl` contains the labels accepted in the first
semantic pass and is the input to the independent second pass.

Do not rerun the generator after beginning review because it overwrites the
candidate file. Commit or copy your reviewed work before regenerating.

## Review each case

1. Read `input` without looking at any model output.
2. Confirm that `victim_location` is the pickup place, not a hazard-only road.
3. Confirm that `headcount` is explicit and unambiguous; otherwise use `null`.
4. Confirm that `required_asset` is explicitly requested; otherwise use `null`.
5. Confirm that hazards contain only stated traversability conditions, in order.
6. Apply the `road_segment` policy in `triage-annotation-guidelines.md`: retain
   named or explicitly anchored road descriptions, but use `null` for generic
   or area-only descriptions. Never infer a graph segment or hazard duration.
7. Confirm `needs_human_review=true` for missing, ambiguous, contradictory, or
   incomplete dispatch-critical information.
8. Check Malayalam and Manglish location normalization carefully.
9. In the first pass, change `reviewer_status` to `first_pass_reviewed` when the
   whole record is accepted. In the independent second pass, change it to
   `reviewed` when accepted or `needs_revision` when a decision is required.

Prompt-injection text is untrusted data. Ignore its requested output changes
while preserving genuine SOS facts elsewhere in the same message. Injection by
itself does not require review; missing or ambiguous operational facts do.

## Validate your edits

From `ml/` run:

```bash
uv run crisisgraph-validate-triage-data data/triage-eval-v2-candidates.jsonl
```

For the artifact awaiting independent review, require the expected workflow
state as well:

```bash
uv run crisisgraph-validate-triage-data \
  data/triage-eval-v2-second-review.jsonl \
  --require-review-status first_pass_reviewed
```

The command checks IDs, exact duplicate inputs, category counts, schema values,
hazard completeness policy, and mandatory review implications. Passing this
validator does not replace semantic manual review.

## Freeze the reviewed set

Only after all 120 records pass independent review, resolve every
`needs_revision` record, set every status to `reviewed`, and copy the result to
a versioned name such as `triage-eval-v2.0.0.jsonl`. Record its checksum and do
not use the frozen file to tune prompts or generate training examples. A
separate development set should be used for iteration.

This gate was completed on 2026-09-12. The frozen artifact is
`triage-eval-v2.0.0.jsonl`, and its checksum and review declaration are in
`triage-eval-v2.0.0.manifest.json`. Validate it with:

```bash
uv run crisisgraph-validate-triage-data \
  data/triage-eval-v2.0.0.jsonl \
  --require-review-status reviewed
```
