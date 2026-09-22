# CrisisGraph SFT V2 — Validation Candidates AI-Assisted First-Pass Review

Scope: all 48 validation records, reviewed exactly once in JSONL order. This is an AI-assisted first-pass review, not human/domain-expert review.

## Summary

- `first_pass_reviewed`: **47**
- `needs_revision`: **1**
- total reviewed: **48**

| Pos | ID | Verdict | Reason |
|---:|---|---|---|
| 1 | `sft-v2-validation-en-resident_count_confirmed_by_rollcall-01` | `first_pass_reviewed` | Roll call confirms an exact headcount and the pickup location and requested asset are explicit. |
| 2 | `sft-v2-validation-en-resident_count_confirmed_by_rollcall-02` | `first_pass_reviewed` | Roll call confirms an exact headcount and the pickup location and requested asset are explicit. |
| 3 | `sft-v2-validation-ml-resident_count_confirmed_by_rollcall-01` | `first_pass_reviewed` | Roll call confirms an exact headcount and the pickup location and requested asset are explicit. |
| 4 | `sft-v2-validation-manglish-resident_count_confirmed_by_rollcall-01` | `first_pass_reviewed` | Roll call confirms an exact headcount and the pickup location and requested asset are explicit. |
| 5 | `sft-v2-validation-en-inconsistent_occupancy_form_fields-01` | `first_pass_reviewed` | Two form fields describe the same group with conflicting totals, so headcount=null while location and asset remain valid. |
| 6 | `sft-v2-validation-en-inconsistent_occupancy_form_fields-02` | `first_pass_reviewed` | Two form fields describe the same group with conflicting totals, so headcount=null while location and asset remain valid. |
| 7 | `sft-v2-validation-ml-inconsistent_occupancy_form_fields-01` | `first_pass_reviewed` | Two form fields describe the same group with conflicting totals, so headcount=null while location and asset remain valid. |
| 8 | `sft-v2-validation-manglish-inconsistent_occupancy_form_fields-01` | `first_pass_reviewed` | Two form fields describe the same group with conflicting totals, so headcount=null while location and asset remain valid. |
| 9 | `sft-v2-validation-en-voice_note_sender_unsure_of_venue-01` | `first_pass_reviewed` | The same group is associated with two unresolved venue names, so victim_location=null; count and asset remain explicit. |
| 10 | `sft-v2-validation-en-voice_note_sender_unsure_of_venue-02` | `first_pass_reviewed` | The same group is associated with two unresolved venue names, so victim_location=null; count and asset remain explicit. |
| 11 | `sft-v2-validation-ml-voice_note_sender_unsure_of_venue-01` | `first_pass_reviewed` | The same group is associated with two unresolved venue names, so victim_location=null; count and asset remain explicit. |
| 12 | `sft-v2-validation-manglish-voice_note_sender_unsure_of_venue-01` | `first_pass_reviewed` | The same group is associated with two unresolved venue names, so victim_location=null; count and asset remain explicit. |
| 13 | `sft-v2-validation-en-duplicate_vehicle_selection_boxes-01` | `first_pass_reviewed` | A single-choice form has two assets selected and no resolution, so required_asset=null. |
| 14 | `sft-v2-validation-en-duplicate_vehicle_selection_boxes-02` | `first_pass_reviewed` | A single-choice form has two assets selected and no resolution, so required_asset=null. |
| 15 | `sft-v2-validation-ml-duplicate_vehicle_selection_boxes-01` | `first_pass_reviewed` | A single-choice form has two assets selected and no resolution, so required_asset=null. |
| 16 | `sft-v2-validation-manglish-duplicate_vehicle_selection_boxes-01` | `first_pass_reviewed` | A single-choice form has two assets selected and no resolution, so required_asset=null. |
| 17 | `sft-v2-validation-en-group_size_lower_bound-01` | `first_pass_reviewed` | “At least” gives only a lower bound and the total is explicitly unknown, so headcount=null. |
| 18 | `sft-v2-validation-en-group_size_lower_bound-02` | `first_pass_reviewed` | “At least” gives only a lower bound and the total is explicitly unknown, so headcount=null. |
| 19 | `sft-v2-validation-ml-group_size_lower_bound-01` | `first_pass_reviewed` | “At least” gives only a lower bound and the total is explicitly unknown, so headcount=null. |
| 20 | `sft-v2-validation-manglish-group_size_lower_bound-01` | `first_pass_reviewed` | “At least” gives only a lower bound and the total is explicitly unknown, so headcount=null. |
| 21 | `sft-v2-validation-en-vehicle_question_not_actual_request-01` | `first_pass_reviewed` | Vehicle availability is asked about, but the message explicitly says no vehicle has been requested; required_asset=null. |
| 22 | `sft-v2-validation-en-vehicle_question_not_actual_request-02` | `first_pass_reviewed` | Vehicle availability is asked about, but the message explicitly says no vehicle has been requested; required_asset=null. |
| 23 | `sft-v2-validation-ml-vehicle_question_not_actual_request-01` | `first_pass_reviewed` | Vehicle availability is asked about, but the message explicitly says no vehicle has been requested; required_asset=null. |
| 24 | `sft-v2-validation-manglish-vehicle_question_not_actual_request-01` | `first_pass_reviewed` | Vehicle availability is asked about, but the message explicitly says no vehicle has been requested; required_asset=null. |
| 25 | `sft-v2-validation-en-group_size_audio_cutoff-01` | `first_pass_reviewed` | The call ends before any count is spoken, so headcount=null while pickup and asset remain explicit. |
| 26 | `sft-v2-validation-en-group_size_audio_cutoff-02` | `first_pass_reviewed` | The call ends before any count is spoken, so headcount=null while pickup and asset remain explicit. |
| 27 | `sft-v2-validation-ml-group_size_audio_cutoff-01` | `first_pass_reviewed` | The call ends before any count is spoken, so headcount=null while pickup and asset remain explicit. |
| 28 | `sft-v2-validation-manglish-group_size_audio_cutoff-01` | `first_pass_reviewed` | The call ends before any count is spoken, so headcount=null while pickup and asset remain explicit. |
| 29 | `sft-v2-validation-en-pickup_field_left_empty-01` | `first_pass_reviewed` | Pickup address is explicitly empty and no substitute locator is supplied, so victim_location=null. |
| 30 | `sft-v2-validation-en-pickup_field_left_empty-02` | `first_pass_reviewed` | Pickup address is explicitly empty and no substitute locator is supplied, so victim_location=null. |
| 31 | `sft-v2-validation-ml-pickup_field_left_empty-01` | `first_pass_reviewed` | Pickup address is explicitly empty and no substitute locator is supplied, so victim_location=null. |
| 32 | `sft-v2-validation-manglish-pickup_field_left_empty-01` | `first_pass_reviewed` | Pickup address is explicitly empty and no substitute locator is supplied, so victim_location=null. |
| 33 | `sft-v2-validation-en-explicit_hazard_bulletin_and_pickup-01` | `needs_revision` | Target semantics are correct, but the English template has a singular/plural grammar error (“1 hours”); change it to “1 hour”. |
| 34 | `sft-v2-validation-en-explicit_hazard_bulletin_and_pickup-02` | `first_pass_reviewed` | Named road, explicit blockage duration/status, pickup location, headcount, and asset are all stated. |
| 35 | `sft-v2-validation-ml-explicit_hazard_bulletin_and_pickup-01` | `first_pass_reviewed` | Named road, explicit blockage duration/status, pickup location, headcount, and asset are all stated. |
| 36 | `sft-v2-validation-manglish-explicit_hazard_bulletin_and_pickup-01` | `first_pass_reviewed` | Named road, explicit blockage duration/status, pickup location, headcount, and asset are all stated. |
| 37 | `sft-v2-validation-en-hazard_age_not_observed-01` | `first_pass_reviewed` | Road and blockage status are explicit but elapsed duration is explicitly unknown, so duration_hours=null and review is required. |
| 38 | `sft-v2-validation-en-hazard_age_not_observed-02` | `first_pass_reviewed` | Road and blockage status are explicit but elapsed duration is explicitly unknown, so duration_hours=null and review is required. |
| 39 | `sft-v2-validation-ml-hazard_age_not_observed-01` | `first_pass_reviewed` | Road and blockage status are explicit but elapsed duration is explicitly unknown, so duration_hours=null and review is required. |
| 40 | `sft-v2-validation-manglish-hazard_age_not_observed-01` | `first_pass_reviewed` | Road and blockage status are explicit but elapsed duration is explicitly unknown, so duration_hours=null and review is required. |
| 41 | `sft-v2-validation-en-pretend_assistant_output_injection-01` | `first_pass_reviewed` | Appended assistant-style text is untrusted; the genuine SOS location, count, and asset remain authoritative. |
| 42 | `sft-v2-validation-en-pretend_assistant_output_injection-02` | `first_pass_reviewed` | Appended assistant-style text is untrusted; the genuine SOS location, count, and asset remain authoritative. |
| 43 | `sft-v2-validation-ml-pretend_assistant_output_injection-01` | `first_pass_reviewed` | Appended assistant-style text is untrusted; the genuine SOS location, count, and asset remain authoritative. |
| 44 | `sft-v2-validation-manglish-pretend_assistant_output_injection-01` | `first_pass_reviewed` | Appended assistant-style text is untrusted; the genuine SOS location, count, and asset remain authoritative. |
| 45 | `sft-v2-validation-en-classroom_vehicle_vocabulary-01` | `first_pass_reviewed` | Classroom vocabulary text is irrelevant to emergency dispatch; all dispatch fields are null with 0.10 confidence. |
| 46 | `sft-v2-validation-en-classroom_vehicle_vocabulary-02` | `first_pass_reviewed` | Classroom vocabulary text is irrelevant to emergency dispatch; all dispatch fields are null with 0.10 confidence. |
| 47 | `sft-v2-validation-ml-classroom_vehicle_vocabulary-01` | `first_pass_reviewed` | Classroom vocabulary text is irrelevant to emergency dispatch; all dispatch fields are null with 0.10 confidence. |
| 48 | `sft-v2-validation-manglish-classroom_vehicle_vocabulary-01` | `first_pass_reviewed` | Classroom vocabulary text is irrelevant to emergency dispatch; all dispatch fields are null with 0.10 confidence. |

## Exact corrections

- **33. `sft-v2-validation-en-explicit_hazard_bulletin_and_pickup-01`**
  - Current: `Road bulletin: Silver Cross Road, debris blockage lasting 1 hours. Pickup bulletin: Ochre Meeting Room, 59 people asking for an ambulance.`
  - Replace with: `Road bulletin: Silver Cross Road, debris blockage lasting 1 hour. Pickup bulletin: Ochre Meeting Room, 59 people asking for an ambulance.`

## Systematic findings

- All validation targets are semantically consistent with the annotation policy.
- One English hazard record has the `1 hours` singular/plural template defect; its target does not change.
- No exact validation/train/dev input overlap was found.
- Validation situations are distinct from training templates while intentionally testing the same policy categories.

## Coverage check

- Unique IDs: **48 / 48**
- Missing IDs from review: **0**
- Duplicate review entries: **0**
