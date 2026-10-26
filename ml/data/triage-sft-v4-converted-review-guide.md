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

**SFT-V4 conversion, after owner review.** 372 SFT-V3 records converted to V3 targets; messages and V2 labels unchanged (tested). Review applied: the 12 coordinator-style "We have N people who need …" rows now have relation null (guideline change 5), t273 gold URGENT to match v3.1.0 case 105, and t270 DISABLED kept (guideline change 6). **8 records have profile facts and are listed below**; a marker-word grep confirmed the other 364 are empty.


## train

| ID | Lang | Message | V2 labels | Profile | Gold / Rule | Note | Review |
|---|---|---|---|---|---|---|---|
| t264 | en | We are still here and 19 people need a rescue boat immediately. The voice note gives no place name or usable locator. | loc: None<br>count: 19<br>asset: RESCUE_BOAT<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t269 | manglish | Njangal ivide 15 perundu, ambulance venam. Place name, address, landmark onnum messageil illa. | loc: None<br>count: 15<br>asset: AMBULANCE<br>review: True | rel: SELF<br>on_scene: True | UNKNOWN / UNKNOWN (LOW) |  |  |
| t270 | en | At Briar Hall, 12 residents cannot walk without help and need evacuation. The caller does not request any particular vehicle. | loc: Briar Hall<br>count: 12<br>asset: None<br>review: True | groups: DISABLED | UNKNOWN / UNKNOWN (HIGH) |  |  |
| t271 | en | Four people at Valley Office report chest pain and ask for urgent help, without naming an ambulance or any other transport. | loc: Valley Office<br>count: 4<br>asset: None<br>review: True | med: CARDIAC | CRITICAL / CRITICAL |  |  |
| t273 | en | Seven people are isolated on the roof of Slate House and request assistance. The report never specifies a helicopter, boat, or other asset. | loc: Slate House<br>count: 7<br>asset: None<br>review: True | other: stranded on a roof | URGENT / CRITICAL ⚠ |  |  |
| t274 | ml | സീഡാർ ഹോമിൽ 18 വയോധികർക്ക് നടക്കാൻ ബുദ്ധിമുട്ടുണ്ട്; അടിയന്തരമായി മാറ്റണമെന്ന് പറയുന്നു. പ്രത്യേക വാഹനം ആവശ്യപ്പെട്ടിട്ടില്ല. | loc: സീഡാർ ഹോം<br>count: 18<br>asset: None<br>review: True | groups: ELDERLY | UNKNOWN / UNKNOWN (HIGH) |  |  |
| t275 | manglish | Palm Lodge il 9 injured aalukalkku urgent transport venam, pakshe ethu vehicle venam ennu requestil paranjittilla. | loc: Palm Lodge<br>count: 9<br>asset: None<br>review: True | med: INJURY | URGENT / URGENT |  |  |

## validation

| ID | Lang | Message | V2 labels | Profile | Gold / Rule | Note | Review |
|---|---|---|---|---|---|---|---|
| v057 | en | Ten wheelchair users at Cypress Centre ask for urgent evacuation, but their message does not specify a vehicle. | loc: Cypress Centre<br>count: 10<br>asset: None<br>review: True | groups: DISABLED | UNKNOWN / UNKNOWN (HIGH) |  |  |
