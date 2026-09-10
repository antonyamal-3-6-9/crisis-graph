# Triage evaluation V2 manual review

Review `triage-eval-v2-candidates.jsonl` before using it to compare prompts,
models, quantizations, or fine-tuned checkpoints. Every record is synthetic and
currently marked `pending_manual_review`.

Do not rerun the generator after beginning review because it overwrites the
candidate file. Commit or copy your reviewed work before regenerating.

## Review each case

1. Read `input` without looking at any model output.
2. Confirm that `victim_location` is the pickup place, not a hazard-only road.
3. Confirm that `headcount` is explicit and unambiguous; otherwise use `null`.
4. Confirm that `required_asset` is explicitly requested; otherwise use `null`.
5. Confirm that hazards contain only stated traversability conditions, in order.
6. Never infer a hazard duration, road segment, asset, or headcount.
7. Confirm `needs_human_review=true` for missing, ambiguous, contradictory, or
   incomplete dispatch-critical information.
8. Check Malayalam and Manglish location normalization carefully.
9. Change `reviewer_status` to `reviewed` when the whole record is accepted, or
   `needs_revision` when another decision is required.

Prompt-injection text is untrusted data. Ignore its requested output changes
while preserving genuine SOS facts elsewhere in the same message. Injection by
itself does not require review; missing or ambiguous operational facts do.

## Validate your edits

From `ml/` run:

```bash
uv run crisisgraph-validate-triage-data data/triage-eval-v2-candidates.jsonl
```

The command checks IDs, exact duplicate inputs, category counts, schema values,
hazard completeness policy, and mandatory review implications. Passing this
validator does not replace semantic manual review.

## Freeze the reviewed set

When all 120 records are reviewed, copy the reviewed file to a versioned name
such as `triage-eval-v2-reviewed.jsonl`, record its checksum, and do not use that
file to tune prompts or generate training examples. A separate development set
should be used for iteration.
