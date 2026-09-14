# Triage SFT Pilot V1 review guide

The three `triage-sft-v1-*-candidates.jsonl` files are synthetic candidates.
Do not export pending or rejected records. The exporter defaults to requiring
`reviewer_status: "reviewed"`; this controlled pilot may explicitly select the
`pilot-first-pass` policy after a complete first pass.

Candidate revision 3 was regenerated after revision 1 exposed semantic template
issues and revision 2 exposed unnatural Malayalam suffix concatenation. Earlier
review reports and status-only copies describe obsolete candidate revisions and
must not be merged into these files. The revision-3 first pass initially
accepted 38 records and held 10 Malayalam records for a location-normalization
policy decision. After that policy was clarified in
`triage-annotation-guidelines.md`, all 10 passed targeted re-review. All 48 dev
records are now `first_pass_reviewed`; the independent second pass is pending.
All 240 training records subsequently passed first-pass review in four
consecutive 60-record batches and are also `first_pass_reviewed`. Validation
then passed first-pass review with all 48 records accepted. Every split has now
completed its first pass; independent acceptance is still pending.

## Split roles

- `train` updates model weights.
- `validation` controls checkpoint selection and early stopping during
  fine-tuning. It must never be merged into training.
- `dev` is for inspecting behavior and choosing experiment settings. It must
  never be merged into training or validation.
- `triage-eval-v2.0.0.jsonl` is separate frozen evaluation data and must never
  enter any SFT split.

## Review every record

1. Read `input` before reading `target`.
2. Apply `triage-annotation-guidelines.md`, including the clarified textual
   `road_segment` policy.
3. Confirm location, headcount, and asset are explicit and unambiguous or
   correctly `null`.
4. Confirm hazards contain only stated road conditions in appearance order.
5. Confirm every incomplete or contradictory critical fact forces
   `needs_human_review=true`.
6. Confirm adversarial instructions do not alter genuine SOS facts.
7. Confirm irrelevant inputs contain no invented SOS facts.
8. Check Malayalam and Manglish phrasing with a fluent reviewer; generated
   text may be grammatically awkward even when its target is mechanically
   consistent.
9. Treat `confidence_score` as a coarse label policy, not a calibrated
   probability: `0.95` for complete explicit reports, `0.55` for review cases,
   and `0.10` for irrelevant messages.
10. Ensure `uncertainty_reasons` accurately describe every reason for review.

Set `reviewer_status` to `first_pass_reviewed` after the first complete pass,
`reviewed` only after independent acceptance, or `needs_revision` when a label
or input requires correction. Regenerate only before review begins because the
generator overwrites candidate files.

## Validate

From `ml/`:

```bash
uv run crisisgraph-triage-sft-data validate
```

Validation checks JSON Schema compliance, fixed split sizes and language
counts, unique IDs and normalized inputs, split metadata, scenario-family
isolation, frozen-evaluation overlap, and mandatory review implications. It
does not replace semantic or language review.

## Export after review

Only after all records in a split are independently accepted:

```bash
uv run crisisgraph-triage-sft-data export \
  data/triage-sft-v1-train-candidates.jsonl \
  data/generated/triage-sft-v1-train.jsonl
```

For this documented synthetic pilot, first-pass-reviewed records may instead
be exported explicitly:

```bash
uv run crisisgraph-triage-sft-data export \
  data/triage-sft-v1-train-candidates.jsonl \
  data/generated/triage-sft-v1-train.jsonl \
  --review-policy pilot-first-pass
```

Repeat separately for validation. Exported chat-format data is intentionally
ignored by Git. Record reviewed-candidate checksums in a new manifest rather
than overwriting the generation manifest.
