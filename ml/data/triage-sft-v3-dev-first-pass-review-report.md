# CrisisGraph Triage SFT V3 dev-pilot first-pass review

Review type: AI-assisted first-pass semantic and language review. This was not
a human, domain-expert, or independent second review.

## Result

- Records reviewed: 18/18
- Initially accepted: 17
- Initially requiring revision: 1
- Corrections applied: 1 record and 1 policy-documentation clarification
- Final unresolved revisions: 0
- Final gate: **PASS AFTER LISTED CORRECTIONS**

## Record decisions

| ID | Initial verdict | Final disposition |
|---|---|---|
| `sft-v3-dev-en-001` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-en-002` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-en-003` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-en-004` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-en-005` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-en-006` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-ml-001` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-ml-002` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-ml-003` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-ml-004` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-ml-005` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-ml-006` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-manglish-001` | `needs_revision` | Wording corrected; target unchanged; accepted |
| `sft-v3-dev-manglish-002` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-manglish-003` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-manglish-004` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-manglish-005` | `first_pass_reviewed` | Accepted unchanged |
| `sft-v3-dev-manglish-006` | `first_pass_reviewed` | Accepted unchanged |

## Applied record correction

For `sft-v3-dev-manglish-001`, replace the original input with:

> 19 perulla same groupinte pickup location randu callil different aanu. Oru
> callil Rose Tuition Centre ennum mattoru callil Mango Reading Room ennum
> paranju; correct place confirm aayittilla. Rescue boat venam.

This improves Manglish fluency without changing the two unresolved pickup
locations, headcount, requested asset, or target JSON.

## Applied policy clarification

The confidence-label section of `triage-annotation-guidelines.md` previously
named only V1 and V2. It now explicitly applies the same fixed `0.95`, `0.55`,
and `0.10` synthetic-label convention to V3. No target labels changed.

## Semantic findings

- Contradiction cases null only the unresolved scalar and preserve unrelated
  explicit facts.
- The resolved roster correction correctly uses the explicitly verified final
  value and does not escalate.
- The missing-asset case does not infer a vehicle from injury or transport need.
- Hazard road, status, and duration are treated as independent fields.
- Malayalam case-marker removal follows the documented lexical-preservation rule.
- All three complete counterexamples remain `needs_human_review=false`.
- No record appears to be a direct translation or slot-filled variant of another
  pilot record.

After applying the two listed corrections, all 18 candidates have
`reviewer_status=first_pass_reviewed`. The V3 dev gate is complete, and authoring
of the separate training augmentation and validation extension may begin.
