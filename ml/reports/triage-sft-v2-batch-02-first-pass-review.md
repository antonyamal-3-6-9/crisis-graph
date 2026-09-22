# SFT V2 batch 02 first-pass review disposition

Source: user-supplied detailed first-pass review in the conversation. This records
the supplied review and correction, not an additional independent second pass.

Initial result: 23 `first_pass_reviewed`, one `needs_revision`. The reviewer found
no extraction-target or confidence-policy errors; the only issue was Malayalam
wording in `sft-v2-dev-ml-museum_rescue_equipment_exhibit`.

## Correction and acceptance

- Before: `അടിയന്തര സാഹചര്യത്തിനോ യാത്രാസഹായത്തിനോ ഉള്ള അപേക്ഷയല്ല.`
- After: `അടിയന്തര സഹായത്തിനോ യാത്രാസഹായത്തിനോ ഉള്ള അപേക്ഷയല്ല.`
- Applied to both the stored candidate and generator.
- No target fields changed. All other inputs remain unchanged.

The reviewer explicitly conditionally accepted all 24 after this exact correction.
The revision-2 artifact therefore marks all 24 `first_pass_reviewed`. The manifest
preserves revision-1 hashes and the original 23/1 outcome, with current hashes and
the correction disposition. Fresh generated copies remain pending manual review.

## Combined dev gate

Batch 01 and Batch 02 now total 48 first-pass-accepted language variants across
16 scenario families. Independent second-pass acceptance is still pending. Neither
batch is frozen or exported for training; these families remain reserved for dev.

The next authoring step is 240 training and 48 validation candidates, with separate
scenario and wording families. Keep the same base model and initial experiment
settings for the planned base/V1/V2 comparison. No GPU training has started for V2.
