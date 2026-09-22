# CrisisGraph SFT V2 — Training Candidates AI-Assisted First-Pass Review

Scope: all 240 training records, reviewed exactly once in JSONL order. This is an AI-assisted first-pass review, not human/domain-expert review.

## Summary

- `first_pass_reviewed`: **218**
- `needs_revision`: **22**
- total reviewed: **240**

## Phase 1: records 1–60

Accepted: **51**; needs revision: **9**.

| Pos | ID | Verdict | Reason |
|---:|---|---|---|
| 1 | `sft-v2-train-en-rooftop_group_pickup-01` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 2 | `sft-v2-train-en-rooftop_group_pickup-02` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 3 | `sft-v2-train-en-rooftop_group_pickup-03` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 4 | `sft-v2-train-en-rooftop_group_pickup-04` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 5 | `sft-v2-train-en-rooftop_group_pickup-05` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 6 | `sft-v2-train-en-rooftop_group_pickup-06` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 7 | `sft-v2-train-en-rooftop_group_pickup-07` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 8 | `sft-v2-train-en-rooftop_group_pickup-08` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 9 | `sft-v2-train-en-rooftop_group_pickup-09` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 10 | `sft-v2-train-ml-rooftop_group_pickup-01` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 11 | `sft-v2-train-ml-rooftop_group_pickup-02` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 12 | `sft-v2-train-ml-rooftop_group_pickup-03` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 13 | `sft-v2-train-manglish-rooftop_group_pickup-01` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 14 | `sft-v2-train-manglish-rooftop_group_pickup-02` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 15 | `sft-v2-train-manglish-rooftop_group_pickup-03` | `first_pass_reviewed` | Complete explicit pickup: location, headcount, and requested asset are all stated; no hazard or ambiguity. |
| 16 | `sft-v2-train-en-gate_sign_and_room_number-01` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 17 | `sft-v2-train-en-gate_sign_and_room_number-02` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 18 | `sft-v2-train-en-gate_sign_and_room_number-03` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 19 | `sft-v2-train-en-gate_sign_and_room_number-04` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 20 | `sft-v2-train-en-gate_sign_and_room_number-05` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 21 | `sft-v2-train-en-gate_sign_and_room_number-06` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 22 | `sft-v2-train-en-gate_sign_and_room_number-07` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 23 | `sft-v2-train-en-gate_sign_and_room_number-08` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 24 | `sft-v2-train-en-gate_sign_and_room_number-09` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 25 | `sft-v2-train-ml-gate_sign_and_room_number-01` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 26 | `sft-v2-train-ml-gate_sign_and_room_number-02` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 27 | `sft-v2-train-ml-gate_sign_and_room_number-03` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 28 | `sft-v2-train-manglish-gate_sign_and_room_number-01` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 29 | `sft-v2-train-manglish-gate_sign_and_room_number-02` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 30 | `sft-v2-train-manglish-gate_sign_and_room_number-03` | `first_pass_reviewed` | The empty room number is a distractor; the marked entrance is the explicit pickup location, with a clear headcount and asset request. |
| 31 | `sft-v2-train-en-count_conflict_inside_single_call-01` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 32 | `sft-v2-train-en-count_conflict_inside_single_call-02` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 33 | `sft-v2-train-en-count_conflict_inside_single_call-03` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 34 | `sft-v2-train-en-count_conflict_inside_single_call-04` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 35 | `sft-v2-train-en-count_conflict_inside_single_call-05` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 36 | `sft-v2-train-en-count_conflict_inside_single_call-06` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 37 | `sft-v2-train-en-count_conflict_inside_single_call-07` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 38 | `sft-v2-train-en-count_conflict_inside_single_call-08` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 39 | `sft-v2-train-en-count_conflict_inside_single_call-09` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 40 | `sft-v2-train-ml-count_conflict_inside_single_call-01` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 41 | `sft-v2-train-ml-count_conflict_inside_single_call-02` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 42 | `sft-v2-train-ml-count_conflict_inside_single_call-03` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 43 | `sft-v2-train-manglish-count_conflict_inside_single_call-01` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 44 | `sft-v2-train-manglish-count_conflict_inside_single_call-02` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 45 | `sft-v2-train-manglish-count_conflict_inside_single_call-03` | `first_pass_reviewed` | Two exact counts refer to the same current group and neither is withdrawn, so headcount=null while location and asset remain valid. |
| 46 | `sft-v2-train-en-two_occupant_lists_one_vehicle-01` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 47 | `sft-v2-train-en-two_occupant_lists_one_vehicle-02` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 48 | `sft-v2-train-en-two_occupant_lists_one_vehicle-03` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 49 | `sft-v2-train-en-two_occupant_lists_one_vehicle-04` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 50 | `sft-v2-train-en-two_occupant_lists_one_vehicle-05` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 51 | `sft-v2-train-en-two_occupant_lists_one_vehicle-06` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 52 | `sft-v2-train-en-two_occupant_lists_one_vehicle-07` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 53 | `sft-v2-train-en-two_occupant_lists_one_vehicle-08` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 54 | `sft-v2-train-en-two_occupant_lists_one_vehicle-09` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 55 | `sft-v2-train-ml-two_occupant_lists_one_vehicle-01` | `first_pass_reviewed` | Both current lists claim to cover the same full group but disagree on total, so headcount=null; location and asset are explicit. |
| 56 | `sft-v2-train-ml-two_occupant_lists_one_vehicle-02` | `first_pass_reviewed` | Both current lists claim to cover the same full group but disagree on total, so headcount=null; location and asset are explicit. |
| 57 | `sft-v2-train-ml-two_occupant_lists_one_vehicle-03` | `first_pass_reviewed` | Both current lists claim to cover the same full group but disagree on total, so headcount=null; location and asset are explicit. |
| 58 | `sft-v2-train-manglish-two_occupant_lists_one_vehicle-01` | `first_pass_reviewed` | Both current lists claim to cover the same full group but disagree on total, so headcount=null; location and asset are explicit. |
| 59 | `sft-v2-train-manglish-two_occupant_lists_one_vehicle-02` | `first_pass_reviewed` | Both current lists claim to cover the same full group but disagree on total, so headcount=null; location and asset are explicit. |
| 60 | `sft-v2-train-manglish-two_occupant_lists_one_vehicle-03` | `first_pass_reviewed` | Both current lists claim to cover the same full group but disagree on total, so headcount=null; location and asset are explicit. |

### Exact corrections

- **46. `sft-v2-train-en-two_occupant_lists_one_vehicle-01`**
  - Current: `a helicopter is requested at Bamboo Reading Room. Two lists both claim to cover everyone currently waiting: list A totals 24, list B totals 28. Both are marked current.`
  - Replace with: `A helicopter is requested at Bamboo Reading Room. Two lists both claim to cover everyone currently waiting: list A totals 24, list B totals 28. Both are marked current.`
- **47. `sft-v2-train-en-two_occupant_lists_one_vehicle-02`**
  - Current: `an ambulance is requested at Lotus Meeting Hall. Two lists both claim to cover everyone currently waiting: list A totals 27, list B totals 31. Both are marked current.`
  - Replace with: `An ambulance is requested at Lotus Meeting Hall. Two lists both claim to cover everyone currently waiting: list A totals 27, list B totals 31. Both are marked current.`
- **48. `sft-v2-train-en-two_occupant_lists_one_vehicle-03`**
  - Current: `a rescue boat is requested at Palm Crafts Centre. Two lists both claim to cover everyone currently waiting: list A totals 30, list B totals 34. Both are marked current.`
  - Replace with: `A rescue boat is requested at Palm Crafts Centre. Two lists both claim to cover everyone currently waiting: list A totals 30, list B totals 34. Both are marked current.`
- **49. `sft-v2-train-en-two_occupant_lists_one_vehicle-04`**
  - Current: `an evacuation truck is requested at Teak Sports Hall. Two lists both claim to cover everyone currently waiting: list A totals 33, list B totals 37. Both are marked current.`
  - Replace with: `An evacuation truck is requested at Teak Sports Hall. Two lists both claim to cover everyone currently waiting: list A totals 33, list B totals 37. Both are marked current.`
- **50. `sft-v2-train-en-two_occupant_lists_one_vehicle-05`**
  - Current: `a helicopter is requested at Orchid Guest House. Two lists both claim to cover everyone currently waiting: list A totals 36, list B totals 40. Both are marked current.`
  - Replace with: `A helicopter is requested at Orchid Guest House. Two lists both claim to cover everyone currently waiting: list A totals 36, list B totals 40. Both are marked current.`
- **51. `sft-v2-train-en-two_occupant_lists_one_vehicle-06`**
  - Current: `an ambulance is requested at Pine Study Centre. Two lists both claim to cover everyone currently waiting: list A totals 39, list B totals 43. Both are marked current.`
  - Replace with: `An ambulance is requested at Pine Study Centre. Two lists both claim to cover everyone currently waiting: list A totals 39, list B totals 43. Both are marked current.`
- **52. `sft-v2-train-en-two_occupant_lists_one_vehicle-07`**
  - Current: `a rescue boat is requested at Jasmine Rest House. Two lists both claim to cover everyone currently waiting: list A totals 42, list B totals 46. Both are marked current.`
  - Replace with: `A rescue boat is requested at Jasmine Rest House. Two lists both claim to cover everyone currently waiting: list A totals 42, list B totals 46. Both are marked current.`
- **53. `sft-v2-train-en-two_occupant_lists_one_vehicle-08`**
  - Current: `an evacuation truck is requested at Coconut Youth Hall. Two lists both claim to cover everyone currently waiting: list A totals 45, list B totals 49. Both are marked current.`
  - Replace with: `An evacuation truck is requested at Coconut Youth Hall. Two lists both claim to cover everyone currently waiting: list A totals 45, list B totals 49. Both are marked current.`
- **54. `sft-v2-train-en-two_occupant_lists_one_vehicle-09`**
  - Current: `a helicopter is requested at Fig Assembly Hall. Two lists both claim to cover everyone currently waiting: list A totals 48, list B totals 52. Both are marked current.`
  - Replace with: `A helicopter is requested at Fig Assembly Hall. Two lists both claim to cover everyone currently waiting: list A totals 48, list B totals 52. Both are marked current.`

## Phase 2: records 61–120

Accepted: **60**; needs revision: **0**.

| Pos | ID | Verdict | Reason |
|---:|---|---|---|
| 61 | `sft-v2-train-en-caller_cannot_identify_building-01` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 62 | `sft-v2-train-en-caller_cannot_identify_building-02` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 63 | `sft-v2-train-en-caller_cannot_identify_building-03` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 64 | `sft-v2-train-en-caller_cannot_identify_building-04` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 65 | `sft-v2-train-en-caller_cannot_identify_building-05` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 66 | `sft-v2-train-en-caller_cannot_identify_building-06` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 67 | `sft-v2-train-en-caller_cannot_identify_building-07` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 68 | `sft-v2-train-en-caller_cannot_identify_building-08` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 69 | `sft-v2-train-en-caller_cannot_identify_building-09` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 70 | `sft-v2-train-ml-caller_cannot_identify_building-01` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 71 | `sft-v2-train-ml-caller_cannot_identify_building-02` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 72 | `sft-v2-train-ml-caller_cannot_identify_building-03` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 73 | `sft-v2-train-manglish-caller_cannot_identify_building-01` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 74 | `sft-v2-train-manglish-caller_cannot_identify_building-02` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 75 | `sft-v2-train-manglish-caller_cannot_identify_building-03` | `first_pass_reviewed` | Two names compete for the same pickup building and the caller cannot resolve them, so victim_location=null; count and asset remain explicit. |
| 76 | `sft-v2-train-en-mutually_exclusive_vehicle_choices-01` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 77 | `sft-v2-train-en-mutually_exclusive_vehicle_choices-02` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 78 | `sft-v2-train-en-mutually_exclusive_vehicle_choices-03` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 79 | `sft-v2-train-en-mutually_exclusive_vehicle_choices-04` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 80 | `sft-v2-train-en-mutually_exclusive_vehicle_choices-05` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 81 | `sft-v2-train-en-mutually_exclusive_vehicle_choices-06` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 82 | `sft-v2-train-en-mutually_exclusive_vehicle_choices-07` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 83 | `sft-v2-train-en-mutually_exclusive_vehicle_choices-08` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 84 | `sft-v2-train-en-mutually_exclusive_vehicle_choices-09` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 85 | `sft-v2-train-ml-mutually_exclusive_vehicle_choices-01` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 86 | `sft-v2-train-ml-mutually_exclusive_vehicle_choices-02` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 87 | `sft-v2-train-ml-mutually_exclusive_vehicle_choices-03` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 88 | `sft-v2-train-manglish-mutually_exclusive_vehicle_choices-01` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 89 | `sft-v2-train-manglish-mutually_exclusive_vehicle_choices-02` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 90 | `sft-v2-train-manglish-mutually_exclusive_vehicle_choices-03` | `first_pass_reviewed` | Two mutually exclusive assets are offered with no selection, so required_asset=null; location and headcount are explicit. |
| 91 | `sft-v2-train-en-coordinators_disagree_on_transport-01` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 92 | `sft-v2-train-en-coordinators_disagree_on_transport-02` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 93 | `sft-v2-train-en-coordinators_disagree_on_transport-03` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 94 | `sft-v2-train-en-coordinators_disagree_on_transport-04` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 95 | `sft-v2-train-en-coordinators_disagree_on_transport-05` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 96 | `sft-v2-train-en-coordinators_disagree_on_transport-06` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 97 | `sft-v2-train-en-coordinators_disagree_on_transport-07` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 98 | `sft-v2-train-en-coordinators_disagree_on_transport-08` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 99 | `sft-v2-train-en-coordinators_disagree_on_transport-09` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 100 | `sft-v2-train-ml-coordinators_disagree_on_transport-01` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 101 | `sft-v2-train-ml-coordinators_disagree_on_transport-02` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 102 | `sft-v2-train-ml-coordinators_disagree_on_transport-03` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 103 | `sft-v2-train-manglish-coordinators_disagree_on_transport-01` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 104 | `sft-v2-train-manglish-coordinators_disagree_on_transport-02` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 105 | `sft-v2-train-manglish-coordinators_disagree_on_transport-03` | `first_pass_reviewed` | Two coordinators make unresolved competing asset requests for the same pickup, so required_asset=null. |
| 106 | `sft-v2-train-en-estimated_crowd_not_counted-01` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 107 | `sft-v2-train-en-estimated_crowd_not_counted-02` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 108 | `sft-v2-train-en-estimated_crowd_not_counted-03` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 109 | `sft-v2-train-en-estimated_crowd_not_counted-04` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 110 | `sft-v2-train-en-estimated_crowd_not_counted-05` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 111 | `sft-v2-train-en-estimated_crowd_not_counted-06` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 112 | `sft-v2-train-en-estimated_crowd_not_counted-07` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 113 | `sft-v2-train-en-estimated_crowd_not_counted-08` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 114 | `sft-v2-train-en-estimated_crowd_not_counted-09` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 115 | `sft-v2-train-ml-estimated_crowd_not_counted-01` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 116 | `sft-v2-train-ml-estimated_crowd_not_counted-02` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 117 | `sft-v2-train-ml-estimated_crowd_not_counted-03` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 118 | `sft-v2-train-manglish-estimated_crowd_not_counted-01` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 119 | `sft-v2-train-manglish-estimated_crowd_not_counted-02` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |
| 120 | `sft-v2-train-manglish-estimated_crowd_not_counted-03` | `first_pass_reviewed` | The stated crowd size is explicitly an unconfirmed estimate, so headcount=null; location and asset are explicit. |

## Phase 3: records 121–180

Accepted: **49**; needs revision: **11**.

| Pos | ID | Verdict | Reason |
|---:|---|---|---|
| 121 | `sft-v2-train-en-transport_needed_without_vehicle_name-01` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 122 | `sft-v2-train-en-transport_needed_without_vehicle_name-02` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 123 | `sft-v2-train-en-transport_needed_without_vehicle_name-03` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 124 | `sft-v2-train-en-transport_needed_without_vehicle_name-04` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 125 | `sft-v2-train-en-transport_needed_without_vehicle_name-05` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 126 | `sft-v2-train-en-transport_needed_without_vehicle_name-06` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 127 | `sft-v2-train-en-transport_needed_without_vehicle_name-07` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 128 | `sft-v2-train-en-transport_needed_without_vehicle_name-08` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 129 | `sft-v2-train-en-transport_needed_without_vehicle_name-09` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 130 | `sft-v2-train-ml-transport_needed_without_vehicle_name-01` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 131 | `sft-v2-train-ml-transport_needed_without_vehicle_name-02` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 132 | `sft-v2-train-ml-transport_needed_without_vehicle_name-03` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 133 | `sft-v2-train-manglish-transport_needed_without_vehicle_name-01` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 134 | `sft-v2-train-manglish-transport_needed_without_vehicle_name-02` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 135 | `sft-v2-train-manglish-transport_needed_without_vehicle_name-03` | `first_pass_reviewed` | Transport is needed but no specific permitted asset is requested, so required_asset=null. |
| 136 | `sft-v2-train-en-crowd_hidden_from_reporter-01` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 137 | `sft-v2-train-en-crowd_hidden_from_reporter-02` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 138 | `sft-v2-train-en-crowd_hidden_from_reporter-03` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 139 | `sft-v2-train-en-crowd_hidden_from_reporter-04` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 140 | `sft-v2-train-en-crowd_hidden_from_reporter-05` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 141 | `sft-v2-train-en-crowd_hidden_from_reporter-06` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 142 | `sft-v2-train-en-crowd_hidden_from_reporter-07` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 143 | `sft-v2-train-en-crowd_hidden_from_reporter-08` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 144 | `sft-v2-train-en-crowd_hidden_from_reporter-09` | `needs_revision` | Target semantics are correct, but the synthetic English input starts with a lowercase sentence-initial article; capitalize the first character. |
| 145 | `sft-v2-train-ml-crowd_hidden_from_reporter-01` | `first_pass_reviewed` | People are present but no number is observable or stated, so headcount=null; location and asset remain explicit. |
| 146 | `sft-v2-train-ml-crowd_hidden_from_reporter-02` | `first_pass_reviewed` | People are present but no number is observable or stated, so headcount=null; location and asset remain explicit. |
| 147 | `sft-v2-train-ml-crowd_hidden_from_reporter-03` | `first_pass_reviewed` | People are present but no number is observable or stated, so headcount=null; location and asset remain explicit. |
| 148 | `sft-v2-train-manglish-crowd_hidden_from_reporter-01` | `first_pass_reviewed` | People are present but no number is observable or stated, so headcount=null; location and asset remain explicit. |
| 149 | `sft-v2-train-manglish-crowd_hidden_from_reporter-02` | `first_pass_reviewed` | People are present but no number is observable or stated, so headcount=null; location and asset remain explicit. |
| 150 | `sft-v2-train-manglish-crowd_hidden_from_reporter-03` | `first_pass_reviewed` | People are present but no number is observable or stated, so headcount=null; location and asset remain explicit. |
| 151 | `sft-v2-train-en-caller_only_says_here-01` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 152 | `sft-v2-train-en-caller_only_says_here-02` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 153 | `sft-v2-train-en-caller_only_says_here-03` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 154 | `sft-v2-train-en-caller_only_says_here-04` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 155 | `sft-v2-train-en-caller_only_says_here-05` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 156 | `sft-v2-train-en-caller_only_says_here-06` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 157 | `sft-v2-train-en-caller_only_says_here-07` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 158 | `sft-v2-train-en-caller_only_says_here-08` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 159 | `sft-v2-train-en-caller_only_says_here-09` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 160 | `sft-v2-train-ml-caller_only_says_here-01` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 161 | `sft-v2-train-ml-caller_only_says_here-02` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 162 | `sft-v2-train-ml-caller_only_says_here-03` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 163 | `sft-v2-train-manglish-caller_only_says_here-01` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 164 | `sft-v2-train-manglish-caller_only_says_here-02` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 165 | `sft-v2-train-manglish-caller_only_says_here-03` | `first_pass_reviewed` | The caller provides only deictic “here” and explicitly lacks an address/landmark, so victim_location=null. |
| 166 | `sft-v2-train-en-named_road_debris_and_waiting_group-01` | `needs_revision` | Target semantics are correct, but the English template has a singular/plural grammar error (“1 hours”); change it to “1 hour”. |
| 167 | `sft-v2-train-en-named_road_debris_and_waiting_group-02` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 168 | `sft-v2-train-en-named_road_debris_and_waiting_group-03` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 169 | `sft-v2-train-en-named_road_debris_and_waiting_group-04` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 170 | `sft-v2-train-en-named_road_debris_and_waiting_group-05` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 171 | `sft-v2-train-en-named_road_debris_and_waiting_group-06` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 172 | `sft-v2-train-en-named_road_debris_and_waiting_group-07` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 173 | `sft-v2-train-en-named_road_debris_and_waiting_group-08` | `needs_revision` | Target semantics are correct, but the English template has a singular/plural grammar error (“1 hours”); change it to “1 hour”. |
| 174 | `sft-v2-train-en-named_road_debris_and_waiting_group-09` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 175 | `sft-v2-train-ml-named_road_debris_and_waiting_group-01` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 176 | `sft-v2-train-ml-named_road_debris_and_waiting_group-02` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 177 | `sft-v2-train-ml-named_road_debris_and_waiting_group-03` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 178 | `sft-v2-train-manglish-named_road_debris_and_waiting_group-01` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 179 | `sft-v2-train-manglish-named_road_debris_and_waiting_group-02` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |
| 180 | `sft-v2-train-manglish-named_road_debris_and_waiting_group-03` | `first_pass_reviewed` | Named road, blockage status, elapsed duration, pickup location, headcount, and requested asset are explicit and correctly separated. |

### Exact corrections

- **136. `sft-v2-train-en-crowd_hidden_from_reporter-01`**
  - Current: `a rescue boat is needed at Jasmine Rest House. People are waiting behind the compound wall, and I cannot see how many there are.`
  - Replace with: `A rescue boat is needed at Jasmine Rest House. People are waiting behind the compound wall, and I cannot see how many there are.`
- **137. `sft-v2-train-en-crowd_hidden_from_reporter-02`**
  - Current: `an evacuation truck is needed at Coconut Youth Hall. People are waiting behind the compound wall, and I cannot see how many there are.`
  - Replace with: `An evacuation truck is needed at Coconut Youth Hall. People are waiting behind the compound wall, and I cannot see how many there are.`
- **138. `sft-v2-train-en-crowd_hidden_from_reporter-03`**
  - Current: `a helicopter is needed at Fig Assembly Hall. People are waiting behind the compound wall, and I cannot see how many there are.`
  - Replace with: `A helicopter is needed at Fig Assembly Hall. People are waiting behind the compound wall, and I cannot see how many there are.`
- **139. `sft-v2-train-en-crowd_hidden_from_reporter-04`**
  - Current: `an ambulance is needed at Larch Rest Hall. People are waiting behind the compound wall, and I cannot see how many there are.`
  - Replace with: `An ambulance is needed at Larch Rest Hall. People are waiting behind the compound wall, and I cannot see how many there are.`
- **140. `sft-v2-train-en-crowd_hidden_from_reporter-05`**
  - Current: `a rescue boat is needed at Mango Workers Hostel. People are waiting behind the compound wall, and I cannot see how many there are.`
  - Replace with: `A rescue boat is needed at Mango Workers Hostel. People are waiting behind the compound wall, and I cannot see how many there are.`
- **141. `sft-v2-train-en-crowd_hidden_from_reporter-06`**
  - Current: `an evacuation truck is needed at Rose Training Centre. People are waiting behind the compound wall, and I cannot see how many there are.`
  - Replace with: `An evacuation truck is needed at Rose Training Centre. People are waiting behind the compound wall, and I cannot see how many there are.`
- **142. `sft-v2-train-en-crowd_hidden_from_reporter-07`**
  - Current: `a helicopter is needed at Bamboo Reading Room. People are waiting behind the compound wall, and I cannot see how many there are.`
  - Replace with: `A helicopter is needed at Bamboo Reading Room. People are waiting behind the compound wall, and I cannot see how many there are.`
- **143. `sft-v2-train-en-crowd_hidden_from_reporter-08`**
  - Current: `an ambulance is needed at Lotus Meeting Hall. People are waiting behind the compound wall, and I cannot see how many there are.`
  - Replace with: `An ambulance is needed at Lotus Meeting Hall. People are waiting behind the compound wall, and I cannot see how many there are.`
- **144. `sft-v2-train-en-crowd_hidden_from_reporter-09`**
  - Current: `a rescue boat is needed at Palm Crafts Centre. People are waiting behind the compound wall, and I cannot see how many there are.`
  - Replace with: `A rescue boat is needed at Palm Crafts Centre. People are waiting behind the compound wall, and I cannot see how many there are.`
- **166. `sft-v2-train-en-named_road_debris_and_waiting_group-01`**
  - Current: `Larch Link Road has been blocked by debris for 1 hours. Separately, 80 people waiting at Fig Assembly Hall request a helicopter.`
  - Replace with: `Larch Link Road has been blocked by debris for 1 hour. Separately, 80 people waiting at Fig Assembly Hall request a helicopter.`
- **173. `sft-v2-train-en-named_road_debris_and_waiting_group-08`**
  - Current: `Larch Link Road has been blocked by debris for 1 hours. Separately, 18 people waiting at Teak Sports Hall request an evacuation truck.`
  - Replace with: `Larch Link Road has been blocked by debris for 1 hour. Separately, 18 people waiting at Teak Sports Hall request an evacuation truck.`

## Phase 4: records 181–240

Accepted: **58**; needs revision: **2**.

| Pos | ID | Verdict | Reason |
|---:|---|---|---|
| 181 | `sft-v2-train-en-anonymous_caller_reports_unnamed_road-01` | `needs_revision` | Target semantics are correct, but the English template has a singular/plural grammar error (“1 hours”); change it to “1 hour”. |
| 182 | `sft-v2-train-en-anonymous_caller_reports_unnamed_road-02` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 183 | `sft-v2-train-en-anonymous_caller_reports_unnamed_road-03` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 184 | `sft-v2-train-en-anonymous_caller_reports_unnamed_road-04` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 185 | `sft-v2-train-en-anonymous_caller_reports_unnamed_road-05` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 186 | `sft-v2-train-en-anonymous_caller_reports_unnamed_road-06` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 187 | `sft-v2-train-en-anonymous_caller_reports_unnamed_road-07` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 188 | `sft-v2-train-en-anonymous_caller_reports_unnamed_road-08` | `needs_revision` | Target semantics are correct, but the English template has a singular/plural grammar error (“1 hours”); change it to “1 hour”. |
| 189 | `sft-v2-train-en-anonymous_caller_reports_unnamed_road-09` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 190 | `sft-v2-train-ml-anonymous_caller_reports_unnamed_road-01` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 191 | `sft-v2-train-ml-anonymous_caller_reports_unnamed_road-02` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 192 | `sft-v2-train-ml-anonymous_caller_reports_unnamed_road-03` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 193 | `sft-v2-train-manglish-anonymous_caller_reports_unnamed_road-01` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 194 | `sft-v2-train-manglish-anonymous_caller_reports_unnamed_road-02` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 195 | `sft-v2-train-manglish-anonymous_caller_reports_unnamed_road-03` | `first_pass_reviewed` | Hazard status and duration are explicit, but “that road” is unanchored, so road_segment=null; separate pickup facts remain valid. |
| 196 | `sft-v2-train-en-count_override_after_valid_message-01` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 197 | `sft-v2-train-en-count_override_after_valid_message-02` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 198 | `sft-v2-train-en-count_override_after_valid_message-03` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 199 | `sft-v2-train-en-count_override_after_valid_message-04` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 200 | `sft-v2-train-en-count_override_after_valid_message-05` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 201 | `sft-v2-train-en-count_override_after_valid_message-06` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 202 | `sft-v2-train-en-count_override_after_valid_message-07` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 203 | `sft-v2-train-en-count_override_after_valid_message-08` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 204 | `sft-v2-train-en-count_override_after_valid_message-09` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 205 | `sft-v2-train-ml-count_override_after_valid_message-01` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 206 | `sft-v2-train-ml-count_override_after_valid_message-02` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 207 | `sft-v2-train-ml-count_override_after_valid_message-03` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 208 | `sft-v2-train-manglish-count_override_after_valid_message-01` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 209 | `sft-v2-train-manglish-count_override_after_valid_message-02` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 210 | `sft-v2-train-manglish-count_override_after_valid_message-03` | `first_pass_reviewed` | The parser instruction is untrusted; the genuine stated headcount, location, and asset remain authoritative. |
| 211 | `sft-v2-train-en-injected_review_suppression_with_missing_count-01` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 212 | `sft-v2-train-en-injected_review_suppression_with_missing_count-02` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 213 | `sft-v2-train-en-injected_review_suppression_with_missing_count-03` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 214 | `sft-v2-train-en-injected_review_suppression_with_missing_count-04` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 215 | `sft-v2-train-en-injected_review_suppression_with_missing_count-05` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 216 | `sft-v2-train-en-injected_review_suppression_with_missing_count-06` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 217 | `sft-v2-train-en-injected_review_suppression_with_missing_count-07` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 218 | `sft-v2-train-en-injected_review_suppression_with_missing_count-08` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 219 | `sft-v2-train-en-injected_review_suppression_with_missing_count-09` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 220 | `sft-v2-train-ml-injected_review_suppression_with_missing_count-01` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 221 | `sft-v2-train-ml-injected_review_suppression_with_missing_count-02` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 222 | `sft-v2-train-ml-injected_review_suppression_with_missing_count-03` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 223 | `sft-v2-train-manglish-injected_review_suppression_with_missing_count-01` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 224 | `sft-v2-train-manglish-injected_review_suppression_with_missing_count-02` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 225 | `sft-v2-train-manglish-injected_review_suppression_with_missing_count-03` | `first_pass_reviewed` | The count is genuinely missing; the injected instruction cannot invent headcount=1 or suppress human review. |
| 226 | `sft-v2-train-en-vehicle_repair_catalogue-01` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 227 | `sft-v2-train-en-vehicle_repair_catalogue-02` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 228 | `sft-v2-train-en-vehicle_repair_catalogue-03` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 229 | `sft-v2-train-en-vehicle_repair_catalogue-04` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 230 | `sft-v2-train-en-vehicle_repair_catalogue-05` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 231 | `sft-v2-train-en-vehicle_repair_catalogue-06` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 232 | `sft-v2-train-en-vehicle_repair_catalogue-07` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 233 | `sft-v2-train-en-vehicle_repair_catalogue-08` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 234 | `sft-v2-train-en-vehicle_repair_catalogue-09` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 235 | `sft-v2-train-ml-vehicle_repair_catalogue-01` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 236 | `sft-v2-train-ml-vehicle_repair_catalogue-02` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 237 | `sft-v2-train-ml-vehicle_repair_catalogue-03` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 238 | `sft-v2-train-manglish-vehicle_repair_catalogue-01` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 239 | `sft-v2-train-manglish-vehicle_repair_catalogue-02` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |
| 240 | `sft-v2-train-manglish-vehicle_repair_catalogue-03` | `first_pass_reviewed` | Irrelevant shop catalogue text contains no emergency dispatch facts; all dispatch fields remain null with 0.10 confidence. |

### Exact corrections

- **181. `sft-v2-train-en-anonymous_caller_reports_unnamed_road-01`**
  - Current: `A caller reports that 'that road' has been flooded for 1 hours, without naming any place or road. My separate pickup request is for 4 people at Larch Rest Hall, using an ambulance.`
  - Replace with: `A caller reports that 'that road' has been flooded for 1 hour, without naming any place or road. My separate pickup request is for 4 people at Larch Rest Hall, using an ambulance.`
- **188. `sft-v2-train-en-anonymous_caller_reports_unnamed_road-08`**
  - Current: `A caller reports that 'that road' has been flooded for 1 hours, without naming any place or road. My separate pickup request is for 25 people at Orchid Guest House, using a helicopter.`
  - Replace with: `A caller reports that 'that road' has been flooded for 1 hour, without naming any place or road. My separate pickup request is for 25 people at Orchid Guest House, using a helicopter.`

## Systematic findings

- Target semantics and policy-state consistency passed across all 240 records: non-null extracted facts are explicit in the corresponding input, asset enums are valid, and confidence/review labels follow the documented mapping.
- Two English templates have a sentence-initial capitalization defect, affecting 18 records total: `two_occupant_lists_one_vehicle` and `crowd_hidden_from_reporter`. Targets do not change.
- Two hazard families render `1 hours` when duration is 1, affecting 4 training records. Change only the input wording to `1 hour`; targets remain unchanged.
- No exact train/validation/dev input overlap was found. Cross-split similarity is category-level rather than near-duplicate wording.
- Repetition remains structurally high by design: 15 slot-substitution variants per training family. This is a dataset limitation, not a per-record target defect.

## Coverage check

- Unique IDs: **240 / 240**
- Missing IDs from review: **0**
- Duplicate review entries: **0**
