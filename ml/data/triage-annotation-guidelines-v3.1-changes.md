# Annotation guideline changes after triage-eval-v3.0.1

`triage-annotation-guidelines.md` is pinned by checksum in the
`triage-eval-v3.0.1` manifest, so rules decided after that freeze are recorded
here. They apply to **every dataset created after v3.0.1** (SFT-V4 training
data, the next blind test set) and are merged into the main guideline when the
next benchmark version is frozen. v3.0.1 itself keeps its original labels;
cases it labels differently are documented gaps, not relabelled.

## 1. People stranded on a roof or at height (owner decision, 2026-10-26)

People stranded on a roof, terrace, tree or other height and waiting for rescue
get `other_conditions: ["stranded on a roof"]` (or "stranded on a terrace",
"stranded in a tree"). This is a stated situation, not an inferred water level:
`water_level` stays `null` unless the depth or the flooding is stated, as
before.

Under `urgency-rules-v2` any `other_conditions` entry is CRITICAL with immediate
human review, so these reports are prioritised at once without assuming a
flood.

Documented gap in v3.0.1: case `v3-test-085` ("a family of four on the roof of
the house next door") is labelled with no `other_conditions` and is
under-triaged under both rule versions.

## 2. Urgency rules v2 (owner decision, 2026-10-26)

Gold `expected_urgency` labels are set independently of the rules and are
unchanged. For reference, `urgency-rules-v2` makes any `other_conditions` entry
CRITICAL (was URGENT in v1); see `urgency-rules-v2.jsonl`.
