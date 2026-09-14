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
