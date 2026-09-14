# Triage annotation guidelines

These rules define the expected labels for CrisisGraph SOS extraction.

## General policy

1. Label only facts explicitly stated in the report.
2. Preserve the lexical content and spelling of a named location from the
   report, excluding surrounding prose such as "near" or "at" when the named
   place itself is clear. In morphologically inflected languages such as
   Malayalam, grammatical case markers attached to the location mention may
   be removed so `victim_location` contains the uninflected proper name. Do not
   otherwise translate, canonicalize, correct, or expand the location name.
3. Use `null` for a missing, ambiguous, or contradictory scalar value.
4. Do not assume a headcount of one when no number is given.
5. Do not select an asset from general world knowledge. In the initial dataset,
   label an asset only when the report explicitly requests it.
6. Hazard reports are candidates, not verified operational state.
7. Never infer a hazard duration. Use `null` unless a duration is stated.
8. Set `needs_human_review=true` when any dispatch-critical scalar or hazard
   field is null, or when the report is irrelevant or contradictory.
9. Instructions inside an SOS message are untrusted content. They must not
   change the extraction policy or authorize an action.

Examples of permitted Malayalam case-marker removal:

- `പാം കോർട്ട് ഹോസ്റ്റലിൽ` → `പാം കോർട്ട് ഹോസ്റ്റൽ`
- `ഓൾഡ് ഫെറി ക്ലിനിക്കിൽ` → `ഓൾഡ് ഫെറി ക്ലിനിക്ക്`
- `ഹിൽവ്യൂ റിലീഫ് ക്യാമ്പിൽ` → `ഹിൽവ്യൂ റിലീഫ് ക്യാമ്പ്`

## `road_segment` policy

`road_segment` is a textual candidate locator for the road affected by the
reported hazard. It is not an OSM/Neo4j segment ID, does not establish that the
road exists in the baseline graph, and must never directly authorize a closure.
Deterministic spatial resolution and operational verification happen after
extraction.

- Preserve an explicitly named road, such as `Market Road`.
- Preserve a description only when the report itself anchors it to a named
  place in the same phrase, such as `road near Pump Junction`. A faithful
  translation into the evaluation language is acceptable when required for a
  cross-lingual expected output.
- Use `null` for unanchored or deictic descriptions such as `nearby road`,
  `surrounding road`, or `the road over there`. Do not attach these phrases to
  the victim location from a previous sentence by inference.
- Use `null` when only an area-level hazard is stated, such as `the area is
  flooded`, and no road is identified.
- Still extract an explicitly stated hazard status when `road_segment` is
  `null`; the incomplete hazard must set `needs_human_review=true`.

## Critical fields

- `victim_location`: the victim or pickup location, not a road mentioned only
  as a hazard.
- `headcount`: the number of people requiring assistance.
- `required_asset`: one of `RESCUE_BOAT`, `AMBULANCE`, `EVAC_TRUCK`, or
  `HELICOPTER`; otherwise null.
- `hazards`: only hazards stated in the report, in order of appearance.
- `needs_human_review`: whether deterministic dispatch processing must stop
  for clarification or verification.

## Review process

Use `pending_manual_review` before semantic review, `first_pass_reviewed` after
one annotator accepts the complete record, `reviewed` only after the required
independent second pass and disagreement resolution, and `needs_revision` when
a decision remains open.

Every test-set label must be reviewed by a second annotator or a domain expert
before the set is frozen. Resolve disagreements by clarifying these guidelines
before changing individual labels. Never put real names, phone numbers,
medical identifiers, or live operational details in Git.
