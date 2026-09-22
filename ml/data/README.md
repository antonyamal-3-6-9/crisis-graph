# Dataset policy

Only synthetic, consented, or properly de-identified reports belong here.

Curated evaluation fixtures may be tracked when they contain no operational or
personal information. Raw reports, generated corpora, operational exports, and
training derivatives are ignored by `ml/.gitignore`.

Every labelled case should record its provenance, language, scenario category,
expected fields, expected human-review decision, and reviewer status.

Apply `triage-annotation-guidelines.md` when creating or reviewing labels. The
current `triage-eval-v1.jsonl` is a small seed dataset for exercising the
evaluation pipeline; it is not yet statistically representative or
domain-expert certified.

`triage-eval-v2-candidates.jsonl` contains the 120 reproducible synthetic source
candidates. `triage-eval-v2-second-review.jsonl` records the labels accepted in
the first semantic pass. Both semantic reviews are complete, and the immutable
benchmark is `triage-eval-v2.0.0.jsonl`; its provenance, checksum, permitted
uses, and reporting requirements are recorded in
`triage-eval-v2.0.0.manifest.json`.

Do not edit the frozen JSONL in place. A substantive label or policy correction
requires a new dataset version, a documented reason, and a fresh checksum. Do
not use the frozen benchmark for model training, prompt tuning, or generating
training examples.

The SFT pilot candidates are split before review to prevent accidental
train/validation/dev mixing:

- `triage-sft-v1-train-candidates.jsonl` — 240 records
- `triage-sft-v1-validation-candidates.jsonl` — 48 records
- `triage-sft-v1-dev-candidates.jsonl` — 48 records

They are not training-ready. Review them using
`triage-sft-v1-review-guide.md`; generation metadata and candidate checksums are
in `triage-sft-v1-manifest.json`. Candidate revision 3 incorporates both prior
dev-pass findings; all regenerated records require fresh review.

The revision-3 train and validation candidates completed first-pass review and
were exported under the explicit `pilot-first-pass` policy. The generated
chat-format JSONL files remain ignored and reproducible; their source, prompt,
record counts, and checksums are tracked in
`triage-sft-v1-export-manifest.json`.

V2 and V3 are controlled synthetic iterations, not replacements for a
field-derived corpus:

- V2 tested broader contradiction and review behavior but regressed on hazards
  and Malayalam in the frozen regression suite.
- V3 combines the unchanged 240-record V1 training split with 60 targeted,
  first-pass-reviewed additions. Its validation set combines the unchanged 48
  V1 records with 24 additions, producing 300 train and 72 validation records.
- `triage-sft-v3-export-manifest.json` records source hashes, export hashes,
  review status, language counts, and overlap checks. The ignored chat exports
  can be reproduced from those reviewed sources.
- The 120-case frozen evaluation set was not included in any training export.

The V3 dataset received AI-assisted first-pass semantic/language review with
documented wording corrections. It did not receive an independent human or
domain-expert acceptance pass, and it is synthetic. Reports and resume claims
must retain those qualifications.

The V4 counterexample files are a deferred development diagnostic informed by
errors on the exposed regression suite. They are neither training-ready nor a
blind holdout and should not be mixed into a training export without a new,
documented experiment decision.
