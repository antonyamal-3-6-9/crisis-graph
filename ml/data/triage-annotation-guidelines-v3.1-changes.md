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

## 3. Suspected life-threatening other conditions (owner decision, 2026-10-26)

The safety asymmetry of principle 3 also applies to life-threatening situations
outside the categories: a stated *suspicion* of snakebite, electric shock,
drowning or near-drowning is labelled as present (e.g. "I think a snake bit
him" → `other_conditions: ["suspected snakebite"]`). Missing one is far worse
than an extra entry; under the urgency rules it is CRITICAL with immediate
review.

## 4. Water that is still coming in is RISING (owner decision, 2026-10-26)

Ongoing entry of water counts as `water_trend: RISING`: "water entering",
"still entering", "keeps coming in", "coming up", "vellam kayari varunnu",
"vellam kayari thudangi", "വെള്ളം കയറിക്കൊണ്ടിരിക്കുന്നു". Completed entry
stated in the past with no ongoing sense ("water entered the house last
night") stays `null`.
