"""Generate and validate the targeted CrisisGraph SFT V3 candidate additions."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator

from crisisgraph_ml.triage_eval import DEFAULT_SCHEMA, load_cases, load_json
from crisisgraph_ml.triage_sft_data import ML_ROOT, normalize_text

DATA = ML_ROOT / "data"
OUTPUTS = {
    "train_augmentation": DATA / "triage-sft-v3-train-augmentation-candidates.jsonl",
    "validation_extension": DATA / "triage-sft-v3-validation-extension-candidates.jsonl",
}
EXPECTED_LANGUAGES = {
    "train_augmentation": {"en": 36, "ml": 12, "manglish": 12},
    "validation_extension": {"en": 12, "ml": 6, "manglish": 6},
}
EXPECTED_CATEGORIES = {
    "train_augmentation": {
        "contradictory_count": 6,
        "contradictory_location": 6,
        "contradictory_asset": 6,
        "uncertain_headcount": 6,
        "missing_location": 6,
        "implicit_asset_only": 6,
        "hazard_unknown_status": 6,
        "hazard_missing_duration": 6,
        "hazard_unanchored_road": 6,
        "complete_boundary": 6,
    },
    "validation_extension": {
        "contradictory_count": 3,
        "contradictory_location": 3,
        "contradictory_asset": 3,
        "missing_asset": 1,
        "missing_location": 1,
        "uncertain_headcount": 1,
        "hazard_unknown_status": 3,
        "hazard_missing_duration": 3,
        "hazard_unanchored_road": 3,
        "complete_boundary": 3,
    },
}
ROW_SPLITS = {"train_augmentation": "train", "validation_extension": "validation"}
ASSETS = {"AMBULANCE", "RESCUE_BOAT", "EVAC_TRUCK", "HELICOPTER"}


def h(road: str | None, status: str | None, hours: int | None) -> dict[str, Any]:
    return {"road_segment": road, "status": status, "duration_hours": hours}


def add(
    rows: list[dict[str, Any]], split: str, category: str, language: str,
    family: str, text: str, location: str | None, count: int | None,
    asset: str | None, hazards: list[dict[str, Any]], reason: str | None,
) -> None:
    number = 1 + sum(r["language"] == language and r["category"] == category for r in rows)
    review = reason is not None
    rows.append({
        "id": f"sft-v3-{split}-{language}-{category}-{number:02d}",
        "split": split,
        "category": category,
        "language": language,
        "scenario_family": family,
        "wording_family": f"{family}-wording",
        "provenance": "ai_assisted_synthetic_crisisgraph_sft_v3_targeted",
        "reviewer_status": "pending_manual_review",
        "input": text,
        "target": {
            "schema_version": "triage-extraction-v2",
            "victim_location": location,
            "headcount": count,
            "required_asset": asset,
            "hazards": hazards,
            "confidence_score": 0.55 if review else 0.95,
            "needs_human_review": review,
            "uncertainty_reasons": [reason] if reason else [],
        },
    })


def train_rows() -> list[dict[str, Any]]:
    r: list[dict[str, Any]] = []
    # Four independently worded English situations plus one Malayalam and one
    # Manglish situation per subtype. No authored text is reused.
    specs = [
        ("contradictory_count", "en", "quartz-depot-gate-ledger", "Gate control counted 41 evacuees at Quartz Depot, while the bus ledger lists 36 for the same waiting group. Neither tally is signed off. Send an evacuation truck.", "Quartz Depot", None, "EVAC_TRUCK", [], "Conflicting headcounts were reported for the same group"),
        ("contradictory_count", "en", "willow-centre-radio-tallies", "Two simultaneous radio calls concern everyone at Willow Activity Centre: one reports 18 people and the other 23. The coordinator has not verified either count. An ambulance is requested.", "Willow Activity Centre", None, "AMBULANCE", [], "Conflicting headcounts were reported for the same group"),
        ("contradictory_count", "en", "harbour-annex-whiteboard-list", "At Harbour Annex, the whiteboard says 52 people need rescue, but the current shelter list says 49 for that identical group. Please send a rescue boat; the correct total is unresolved.", "Harbour Annex", None, "RESCUE_BOAT", [], "Conflicting headcounts were reported for the same group"),
        ("contradictory_count", "ml", "cedar-clinic-team-reports", "സീഡാർ ക്ലിനിക്കിൽ ഹെലികോപ്റ്റർ വേണം. കാത്തിരിക്കുന്ന മുഴുവൻ രോഗികളുടെയും എണ്ണം ടീം എ 11 എന്നും ടീം ബി 15 എന്നും റിപ്പോർട്ട് ചെയ്യുന്നു. അന്തിമ കണക്ക് ലഭ്യമല്ല.", "സീഡാർ ക്ലിനിക്ക്", None, "HELICOPTER", [], "Conflicting headcounts were reported for the same group"),
        ("contradictory_count", "ml", "pearl-school-conflicting-registers", "പേൾ സ്കൂളിൽ കാത്തിരിക്കുന്ന ഒരേ സംഘത്തിന് വാർഡ് രജിസ്റ്ററിൽ 27 പേരെന്നും റേഡിയോ റിപ്പോർട്ടിൽ 31 പേരെന്നും പറയുന്നു. ശരിയായ എണ്ണം സ്ഥിരീകരിച്ചിട്ടില്ല. രക്ഷാബോട്ട് വേണം.", "പേൾ സ്കൂൾ", None, "RESCUE_BOAT", [], "Conflicting headcounts were reported for the same group"),
        ("contradictory_count", "manglish", "maple-hall-double-tally", "Maple Hall il wait cheyyunna same groupine kurichu oru tally 20 ennum vere tally 24 ennum aanu. Final count verify cheythittilla; ambulance venam.", "Maple Hall", None, "AMBULANCE", [], "Conflicting headcounts were reported for the same group"),

        ("contradictory_location", "en", "linen-factory-pickup-conflict", "Nine workers need an ambulance, but the incident form names Linen Factory Gate and the radio operator names Cotton Store Yard as their pickup point. Neither location has priority.", None, 9, "AMBULANCE", [], "Conflicting pickup locations were reported"),
        ("contradictory_location", "en", "student-group-two-venues", "A rescue boat is requested for 26 students. One current message places the same group at Fern Tutorial Hall; another places them at Cypress Library. Dispatch has not resolved the venue.", None, 26, "RESCUE_BOAT", [], "Conflicting pickup locations were reported"),
        ("contradictory_location", "en", "clinic-transfer-address-mismatch", "The transfer sheet sends an evacuation truck for 13 people to Dawn Clinic, while its footer gives Meadow Dispensary as pickup for those same people. Staff cannot confirm which address is correct.", None, 13, "EVAC_TRUCK", [], "Conflicting pickup locations were reported"),
        ("contradictory_location", "manglish", "roof-team-coordinate-dispute", "Aaru perku helicopter venam. Roof team Saffron Apartments aanu location ennu parayunnu, command recordil pickup Clay Towers aanu; randum same teamine kurichaanu, correct place verify cheythittilla.", None, 6, "HELICOPTER", [], "Conflicting pickup locations were reported"),
        ("contradictory_location", "ml", "community-group-location-dispute", "17 പേർക്ക് ഒഴിപ്പിക്കൽ ട്രക്ക് വേണം. ഒരേ സംഘത്തിന്റെ സ്ഥലം നീല കമ്മ്യൂണിറ്റി ഹാൾ എന്നും ജാസ്മിൻ വായനശാല എന്നും രണ്ട് സന്ദേശങ്ങളിൽ പറയുന്നു; ശരിയായ സ്ഥലം ഉറപ്പായിട്ടില്ല.", None, 17, "EVAC_TRUCK", [], "Conflicting pickup locations were reported"),
        ("contradictory_location", "manglish", "hostel-group-pickup-mismatch", "12 perulla same groupinu pickup Pine Hostel ennum River Study Centre ennum randu update vannu. Correct location settle aayittilla. Rescue boat venam.", None, 12, "RESCUE_BOAT", [], "Conflicting pickup locations were reported"),

        ("contradictory_asset", "en", "orchid-home-request-channel-conflict", "Orchid Care Home has 21 people at pickup. Its phone request asks for an ambulance, but the signed note asks for a helicopter; operations has not chosen between them.", "Orchid Care Home", 21, None, [], "Conflicting assets were requested"),
        ("contradictory_asset", "en", "timber-yard-two-vehicle-orders", "For 34 people at Timber Yard, the field lead requests an evacuation truck while the control-room entry requests a rescue boat. Both orders are current and unresolved.", "Timber Yard", 34, None, [], "Conflicting assets were requested"),
        ("contradictory_asset", "en", "rainbow-school-form-radio-assets", "Eight evacuees are at Rainbow School. The form selects helicopter, whereas the radio call explicitly requests ambulance. No corrected vehicle request has arrived.", "Rainbow School", 8, None, [], "Conflicting assets were requested"),
        ("contradictory_asset", "en", "canal-lodge-dispatch-choice", "Canal Lodge reports 16 stranded residents. One dispatcher was told to send a rescue boat and another was told to send an evacuation truck; the required asset is not confirmed.", "Canal Lodge", 16, None, [], "Conflicting assets were requested"),
        ("contradictory_asset", "ml", "copper-pavilion-asset-conflict", "കോപ്പർ പവിലിയനിൽ 25 പേരുണ്ട്. റേഡിയോ സന്ദേശത്തിൽ ആംബുലൻസും എഴുത്തുപരമായ അപേക്ഷയിൽ ഹെലികോപ്റ്ററും ആവശ്യപ്പെടുന്നു. ഏത് വാഹനമാണെന്ന് സ്ഥിരീകരിച്ചിട്ടില്ല.", "കോപ്പർ പവിലിയൻ", 25, None, [], "Conflicting assets were requested"),
        ("contradictory_asset", "manglish", "lotus-warehouse-vehicle-conflict", "Lotus Warehouse il 14 perundu. Site message rescue boat chodikkunnu, control note evacuation truck chodikkunnu; final asset confirm aayittilla.", "Lotus Warehouse", 14, None, [], "Conflicting assets were requested"),

        ("uncertain_headcount", "en", "almond-shed-estimated-range", "An ambulance is required at Almond Shed. The caller estimates between 12 and 18 people but says no one has completed a count.", "Almond Shed", None, "AMBULANCE", [], "Only an unverified headcount range was reported"),
        ("uncertain_headcount", "en", "granary-about-thirty", "Please send an evacuation truck to East Granary. There may be about 30 people inside; that figure is only a neighbour's estimate.", "East Granary", None, "EVAC_TRUCK", [], "The headcount was approximate and unverified"),
        ("uncertain_headcount", "en", "terrace-at-least-ten", "A rescue boat is requested at Moss Terrace. The report says at least ten residents are waiting, without giving a complete total.", "Moss Terrace", None, "RESCUE_BOAT", [], "Only a lower bound for the headcount was reported"),
        ("uncertain_headcount", "en", "workshop-rough-crowd-size", "A helicopter is needed at Ruby Workshop. Staff describe a crowd of roughly forty to fifty people and have not taken attendance.", "Ruby Workshop", None, "HELICOPTER", [], "Only an approximate headcount range was reported"),
        ("uncertain_headcount", "ml", "olive-hostel-unverified-range", "ഒലീവ് ഹോസ്റ്റലിൽ ഏകദേശം 8 മുതൽ 12 വരെ ആളുകൾ ഉണ്ടാകാമെന്ന് പറയുന്നു; കൃത്യമായി എണ്ണിയിട്ടില്ല. ആംബുലൻസ് വേണം.", "ഒലീവ് ഹോസ്റ്റൽ", None, "AMBULANCE", [], "Only an unverified headcount range was reported"),
        ("uncertain_headcount", "manglish", "coral-centre-approximate-count", "Coral Centre il around 25 per undennu aanu estimate; attendance eduthittilla. Evacuation truck venam.", "Coral Centre", None, "EVAC_TRUCK", [], "The headcount was approximate and unverified"),

        ("missing_location", "en", "voice-note-deictic-pickup", "We are still here and 19 people need a rescue boat immediately. The voice note gives no place name or usable locator.", None, 19, "RESCUE_BOAT", [], "Victim location was not provided"),
        ("missing_location", "en", "blank-address-field", "The request form asks for an ambulance for five people, but its pickup-address field is blank.", None, 5, "AMBULANCE", [], "Victim location was not provided"),
        ("missing_location", "en", "district-only-helicopter-request", "Twenty-two stranded people somewhere in the Riverside district request a helicopter; no pickup site, address, or coordinate is supplied.", None, 22, "HELICOPTER", [], "Only an area was provided, not a usable pickup location"),
        ("missing_location", "en", "expired-location-link", "An evacuation truck is requested for 31 people. Their location was sent through an expired link and is not present in the message.", None, 31, "EVAC_TRUCK", [], "Victim location was not available in the report"),
        ("missing_location", "ml", "unavailable-pin-location", "ലൊക്കേഷൻ പിൻ അയച്ചതായി പറയുന്നു, പക്ഷേ പിൻ സന്ദേശത്തിൽ ലഭ്യമല്ല. 10 പേർക്ക് രക്ഷാബോട്ട് വേണം.", None, 10, "RESCUE_BOAT", [], "Victim location was not available in the report"),
        ("missing_location", "manglish", "here-only-pickup", "Njangal ivide 15 perundu, ambulance venam. Place name, address, landmark onnum messageil illa.", None, 15, "AMBULANCE", [], "Victim location was not provided"),

        ("implicit_asset_only", "en", "mobility-needs-no-asset", "At Briar Hall, 12 residents cannot walk without help and need evacuation. The caller does not request any particular vehicle.", "Briar Hall", 12, None, [], "Required asset was not specified"),
        ("implicit_asset_only", "en", "chest-pain-generic-help", "Four people at Valley Office report chest pain and ask for urgent help, without naming an ambulance or any other transport.", "Valley Office", 4, None, [], "Required asset was not specified"),
        ("implicit_asset_only", "en", "water-at-entrance-no-boat-request", "Water has reached the entrance of Elm Residence, where 28 people ask to be moved. No rescue vehicle is explicitly requested.", "Elm Residence", 28, None, [], "Required asset was not specified"),
        ("implicit_asset_only", "en", "roof-isolation-generic-assistance", "Seven people are isolated on the roof of Slate House and request assistance. The report never specifies a helicopter, boat, or other asset.", "Slate House", 7, None, [], "Required asset was not specified"),
        ("implicit_asset_only", "ml", "elderly-group-no-vehicle", "സീഡാർ ഹോമിൽ 18 വയോധികർക്ക് നടക്കാൻ ബുദ്ധിമുട്ടുണ്ട്; അടിയന്തരമായി മാറ്റണമെന്ന് പറയുന്നു. പ്രത്യേക വാഹനം ആവശ്യപ്പെട്ടിട്ടില്ല.", "സീഡാർ ഹോം", 18, None, [], "Required asset was not specified"),
        ("implicit_asset_only", "manglish", "injured-group-generic-transport", "Palm Lodge il 9 injured aalukalkku urgent transport venam, pakshe ethu vehicle venam ennu requestil paranjittilla.", "Palm Lodge", 9, None, [], "Required asset was not specified"),

        ("hazard_unknown_status", "en", "forge-road-status-disputed", "Send an ambulance for 10 people at Hazel Clinic. Forge Road has had a reported hazard for 4 hours, but witnesses disagree on whether it is blocked or flooded.", "Hazel Clinic", 10, "AMBULANCE", [h("Forge Road", None, 4)], "Hazard status was not confirmed"),
        ("hazard_unknown_status", "en", "garden-lane-no-condition-update", "Twenty people at Birch Hostel need a rescue boat. A road hazard has been present on Garden Lane for 2 hours; no report confirms whether it is open, blocked, or flooded.", "Birch Hostel", 20, "RESCUE_BOAT", [h("Garden Lane", None, 2)], "Hazard status was not confirmed"),
        ("hazard_unknown_status", "ml", "station-link-conflicting-status", "ഡെൽറ്റ ടവറിൽ ആറു പേർക്ക് ഹെലികോപ്റ്റർ വേണം. സ്റ്റേഷൻ ലിങ്ക് റോഡിൽ 5 മണിക്കൂറായി ഒരു പ്രശ്നം തുടരുന്നു. ഒരു റിപ്പോർട്ട് റോഡ് അടഞ്ഞതാണെന്നും മറ്റൊരു റിപ്പോർട്ട് തുറന്നതാണെന്നും പറയുന്നു; ഒന്നും സ്ഥിരീകരിച്ചിട്ടില്ല.", "ഡെൽറ്റ ടവർ", 6, "HELICOPTER", [h("സ്റ്റേഷൻ ലിങ്ക് റോഡ്", None, 5)], "Conflicting hazard statuses were reported"),
        ("hazard_unknown_status", "en", "market-road-unclear-condition", "Send an evacuation truck for 44 people at Crown Hall. An unresolved road hazard has persisted on Market Road for 3 hours, but its road status is marked unknown.", "Crown Hall", 44, "EVAC_TRUCK", [h("Market Road", None, 3)], "Hazard status was not confirmed"),
        ("hazard_unknown_status", "ml", "temple-road-status-unknown", "റോസ് ക്ലിനിക്കിൽ 13 പേർക്ക് ആംബുലൻസ് വേണം. ടെമ്പിൾ റോഡിലെ പ്രശ്നം 6 മണിക്കൂറായി തുടരുന്നു, പക്ഷേ റോഡ് തുറന്നതാണോ അടഞ്ഞതാണോ വെള്ളത്തിലാണോ വ്യക്തമല്ല.", "റോസ് ക്ലിനിക്ക്", 13, "AMBULANCE", [h("ടെമ്പിൾ റോഡ്", None, 6)], "Hazard status was not confirmed"),
        ("hazard_unknown_status", "manglish", "mill-road-condition-unresolved", "Bamboo Hostel il 16 perku rescue boat venam. Mill Road il 2 hours aayi issue undu, pakshe open aano blocked aano flooded aano confirm alla.", "Bamboo Hostel", 16, "RESCUE_BOAT", [h("Mill Road", None, 2)], "Hazard status was not confirmed"),

        ("hazard_missing_duration", "en", "oak-road-flood-no-onset", "A rescue boat is needed for 27 people at Hill Lodge. Oak Road is flooded, but the report contains no start time or elapsed duration.", "Hill Lodge", 27, "RESCUE_BOAT", [h("Oak Road", "FLOODED", None)], "Hazard duration was not provided"),
        ("hazard_missing_duration", "en", "granite-lane-landslide-no-time", "Send a helicopter for eight people at West Pavilion. Granite Lane is blocked by a landslide; responders do not know when it happened.", "West Pavilion", 8, "HELICOPTER", [h("Granite Lane", "BLOCKED", None)], "Hazard duration was not provided"),
        ("hazard_missing_duration", "en", "school-road-open-duration-unknown", "An ambulance is requested for 15 people at Mint School. School Road is reported open again, although the report does not say how long that status has applied.", "Mint School", 15, "AMBULANCE", [h("School Road", "OPEN", None)], "Hazard duration was not provided"),
        ("hazard_missing_duration", "manglish", "ferry-link-debris-unknown-age", "Coast Centre il 33 perku evacuation truck venam. Ferry Link Road debris kondu blocked aanu; ethra neramayi ennu ariyilla.", "Coast Centre", 33, "EVAC_TRUCK", [h("Ferry Link Road", "BLOCKED", None)], "Hazard duration was not provided"),
        ("hazard_missing_duration", "ml", "canal-road-flood-no-duration", "മേഫിൾ ഹാളിൽ 22 പേർക്ക് രക്ഷാബോട്ട് വേണം. കനാൽ റോഡ് വെള്ളത്തിലാണ്, എന്നാൽ എത്ര സമയമായി എന്ന് റിപ്പോർട്ടിൽ ഇല്ല.", "മേഫിൾ ഹാൾ", 22, "RESCUE_BOAT", [h("കനാൽ റോഡ്", "FLOODED", None)], "Hazard duration was not provided"),
        ("hazard_missing_duration", "manglish", "church-road-blocked-no-time", "Sunrise Home il 11 perku ambulance venam. Church Road blocked aanu, pakshe ethra neramayi ennu aarkkum ariyilla.", "Sunrise Home", 11, "AMBULANCE", [h("Church Road", "BLOCKED", None)], "Hazard duration was not provided"),

        ("hazard_unanchored_road", "en", "plaza-nearby-road-flood", "Send an evacuation truck for 29 people at Marigold Plaza. A nearby road has been flooded for 7 hours, but the caller cannot identify it.", "Marigold Plaza", 29, "EVAC_TRUCK", [h(None, "FLOODED", 7)], "Hazard road was not specifically identified"),
        ("hazard_unanchored_road", "en", "college-road-over-there-blocked", "A helicopter is needed for five people at Juniper College. The road over there has been blocked for 2 hours; no road name or anchored description is given.", "Juniper College", 5, "HELICOPTER", [h(None, "BLOCKED", 2)], "Hazard road was not specifically identified"),
        ("hazard_unanchored_road", "en", "warehouse-surrounding-road-open", "Nineteen people at Falcon Warehouse request an ambulance. The surrounding road has been open for 3 hours, but the message does not identify which road.", "Falcon Warehouse", 19, "AMBULANCE", [h(None, "OPEN", 3)], "Hazard road was not specifically identified"),
        ("hazard_unanchored_road", "en", "shelter-this-road-flooded", "A rescue boat is requested for 38 people at Lotus Shelter. This road has been flooded for 4 hours, with no name, coordinate, or named-place anchor.", "Lotus Shelter", 38, "RESCUE_BOAT", [h(None, "FLOODED", 4)], "Hazard road was not specifically identified"),
        ("hazard_unanchored_road", "ml", "academy-unnamed-near-road", "സിൽവർ അക്കാദമിയിൽ 24 പേർക്ക് ഒഴിപ്പിക്കൽ ട്രക്ക് വേണം. അടുത്തുള്ള ഒരു റോഡ് 5 മണിക്കൂറായി അടഞ്ഞുകിടക്കുന്നു, പക്ഷേ റോഡ് ഏതെന്ന് വ്യക്തമല്ല.", "സിൽവർ അക്കാദമി", 24, "EVAC_TRUCK", [h(None, "BLOCKED", 5)], "Hazard road was not specifically identified"),
        ("hazard_unanchored_road", "manglish", "clinic-aa-road-flood", "Lake Clinic il 14 perku rescue boat venam. Aa road 6 hours aayi flooded aanu, roadinte peru allenkil exact location paranjittilla.", "Lake Clinic", 14, "RESCUE_BOAT", [h(None, "FLOODED", 6)], "Hazard road was not specifically identified"),

        ("complete_boundary", "en", "final-headcount-explicit-correction", "The preliminary note counted 17 people at Iris Hall; the supervisor now confirms the corrected final count is 19 and cancels the earlier figure. Send an ambulance.", "Iris Hall", 19, "AMBULANCE", [], None),
        ("complete_boundary", "en", "asset-request-explicitly-corrected", "For 12 people at Walnut Centre, an early request mentioned a rescue boat. Command has replaced it with the confirmed final request: send one evacuation truck.", "Walnut Centre", 12, "EVAC_TRUCK", [], None),
        ("complete_boundary", "en", "complete-road-open-report", "A helicopter is needed for seven people at Aurora House. Ridge Road has been open for exactly 2 hours.", "Aurora House", 7, "HELICOPTER", [h("Ridge Road", "OPEN", 2)], None),
        ("complete_boundary", "en", "complete-road-blockage-report", "Send a rescue boat for 30 people at Summit Hostel. Valley Link Road has been blocked by debris for 5 hours.", "Summit Hostel", 30, "RESCUE_BOAT", [h("Valley Link Road", "BLOCKED", 5)], None),
        ("complete_boundary", "ml", "resolved-location-correction", "ആദ്യ സന്ദേശത്തിൽ സ്ഥലം പൈൻ ഹാൾ എന്ന് തെറ്റായി രേഖപ്പെടുത്തി. കോർഡിനേറ്റർ അത് തിരുത്തി അന്തിമ പിക്കപ്പ് സ്ഥലം ഓർക്കിഡ് ഹാൾ ആണെന്ന് സ്ഥിരീകരിച്ചു. അവിടെ 18 പേർക്ക് ആംബുലൻസ് വേണം.", "ഓർക്കിഡ് ഹാൾ", 18, "AMBULANCE", [], None),
        ("complete_boundary", "manglish", "complete-named-road-flood", "Cedar School il 21 perku rescue boat venam. Harbour Road 3 hours aayi flooded aanu.", "Cedar School", 21, "RESCUE_BOAT", [h("Harbour Road", "FLOODED", 3)], None),
    ]
    for spec in specs:
        add(r, "train", *spec)
    return r


def validation_rows() -> list[dict[str, Any]]:
    r: list[dict[str, Any]] = []
    specs = [
        ("contradictory_count", "en", "museum-roster-conflict", "The current register at Opal Museum Annex lists 14 evacuees, while the duty officer's current tally lists 18 for the same group. Send an ambulance; neither total is verified.", "Opal Museum Annex", None, "AMBULANCE", [], "Conflicting headcounts were reported for the same group"),
        ("contradictory_count", "ml", "station-platform-count-conflict", "നോർത്ത് സ്റ്റേഷൻ ഹാളിൽ രക്ഷാബോട്ട് വേണം. പ്ലാറ്റ്ഫോമിലുള്ള മുഴുവൻ ആളുകളെയും എണ്ണിയ രണ്ട് ടീമുകൾ 25 പേരെന്നും 29 പേരെന്നും റിപ്പോർട്ട് ചെയ്യുന്നു; അന്തിമ കണക്ക് അംഗീകരിച്ചിട്ടില്ല.", "നോർത്ത് സ്റ്റേഷൻ ഹാൾ", None, "RESCUE_BOAT", [], "Conflicting headcounts were reported for the same group"),
        ("contradictory_count", "ml", "relief-room-count-conflict", "എമറാൾഡ് റിലീഫ് റൂമിലെ ഒരേ സംഘത്തിന് 33 പേരെന്നും 37 പേരെന്നും രണ്ട് നിലവിലെ കണക്കുകളുണ്ട്. ശരിയായ എണ്ണം തീരുമാനിച്ചിട്ടില്ല. ഹെലികോപ്റ്റർ വേണം.", "എമറാൾഡ് റിലീഫ് റൂം", None, "HELICOPTER", [], "Conflicting headcounts were reported for the same group"),
        ("contradictory_location", "en", "choir-group-location-conflict", "Eleven people need an evacuation truck. One live update locates the same choir group at Pearl Chapel Hall and another at Amber Music Room; pickup remains unresolved.", None, 11, "EVAC_TRUCK", [], "Conflicting pickup locations were reported"),
        ("contradictory_location", "en", "clinic-form-address-conflict", "The request is for an ambulance and six patients, but the header says Ivy Clinic while the signed address line says Coral Health Post. Staff have not confirmed either pickup.", None, 6, "AMBULANCE", [], "Conflicting pickup locations were reported"),
        ("contradictory_location", "manglish", "workers-group-location-conflict", "23 workers ulla same group Pearl Workshop il aanu ennu oru callum Sandal Depot il aanu ennu mattoru callum parayunnu. Pickup confirm alla; evacuation truck venam.", None, 23, "EVAC_TRUCK", [], "Conflicting pickup locations were reported"),
        ("contradictory_asset", "en", "terrace-asset-checkbox-conflict", "At Lake Terrace, 17 people await pickup. The radio explicitly requests a helicopter, but the dispatch checkbox selects rescue boat; no final asset is recorded.", "Lake Terrace", 17, None, [], "Conflicting assets were requested"),
        ("contradictory_asset", "en", "school-two-current-assets", "Nine people are at Acacia School. The field request says ambulance and the command message says evacuation truck, both marked current.", "Acacia School", 9, None, [], "Conflicting assets were requested"),
        ("contradictory_asset", "manglish", "pavilion-asset-disagreement", "Violet Pavilion il 20 perundu. Written request ambulance aanu, radio request helicopter aanu; ethu asset final aanu ennu confirm cheythittilla.", "Violet Pavilion", 20, None, [], "Conflicting assets were requested"),
        ("missing_asset", "en", "wheelchair-users-no-vehicle", "Ten wheelchair users at Cypress Centre ask for urgent evacuation, but their message does not specify a vehicle.", "Cypress Centre", 10, None, [], "Required asset was not specified"),
        ("missing_location", "en", "missing-coordinate-attachment", "A helicopter is requested for eight people. The coordinate attachment failed to upload, and no pickup location appears in the text.", None, 8, "HELICOPTER", [], "Victim location was not available in the report"),
        ("uncertain_headcount", "ml", "hostel-headcount-upper-range", "ഇൻഡിഗോ ഹോസ്റ്റലിൽ 15 മുതൽ 20 വരെ ആളുകൾ ഉണ്ടാകാമെന്ന് പറയുന്നു; കൃത്യമായ കണക്ക് എടുത്തിട്ടില്ല. രക്ഷാബോട്ട് വേണം.", "ഇൻഡിഗോ ഹോസ്റ്റൽ", None, "RESCUE_BOAT", [], "Only an unverified headcount range was reported"),
        ("hazard_unknown_status", "en", "bridge-road-status-pending", "Send an ambulance for 12 people at Mercury Hall. Bridge Approach Road has had an unresolved road condition for 3 hours, but its status is still pending and cannot be classified as open, blocked, or flooded.", "Mercury Hall", 12, "AMBULANCE", [h("Bridge Approach Road", None, 3)], "Hazard status was not confirmed"),
        ("hazard_unknown_status", "en", "port-lane-status-conflict", "A rescue boat is requested for 28 people at Lime Hostel. Port Lane has had an unresolved road condition for 2 hours; reports disagree over whether it is flooded or open.", "Lime Hostel", 28, "RESCUE_BOAT", [h("Port Lane", None, 2)], "Conflicting hazard statuses were reported"),
        ("hazard_unknown_status", "ml", "junction-road-status-unknown", "ബ്രോൺസ് ഹാളിൽ 16 പേർക്ക് ഒഴിപ്പിക്കൽ ട്രക്ക് വേണം. ജങ്ഷൻ റോഡിൽ 4 മണിക്കൂറായി പ്രശ്നമുണ്ട്, പക്ഷേ റോഡിന്റെ നില വ്യക്തമല്ല.", "ബ്രോൺസ് ഹാൾ", 16, "EVAC_TRUCK", [h("ജങ്ഷൻ റോഡ്", None, 4)], "Hazard status was not confirmed"),
        ("hazard_missing_duration", "en", "orchard-road-blockage-no-age", "Seven people at Plum House need a helicopter. Orchard Road is blocked by a collapsed wall, and the duration was not recorded.", "Plum House", 7, "HELICOPTER", [h("Orchard Road", "BLOCKED", None)], "Hazard duration was not provided"),
        ("hazard_missing_duration", "manglish", "depot-road-flood-no-duration", "Rose Depot il 24 perku rescue boat venam. Depot Road flooded aanu, pakshe ethra hours aayi ennu reportil illa.", "Rose Depot", 24, "RESCUE_BOAT", [h("Depot Road", "FLOODED", None)], "Hazard duration was not provided"),
        ("hazard_missing_duration", "manglish", "hill-road-open-time-unknown", "Ash School il 13 perku ambulance venam. Hill Road ippo open aanu, ennal eppol muthal open aanu ennu ariyilla.", "Ash School", 13, "AMBULANCE", [h("Hill Road", "OPEN", None)], "Hazard duration was not provided"),
        ("hazard_unanchored_road", "en", "library-unnamed-surrounding-road", "An evacuation truck is requested for 35 people at Topaz Library. A surrounding road has been blocked for 6 hours, but the road is not identified.", "Topaz Library", 35, "EVAC_TRUCK", [h(None, "BLOCKED", 6)], "Hazard road was not specifically identified"),
        ("hazard_unanchored_road", "manglish", "hall-nearby-road-flood", "Golden Hall il 18 perku rescue boat venam. Aduthulla road 5 hours aayi flooded aanu, exact road etha ennu paranjittilla.", "Golden Hall", 18, "RESCUE_BOAT", [h(None, "FLOODED", 5)], "Hazard road was not specifically identified"),
        ("hazard_unanchored_road", "ml", "clinic-this-road-blocked", "പേൾ ക്ലിനിക്കിൽ 9 പേർക്ക് ആംബുലൻസ് വേണം. ഈ റോഡ് 2 മണിക്കൂറായി അടഞ്ഞുകിടക്കുന്നു; റോഡിന്റെ പേരോ വ്യക്തമായ സ്ഥലസൂചനയോ നൽകിയിട്ടില്ല.", "പേൾ ക്ലിനിക്ക്", 9, "AMBULANCE", [h(None, "BLOCKED", 2)], "Hazard road was not specifically identified"),
        ("complete_boundary", "en", "verified-final-asset-change", "A draft request for 15 people at Spruce Hall mentioned an ambulance. The incident lead has withdrawn it and confirms the final request is a helicopter.", "Spruce Hall", 15, "HELICOPTER", [], None),
        ("complete_boundary", "ml", "complete-road-open-duration", "ഗോൾഡ് സ്കൂളിൽ 19 പേർക്ക് ആംബുലൻസ് വേണം. റെയിൽവേ ലിങ്ക് റോഡ് 3 മണിക്കൂറായി തുറന്നിരിക്കുന്നു.", "ഗോൾഡ് സ്കൂൾ", 19, "AMBULANCE", [h("റെയിൽവേ ലിങ്ക് റോഡ്", "OPEN", 3)], None),
        ("complete_boundary", "manglish", "complete-road-blocked-duration", "Teak Lodge il 22 perku evacuation truck venam. Market Bypass Road 4 hours aayi blocked aanu.", "Teak Lodge", 22, "EVAC_TRUCK", [h("Market Bypass Road", "BLOCKED", 4)], None),
    ]
    for spec in specs:
        add(r, "validation", *spec)
    return r


def reference_rows() -> list[dict[str, Any]]:
    rows = []
    for path in sorted(DATA.glob("*.jsonl")):
        if path not in OUTPUTS.values():
            rows.extend(load_cases(path))
    return rows


def validate(datasets: dict[str, list[dict[str, Any]]]) -> None:
    schema_validator = Draft202012Validator(load_json(DEFAULT_SCHEMA))
    refs = reference_rows()
    seen_ids = {row.get("id") for row in refs}
    seen_inputs = {normalize_text(row["input"]) for row in refs if row.get("input")}
    families: set[str] = set()
    wordings: set[str] = set()
    for key, rows in datasets.items():
        if Counter(row["language"] for row in rows) != EXPECTED_LANGUAGES[key]:
            raise ValueError(f"Wrong language allocation for {key}")
        if Counter(row["category"] for row in rows) != EXPECTED_CATEGORIES[key]:
            raise ValueError(f"Wrong category allocation for {key}")
        for row in rows:
            if row["split"] != ROW_SPLITS[key]:
                raise ValueError(f"Wrong split metadata: {row['id']}")
            if row["id"] in seen_ids:
                raise ValueError(f"Duplicate ID: {row['id']}")
            seen_ids.add(row["id"])
            normalized = normalize_text(row["input"])
            if normalized in seen_inputs:
                raise ValueError(f"Exact input overlap: {row['id']}")
            seen_inputs.add(normalized)
            for field, values in (("scenario_family", families), ("wording_family", wordings)):
                if row[field] in values:
                    raise ValueError(f"Duplicate {field}: {row[field]}")
                values.add(row[field])
            schema_validator.validate(row["target"])
            target = row["target"]
            incomplete = any(target[field] is None for field in ("victim_location", "headcount", "required_asset")) or any(value is None for hazard in target["hazards"] for value in hazard.values())
            if incomplete != target["needs_human_review"]:
                raise ValueError(f"Review/null mismatch: {row['id']}")
            if bool(target["uncertainty_reasons"]) != target["needs_human_review"]:
                raise ValueError(f"Reason/review mismatch: {row['id']}")
            if row["reviewer_status"] not in {"pending_manual_review", "first_pass_reviewed"}:
                raise ValueError(f"Unexpected review status: {row['id']}")
            if re.search(r"\{[a-z_]+\}", row["input"]):
                raise ValueError(f"Unrendered slot: {row['id']}")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_jsonl(rows: list[dict[str, Any]], path: Path) -> None:
    path.write_text("".join(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n" for row in rows), encoding="utf-8")


def generate(directory: Path = DATA) -> dict[str, Path]:
    outputs = {key: directory / path.name for key, path in OUTPUTS.items()}
    manifest_path = directory / "triage-sft-v3-train-validation.manifest.json"
    if manifest_path.exists() or any(path.exists() for path in outputs.values()):
        raise FileExistsError("Refusing to overwrite V3 candidates or review history")
    datasets = {"train_augmentation": train_rows(), "validation_extension": validation_rows()}
    validate(datasets)
    for key, path in outputs.items():
        write_jsonl(datasets[key], path)
    manifest = {
        "dataset": "CrisisGraph Triage SFT V3 targeted additions",
        "candidate_revision": 1,
        "generated_on": "2026-09-21",
        "status": "pending_first_pass_review",
        "provenance": "AI-assisted, individually authored synthetic scenarios with deterministic validation and emission",
        "base_dataset": "CrisisGraph Triage SFT Pilot V1 revision 3",
        "generation_uses_frozen_evaluation_records": False,
        "references_used_for_overlap_checks_only": True,
        "training_ready": False,
        "exported": False,
        "files": {key: {"path": path.name, "sha256": digest(path), "records": len(datasets[key]), "language_counts": dict(Counter(r["language"] for r in datasets[key])), "category_counts": dict(Counter(r["category"] for r in datasets[key])), "reviewer_status_counts": {"pending_manual_review": len(datasets[key])}} for key, path in outputs.items()},
        "independence": {"unique_scenario_families": 84, "unique_wording_families": 84, "exact_reference_overlap": 0},
        "review": {"required_next": "AI-assisted first-pass semantic and language review", "reviewed_records": 0, "first_pass_reviewed": 0, "needs_revision": 0},
        "limitations": ["Synthetic records are not evidence of field-language distribution", "Automated checks do not prove semantic correctness or Malayalam/Manglish fluency", "The exposed regression suite cannot support an unbiased generalization claim"],
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return outputs


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("generate", "validate"))
    parser.add_argument("--data-dir", type=Path, default=DATA)
    args = parser.parse_args()
    if args.command == "generate":
        paths = generate(args.data_dir)
    else:
        paths = {key: args.data_dir / path.name for key, path in OUTPUTS.items()}
        validate({key: load_cases(path) for key, path in paths.items()})
    print(json.dumps({"mechanical_checks": "passed", "files": {key: str(path) for key, path in paths.items()}}))


if __name__ == "__main__":
    main()
