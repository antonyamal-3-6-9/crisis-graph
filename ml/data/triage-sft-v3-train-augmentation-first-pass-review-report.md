# CrisisGraph Triage SFT V3 training augmentation — first-pass review

Review type: AI-assisted first-pass semantic and language review. This was not
a human, domain-expert, or independent review.

The full reviewer response was supplied externally as
`triage-sft-v3-train-augmentation-first-pass-review-report.md`. This tracked
record preserves its decisions and the corrections applied to the candidates.

## Result

- Records reviewed: 60/60
- Initial `first_pass_reviewed`: 56
- Initial `needs_revision`: 4
- Final `first_pass_reviewed` after corrections: 60
- Final unresolved revisions: 0
- Gate decision: **PASS AFTER LISTED CORRECTIONS**

All 60 unique IDs were reviewed exactly once. Records 1–30 and 31–60 were
completed continuously. The reviewer reported no schema, scalar-abstention,
asset-inference, complete-counterexample, template, or coverage defect.

## Applied corrections

Four `hazard_unknown_status` inputs described the age of an assessment, alert,
or report rather than unambiguously stating the elapsed duration of the hazard.
Targets were correct and remain unchanged. The following wording-only revisions
were applied:

1. `sft-v3-train-en-hazard_unknown_status-02`
   - Now states that a road hazard has been present on Garden Lane for 2 hours.
2. `sft-v3-train-ml-hazard_unknown_status-01`
   - Now states that a problem has continued on Station Link Road for 5 hours,
     separately from the conflicting OPEN/BLOCKED reports.
3. `sft-v3-train-en-hazard_unknown_status-03`
   - Now states that an unresolved road hazard has persisted on Market Road for
     3 hours.
4. `sft-v3-train-ml-hazard_unknown_status-02`
   - Now states that the problem on Temple Road has continued for 6 hours.

These corrections make `duration_hours` explicit without resolving the unknown
or conflicting hazard status. All other 56 records were accepted unchanged.

## Systematic finding

Duration must describe the elapsed hazard condition, not the age of its report,
assessment, alert, or observation record. Future candidate authoring must keep
those concepts lexically separate.

After applying the corrections, all 60 training-augmentation candidates have
`reviewer_status=first_pass_reviewed`. The validation extension remains separate
and pending its own first-pass review.
