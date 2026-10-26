# SFT-V4 converted records: review sheet

Review each case against `triage-annotation-guidelines.md` and
`triage-annotation-guidelines-v3.1-changes.md`. Mark **OK**, or write the
correction in the last column (e.g. `water: CHEST`, `groups: +ELDERLY`,
`gold: URGENT`). Corrections are applied to `src/crisisgraph_ml/triage_sft_v4_convert.py`; do not edit the JSONL
by hand.

- **Profile** lists only non-empty fields; anything missing is null / [].
- **Gold** is the annotator's clinical-sense urgency; **Rule** is what urgency-rules-v3
  gives on these labels. ⚠ marks a mismatch (rule lower than gold is a
  potential under-triage; rule higher is over-triage).
- **Note** is the annotator's own comment, if any.

**SFT-V4 conversion.** 372 SFT-V3 records (300 train + 72 validation) converted to V3 targets. Messages and V2 labels are unchanged and already reviewed (a test checks they match SFT-V3 exactly), so only the profile labels need review. **20 records have profile facts and are listed below.** The other 352 are templated dispatch requests with no stated facts about the people and get an empty profile; spot-check them in `triage-sft-v4-converted-candidates.jsonl` if you like. `suggested_urgency` in each target equals the gold urgency, or null when gold is UNKNOWN.


## train

| ID | Lang | Message | V2 labels | Profile | Gold / Rule | Note | Review |
|---|---|---|---|---|---|---|---|
| t005 | en | We have 37 people who urgently need a rescue boat. | loc: None<br>count: 37<br>asset: RESCUE_BOAT<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t017 | en | We have 24 people who urgently need an evacuation truck. | loc: None<br>count: 24<br>asset: EVAC_TRUCK<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t029 | en | We have 11 people who urgently need a helicopter. | loc: None<br>count: 11<br>asset: HELICOPTER<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t041 | en | We have 95 people who urgently need an ambulance. | loc: None<br>count: 95<br>asset: AMBULANCE<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t053 | en | We have 82 people who urgently need a rescue boat. | loc: None<br>count: 82<br>asset: RESCUE_BOAT<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t065 | en | We have 69 people who urgently need an evacuation truck. | loc: None<br>count: 69<br>asset: EVAC_TRUCK<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t077 | en | We have 56 people who urgently need a helicopter. | loc: None<br>count: 56<br>asset: HELICOPTER<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t089 | en | We have 43 people who urgently need an ambulance. | loc: None<br>count: 43<br>asset: AMBULANCE<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t101 | en | We have 30 people who urgently need a rescue boat. | loc: None<br>count: 30<br>asset: RESCUE_BOAT<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t113 | en | We have 17 people who urgently need an evacuation truck. | loc: None<br>count: 17<br>asset: EVAC_TRUCK<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t125 | en | We have 4 people who urgently need a helicopter. | loc: None<br>count: 4<br>asset: HELICOPTER<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t137 | en | We have 88 people who urgently need an ambulance. | loc: None<br>count: 88<br>asset: AMBULANCE<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t264 | en | We are still here and 19 people need a rescue boat immediately. The voice note gives no place name or usable locator. | loc: None<br>count: 19<br>asset: RESCUE_BOAT<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t269 | manglish | Njangal ivide 15 perundu, ambulance venam. Place name, address, landmark onnum messageil illa. | loc: None<br>count: 15<br>asset: AMBULANCE<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t270 | en | At Briar Hall, 12 residents cannot walk without help and need evacuation. The caller does not request any particular vehicle. | loc: Briar Hall<br>count: 12<br>asset: None<br>review: True | groups: DISABLED | UNKNOWN / UNKNOWN (HIGH) | Flagged: 'cannot walk without help' read as a lasting disability (DISABLED); the guideline excludes 'cannot walk' from injury or illness, and no cause is stated. |  |
| t271 | en | Four people at Valley Office report chest pain and ask for urgent help, without naming an ambulance or any other transport. | loc: Valley Office<br>count: 4<br>asset: None<br>review: True | med: CARDIAC | CRITICAL / CRITICAL |  |  |
| t273 | en | Seven people are isolated on the roof of Slate House and request assistance. The report never specifies a helicopter, boat, or other asset. | loc: Slate House<br>count: 7<br>asset: None<br>review: True | other: stranded on a roof | CRITICAL / CRITICAL |  |  |
| t274 | ml | സീഡാർ ഹോമിൽ 18 വയോധികർക്ക് നടക്കാൻ ബുദ്ധിമുട്ടുണ്ട്; അടിയന്തരമായി മാറ്റണമെന്ന് പറയുന്നു. പ്രത്യേക വാഹനം ആവശ്യപ്പെട്ടിട്ടില്ല. | loc: സീഡാർ ഹോം<br>count: 18<br>asset: None<br>review: True | groups: ELDERLY | UNKNOWN / UNKNOWN (HIGH) |  |  |
| t275 | manglish | Palm Lodge il 9 injured aalukalkku urgent transport venam, pakshe ethu vehicle venam ennu requestil paranjittilla. | loc: Palm Lodge<br>count: 9<br>asset: None<br>review: True | med: INJURY | URGENT / URGENT |  |  |

## validation

| ID | Lang | Message | V2 labels | Profile | Gold / Rule | Note | Review |
|---|---|---|---|---|---|---|---|
| v057 | en | Ten wheelchair users at Cypress Centre ask for urgent evacuation, but their message does not specify a vehicle. | loc: Cypress Centre<br>count: 10<br>asset: None<br>review: True | groups: DISABLED | UNKNOWN / UNKNOWN (HIGH) |  |  |
