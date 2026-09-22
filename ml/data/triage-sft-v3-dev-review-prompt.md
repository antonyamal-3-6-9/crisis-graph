# CrisisGraph Triage SFT V3 dev-pilot first-pass review prompt

Review all 18 records in `triage-sft-v3-dev-candidates.jsonl` against:

1. `triage-annotation-guidelines.md`; and
2. `triage-sft-v3-augmentation-spec.md`.

This is a first-pass semantic and language-quality review. Complete all 18
records in this review. You may inspect them in internal phases, but do not stop
after one language or category and do not treat this as an independent second
review.

For every record, verify:

- the input is a coherent, fictional emergency report;
- English, Malayalam, or Manglish wording is grammatically acceptable and
  natural enough for its stated register;
- `victim_location`, `headcount`, and `required_asset` contain only explicit,
  unambiguous facts;
- contradictory, approximate, missing, or unavailable scalar facts are `null`;
- hazard road, status, and duration are independently extracted, with no
  inference from timestamps, nearby victim locations, or general knowledge;
- an explicitly resolved earlier value is not incorrectly treated as an
  unresolved contradiction;
- no vehicle is inferred from injury, mobility, water, terrain, or likely need;
- `needs_human_review`, `confidence_score`, and `uncertainty_reasons` follow the
  annotation policy;
- the target conforms to the extraction schema; and
- scenario wording does not look like a translation or slot-filled variant of
  another pilot record.

Pay particular attention to Malayalam case-marker removal, Malayalam/Manglish
fluency, the distinction between an unknown hazard status and no hazard report,
and complete counterexamples that must remain `needs_human_review=false`.

Return a Markdown report containing:

1. total records reviewed;
2. counts for `first_pass_reviewed` and `needs_revision`;
3. one row per record with ID, verdict, and concise reason;
4. exact replacement text or target JSON changes for every revision;
5. any newly discovered systematic policy or generation issue;
6. confirmation that all 18 records were reviewed; and
7. a final gate decision: `PASS`, `PASS AFTER LISTED CORRECTIONS`, or `FAIL`.

Do not silently rewrite records. Do not label this review as human,
domain-expert, or independent unless that accurately describes the reviewer.
Candidate records currently remain `pending_manual_review` until the findings
are applied and accepted.
