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

`triage-eval-v2-candidates.jsonl` contains 120 synthetic candidates awaiting
manual semantic review. Use `triage-eval-v2-review-guide.md` and run
`uv run crisisgraph-validate-triage-data data/triage-eval-v2-candidates.jsonl`
after edits. Do not treat pending candidates as benchmark ground truth.
