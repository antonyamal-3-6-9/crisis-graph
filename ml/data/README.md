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

## Triage extraction V3 (`schemas/triage-extraction-v3.json`)

- `triage-eval-v3-dev-candidates.jsonl` — 42 owner-reviewed cases for **prompt
  development only**; never reported as results. Generator:
  `crisisgraph_ml.triage_eval_v3_dev`; review sheet
  `triage-eval-v3-dev-review-guide.md`.
- `triage-eval-v3.0.1.jsonl` — the **frozen** 120-case V3 benchmark (frozen
  2026-10-25; supersedes v3.0.0, re-frozen before any use, reason in the manifest). Checksum, owner decisions, contract hashes, permitted and
  prohibited uses are in `triage-eval-v3.0.1.manifest.json`. Same rules as V2:
  never edit in place, never use for training, fine-tuning, prompt tuning or
  generating training examples. Tests fail if the file, its generator
  (`crisisgraph_ml.triage_eval_v3_candidates`) or the contract files change.
- `triage-annotation-guidelines-v3.1-changes.md` — labelling rules decided after
  the v3.0.1 freeze (e.g. people stranded on a roof); they apply to every
  later dataset.
- `triage-annotation-guidelines-v3.2-changes.md` — rules decided after the v3.1.0
  freeze (changes 5–13: coordinator phrasing, "cannot walk without help",
  labour, "keeps rising", bleeding gold, autism, approximate counts, stranding
  gold, resolved events).
- `urgency-rules-v5.jsonl` — current urgency rules: v4 plus a child with water at
  waist level or above, and a bedridden person at knee level with rising water,
  are CRITICAL. v4 = v3 plus a child with water at chest level or above. v3 = v2 (any `other_conditions` entry is
  CRITICAL with immediate review) plus a low-mobility group or a pregnant woman
  with water at chest level or above. v1–v4 are kept unchanged to reproduce
  earlier results.
- `triage-sft-v4-new-candidates.jsonl` — SFT-V4 new records (batches 1–7),
  generated by `triage_sft_v4_new.py`: batch 1–5 labels with texts rewritten by
  zero-context agents (`triage-sft-v4-new-rewrites.json`), batches 6–7 written
  by zero-context agents (`triage-sft-v4-batch6-agent.jsonl`,
  `triage-sft-v4-batch7-agent.jsonl`) under `triage-sft-v4-owner-policies.md`.
  Records that matched evaluation items are dropped, and the remaining overlap
  is recorded for the sensitivity check, in `triage-sft-v4-leak-audit.json`.
- `triage-eval-v3.1.0.jsonl` — the **frozen blind** V3 test set for SFT-V4 (frozen
  2026-10-26): drafted by a zero-context agent, AI-reviewed, owner-approved.
  The agent's draft (`-candidates.jsonl`) is unchanged; every change is in
  `triage-eval-v3.1.0-corrections.json` with its reason. Manifest:
  `triage-eval-v3.1.0.manifest.json`.
- `urgency-rules-v1.jsonl` — hand-specified cases for the deterministic urgency
  rules, shared by the Python reference (`crisisgraph_ml.urgency`) and the Rust
  implementation.
