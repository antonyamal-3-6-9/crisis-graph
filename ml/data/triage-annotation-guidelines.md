# Triage annotation guidelines

These rules define the expected labels for CrisisGraph SOS extraction.

## General policy

1. Label only facts explicitly stated in the report.
2. Preserve a named location as written, excluding surrounding prose such as
   "near" or "at" when the named place itself is clear.
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

Every test-set label should eventually be reviewed by a second annotator or a
domain expert. Resolve disagreements by clarifying these guidelines before
changing individual labels. Never put real names, phone numbers, medical
identifiers, or live operational details in Git.
