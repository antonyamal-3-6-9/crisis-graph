# SFT V2 batch 01 first-pass review disposition

Source: detailed review supplied by the user in the conversation. This document
records that review and its disposition, not a new independent review.

Initial result: 23 first-pass acceptances and one wording revision among 24
language variants of eight scenario families. The reviewer found no extraction
target errors. The reviewer conditionally accepted all 24 after the exact wording
correction and documentation or confirmation of the confidence-label convention.

## Applied correction

Record: `sft-v2-dev-ml-unverified_group_size_range`.

- Before: `ആളുകൾക്കാണ് എന്നാണ് തോന്നുന്നത്`
- After: `ആളുകൾക്കാണെന്നാണ് തോന്നുന്നത്`
- All target JSON fields remain unchanged.
- Both the stored candidate and generator were corrected.

The canonical annotation guidelines now explicitly document the existing synthetic
SFT confidence labels: 0.95 for complete reports, 0.55 for relevant review-required
reports, and 0.10 for irrelevant messages. These are not calibrated probabilities.

## Acceptance and provenance

All 24 current candidates are `first_pass_reviewed` under the reviewer's explicit
conditional acceptance. Revision 2 changes only one input phrase plus revision
and review metadata; the other 23 inputs and all 24 targets are preserved.
The manifest retains the original revision-1 hashes and review outcome alongside
the current revision-2 hashes. Fresh generator output remains pending manual review;
it does not automatically inherit acceptance from this reviewed artifact.

No independent second pass has been claimed. The batch is not frozen or approved
for training. Its eight scenario families remain reserved for dev. The next
authoring step is the remaining dev batch, adding subtler conflicts, different
conflict positions, missing fields, irrelevant inputs, and additional clear and
adversarial cases. The initial batch alone does not establish broad coverage.
