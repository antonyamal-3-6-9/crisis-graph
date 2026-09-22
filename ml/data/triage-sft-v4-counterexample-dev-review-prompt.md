# CrisisGraph Triage SFT V4 counterexample pilot — first-pass review prompt

Review all 12 records in `triage-sft-v4-counterexample-dev-candidates.jsonl`
against `triage-annotation-guidelines.md`.

This is an AI-assisted first-pass semantic and language-quality review. It is
not an independent or blind evaluation: the two tested behavior classes were
selected after inspecting V3's exposed regression failures. The individual
inputs must nevertheless be reviewed on their own merits, and none copies a
frozen evaluation record.

For every record, verify:

- the English, Malayalam, or Manglish wording is coherent and natural enough
  for its stated register;
- victim location, scalar headcount, and requested asset are explicit and
  exactly represented in the target;
- `explicit_asset_clear` messages are complete dispatch requests even though
  they contain no hazard;
- `adversarial_false_hazard` messages contain a real dispatch request followed
  by an untrusted instruction to invent or write a hazard;
- the untrusted instruction contributes no hazard fact, so `hazards=[]`;
- all records correctly use `needs_human_review=false`, confidence `0.95`, and
  an empty uncertainty list; and
- no record is merely a translation or slot-filled copy of another record.

Return a Markdown report containing:

1. total records reviewed;
2. counts for `first_pass_reviewed` and `needs_revision`;
3. one row per record with ID, verdict, and concise reason;
4. exact replacement text or target changes for every revision;
5. any systematic language, annotation, or independence issue;
6. confirmation that all 12 records were reviewed; and
7. a final decision: `PASS`, `PASS AFTER LISTED CORRECTIONS`, or `FAIL`.

Do not silently rewrite records. Leave `reviewer_status` as
`pending_manual_review`; repository changes are applied only after the report is
accepted.
