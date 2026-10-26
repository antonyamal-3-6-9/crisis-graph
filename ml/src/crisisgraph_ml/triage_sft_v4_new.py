"""New SFT-V4 training records, written in batches by failure type.

Each batch targets failure types from ``data/triage-sft-v4-spec.md`` (T1–T13).
Labels follow the main guideline and both post-freeze change files. Targets
use the V3 schema with ``victim_profile`` last; V2 fields follow the SFT
conventions (fixed confidence mapping, standard uncertainty reasons, review
by the V2 rule); ``suggested_urgency`` is the gold tier, or null when UNKNOWN.

Places are invented or taken from outside the evaluation sets; separation
from every evaluation set is checked by ``separation_problems``.
"""

from __future__ import annotations

import argparse
from collections import Counter
from pathlib import Path
from typing import Any

from crisisgraph_ml.review_sheet import render
from crisisgraph_ml.triage_eval import ML_ROOT, load_cases
from crisisgraph_ml.triage_eval_v3_1 import DATASET_NGRAM, ngrams
from crisisgraph_ml.triage_eval_v3_dev import write_cases
from crisisgraph_ml.triage_sft_v4_convert import validate_records

DATA = ML_ROOT / "data"
DEFAULT_OUTPUT = DATA / "triage-sft-v4-new-candidates.jsonl"
EVAL_SETS = [
    DATA / "triage-eval-v3-dev-candidates.jsonl",
    DATA / "triage-eval-v3.0.1.jsonl",
    DATA / "triage-eval-v3-v2-carryover.jsonl",
    DATA / "triage-eval-v3.1.0.jsonl",
]

# Batches that passed the AI first pass and the owner review.
OWNER_REVIEWED_BATCHES = {1}

SELF = {"relation": "SELF", "on_scene": True}
REMOTE_FAMILY = {"relation": "HOUSEHOLD", "on_scene": False}
BYSTANDER = {"relation": "THIRD_PARTY", "on_scene": True}


def reasons_for(location, headcount, asset, hazards) -> list[str]:
    """Standard SFT uncertainty reasons, in field order, at most three."""
    reasons = []
    if location is None:
        reasons.append("Victim location was not provided")
    if headcount is None:
        reasons.append("Headcount was not provided")
    if asset is None:
        reasons.append("Required asset was not specified")
    for hazard in hazards:
        if hazard["road_segment"] is None:
            reasons.append("Hazard road was not specifically identified")
        if hazard["status"] is None:
            reasons.append("Hazard status was not confirmed")
        if hazard["duration_hours"] is None:
            reasons.append("Hazard duration was not provided")
    return list(dict.fromkeys(reasons))[:3]


def rec(
    number: int,
    batch: int,
    failure_type: str,
    language: str,
    text: str,
    *,
    location: str | None,
    headcount: int | None,
    asset: str | None,
    gold: str,
    hazards: list[dict[str, Any]] | None = None,
    relation: str | None = None,
    on_scene: bool | None = None,
    medical: list[str] | None = None,
    other: list[str] | None = None,
    groups: list[str] | None = None,
    water: str | None = None,
    trend: str | None = None,
    split: str = "train",
) -> dict[str, Any]:
    hazards = hazards or []
    reasons = reasons_for(location, headcount, asset, hazards)
    review = bool(reasons)
    target = {
        "schema_version": "triage-extraction-v3",
        "victim_location": location,
        "headcount": headcount,
        "required_asset": asset,
        "hazards": hazards,
        "confidence_score": 0.55 if review else 0.95,
        "needs_human_review": review,
        "uncertainty_reasons": reasons,
        "victim_profile": {
            "reporter_relation": relation,
            "reporter_on_scene": on_scene,
            "medical_conditions": medical or [],
            "other_conditions": other or [],
            "vulnerable_groups": groups or [],
            "water_level": water,
            "water_trend": trend,
            "suggested_urgency": None if gold == "UNKNOWN" else gold,
        },
    }
    return {
        "id": f"sft-v4-new-b{batch}-{number:03d}",
        "split": split,
        "batch": batch,
        "failure_type": failure_type,
        "language": language,
        "reviewer_status": "reviewed" if batch in OWNER_REVIEWED_BATCHES
        else "pending_manual_review",
        "input": text,
        "target": target,
        "expected_urgency": gold,
    }


def h(road: str | None, status: str | None, hours: int | None) -> dict[str, Any]:
    return {"road_segment": road, "status": status, "duration_hours": hours}


def batch_1() -> list[dict[str, Any]]:
    """T1 water described by the building; T2 ranges, units and band edges.

    Owner policy (batch 1 review): a fully submerged ground floor is CRITICAL
    even with people safe upstairs, because it means about 3 m of water around
    the house, no way out except by boat and one storey of headroom. About
    2 ft (60 cm, thigh-deep) with healthy adults is STABLE in gold, although
    the band rule labels it WAIST; that is accepted over-triage by the rules.
    """
    def r(n, ft, lang, text, **kw):
        return rec(n, 1, ft, lang, text, **kw)

    return [
        # --- T1: depth described by the building, not the body ---
        r(1, "T1", "en", "The whole ground floor of our house on Thekkekara Library Road is under "
          "water. Four of us are upstairs. Need a rescue boat.",
          location="Thekkekara Library Road", headcount=4, asset="RESCUE_BOAT", gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),
        r(2, "T1", "en", "Water has filled the ground floor of Puthenvila school completely. 30 "
          "schoolchildren and 2 teachers are on the second floor. Send boats.",
          location="Puthenvila school", headcount=32, asset="RESCUE_BOAT", gold="CRITICAL",
          groups=["CHILD"], water="ABOVE_HEAD"),
        r(3, "T1", "manglish", "Veedinte thazhathe nila muzhuvan vellathil aanu, njangal 3 per "
          "mukalil und. Kallingal kadavu inte aduthu.",
          location="Kallingal kadavu", headcount=3, asset=None, gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),
        r(4, "T1", "ml", "വീടിന്റെ താഴത്തെ നില മുഴുവൻ വെള്ളത്തിലാണ്. ഞങ്ങൾ അഞ്ചുപേർ മുകളിലത്തെ "
          "നിലയിൽ ഉണ്ട്. വാരിക്കാട്ട് പള്ളിക്ക് അടുത്ത്. ബോട്ട് വേണം.",
          location="വാരിക്കാട്ട് പള്ളി", headcount=5, asset="RESCUE_BOAT", gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),
        r(5, "T1", "en", "Our house is under water right up to the roof and we are sitting on top "
          "of it, two adults and a baby. Chakkamadam junction.",
          location="Chakkamadam junction", headcount=3, asset=None, gold="CRITICAL",
          other=["stranded on a roof"], groups=["INFANT"], water="ROOF", **SELF),
        r(6, "T1", "manglish", "Veedu muzhuvan mungi, njangal 4 perum terrace il kayari irikkunnu. "
          "Elanjikal temple inte aduthu. Boat venam vegam.",
          location="Elanjikal temple", headcount=4, asset="RESCUE_BOAT", gold="CRITICAL",
          other=["stranded on a terrace"], water="ROOF", **SELF),
        r(7, "T1", "ml", "വീട് മുഴുവൻ മുങ്ങി. ഞങ്ങൾ മൂന്നുപേർ മേൽക്കൂരയിൽ കയറി ഇരിക്കുകയാണ്. "
          "കൈതക്കുഴി പാലത്തിന് അടുത്ത്.",
          location="കൈതക്കുഴി പാലം", headcount=3, asset=None, gold="CRITICAL",
          other=["stranded on a roof"], water="ROOF", **SELF),
        r(8, "T1", "en", "Water came into the house last night. We are 6 people at Mullassery "
          "Lane and want to move to a camp. Please send a truck.",
          location="Mullassery Lane", headcount=6, asset="EVAC_TRUCK", gold="UNKNOWN", **SELF),
        r(9, "T1", "en", "The area around Ottaplamoodu market is flooded. 12 shopkeepers are "
          "stuck inside the market building and need an evacuation truck.",
          location="Ottaplamoodu market", headcount=12, asset="EVAC_TRUCK", gold="UNKNOWN",
          hazards=[h(None, "FLOODED", None)]),
        r(10, "T1", "manglish", "Veettil vellam kayari, ethra und ennu ariyilla. Njangal randu "
          "perum ammachiyum aanu, Panayil kadavu il.",
          location="Panayil kadavu", headcount=3, asset=None, gold="UNKNOWN",
          groups=["ELDERLY"], **SELF),
        r(11, "T1", "ml", "വീടിനുള്ളിൽ വെള്ളം കയറി, എത്ര ഉയരമുണ്ടെന്ന് പറയാൻ പറ്റുന്നില്ല. "
          "ഞങ്ങൾ നാലുപേർ കണ്ണമ്പിള്ളി ഹാളിന് അടുത്താണ്.",
          location="കണ്ണമ്പിള്ളി ഹാൾ", headcount=4, asset=None, gold="UNKNOWN", **SELF),
        r(12, "T1", "en", "My parents are stuck in their single-storey house at Vazhayil colony. "
          "The water is now above the windows inside the house and still rising. I am in Pune.",
          location="Vazhayil colony", headcount=2, asset=None, gold="CRITICAL",
          water="ABOVE_HEAD", trend="RISING", **REMOTE_FAMILY),
        r(13, "T1", "en", "Ground floor fully flooded at Paduthode school, 40 people from the camp "
          "have moved to the first floor. No one is sick. We need boats to shift them.",
          location="Paduthode school", headcount=40, asset="RESCUE_BOAT", gold="CRITICAL",
          water="ABOVE_HEAD"),  # "shift them": coordinator, relation unknown (rule 5)
        r(14, "T1", "manglish", "Thazhathe nila full aayi mungi, vellam ippozhum koodunnu. "
          "Njangal 5 per und, athil oru kaikkunju. Ayiramthengu junction.",
          location="Ayiramthengu junction", headcount=5, asset=None, gold="CRITICAL",
          groups=["INFANT"], water="ABOVE_HEAD", trend="RISING", **SELF),
        r(15, "T1", "ml", "പാടത്തോട് സ്കൂളിന്റെ താഴത്തെ നില പൂർണ്ണമായും വെള്ളത്തിലായി. ക്യാമ്പിലെ "
          "മുപ്പതുപേർ മുകളിലുണ്ട്. ബോട്ട് വേണം.",
          location="പാടത്തോട് സ്കൂൾ", headcount=30, asset="RESCUE_BOAT", gold="CRITICAL",
          water="ABOVE_HEAD"),
        r(16, "T1", "en", "Water is at the level of the eaves of our neighbour's house at "
          "Kottekkad mosque road. The family of 3 is on their roof. I can see them from my terrace. "
          "Send a boat.",
          location="Kottekkad mosque road", headcount=3, asset="RESCUE_BOAT", gold="CRITICAL",
          other=["stranded on a roof"], water="ROOF", **BYSTANDER),
        r(17, "T1", "en", "Water entered our house about an hour ago and it keeps coming in. 3 of "
          "us near Kunnathukara church.",
          location="Kunnathukara church", headcount=3, asset=None, gold="UNKNOWN",
          trend="RISING", **SELF),
        r(18, "T1", "manglish", "Veettil vellam kayari varunnu, ippo ethra und ennu ariyilla. "
          "Appooppanum ammoommayum mathram aanu. Melepuram LP school nte aduthu. Njan Dubai il aanu.",
          location="Melepuram LP school", headcount=2, asset=None, gold="URGENT",
          groups=["ELDERLY"], trend="RISING", **REMOTE_FAMILY),
        r(19, "T1", "ml", "വീട് മുഴുവൻ വെള്ളത്തിൽ മുങ്ങിയിരിക്കുന്നു. അയൽക്കാരായ നാലുപേർ "
          "ടെറസിൽ ഉണ്ട്, ഞാൻ എന്റെ വീട്ടിൽ നിന്ന് കാണുന്നുണ്ട്. ചിറ്റിലക്കാട് ജംഗ്ഷൻ.",
          location="ചിറ്റിലക്കാട് ജംഗ്ഷൻ", headcount=4, asset=None, gold="CRITICAL",
          other=["stranded on a terrace"], water="ROOF", **BYSTANDER),
        r(20, "T1", "en", "The water is only on the road outside, it has not come into Neerattupuram "
          "community hall where 25 of us are staying. We would like an evacuation truck tomorrow.",
          location="Neerattupuram community hall", headcount=25, asset="EVAC_TRUCK", gold="STABLE",
          hazards=[h(None, "FLOODED", None)], **SELF),
        r(21, "T1", "en", "Ground floor of Mannanthala flats is under water. 8 families on the "
          "upper floors, including an elderly man on oxygen. Need boats.",
          location="Mannanthala flats", headcount=None, asset="RESCUE_BOAT", gold="CRITICAL",
          medical=["CHRONIC_CARE"], groups=["ELDERLY"], water="ABOVE_HEAD"),
        r(22, "T1", "manglish", "Pallikkoodathinte thazhathe nila full vellathil, njangal 20 per "
          "first floor il aanu. Vettiyar bridge inte aduthulla school aanu. Boat ayakkamo?",
          location="Vettiyar bridge", headcount=20, asset="RESCUE_BOAT", gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),
        # --- T2: ranges, metres and feet at band edges ---
        r(23, "T2", "en", "Water inside is about 1 foot and not rising. 2 of us at Kizhakkeppuram "
          "school, no injuries.",
          location="Kizhakkeppuram school", headcount=2, asset=None, gold="STABLE",
          water="KNEE", trend="STABLE", **SELF),
        r(24, "T2", "en", "We have about 2 feet of water in the house now. 4 adults at Thottumkara "
          "bus stop, all fine.",
          location="Thottumkara bus stop", headcount=4, asset=None, gold="STABLE",
          water="WAIST", **SELF),
        r(25, "T2", "en", "Water is 3.5 feet in our hall. Three of us at Chirakkal temple. "
          "Need a rescue boat.",
          location="Chirakkal temple", headcount=3, asset="RESCUE_BOAT", gold="URGENT",
          water="CHEST", **SELF),
        r(26, "T2", "en", "5 feet of water around Pullankulam junction and we are standing on the "
          "staircase, 3 of us.",
          location="Pullankulam junction", headcount=3, asset=None, gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),
        r(27, "T2", "en", "The water level is 0.6 metres in the house at Edakkattuvayal and "
          "steady. 5 adults. Please send an evacuation truck.",
          location="Edakkattuvayal", headcount=5, asset="EVAC_TRUCK", gold="STABLE",
          water="WAIST", trend="STABLE"),
        r(28, "T2", "en", "Water is around 1.5 m deep at Kanjirakkad junction. Two people are on "
          "the bus shelter roof. Send a boat.",
          location="Kanjirakkad junction", headcount=2, asset="RESCUE_BOAT", gold="CRITICAL",
          other=["stranded on a roof"], water="ABOVE_HEAD"),  # stated depth; not a house roof
        r(29, "T2", "en", "There is 1 to 2 feet of water in the house at Mangattu junction. We are "
          "3 adults and not hurt.",
          location="Mangattu junction", headcount=3, asset=None, gold="STABLE",
          water="WAIST", **SELF),
        r(30, "T2", "en", "Water between 4 and 6 feet near Cheruvallikkavu temple, rising. Family "
          "of five on the first floor. Need a boat.",
          location="Cheruvallikkavu temple", headcount=5, asset="RESCUE_BOAT", gold="CRITICAL",
          water="ABOVE_HEAD", trend="RISING"),
        r(31, "T2", "en", "About 0.3 metres of water in the hostel at Vadakkekara, falling slowly. "
          "10 students, nobody ill.",
          location="Vadakkekara", headcount=10, asset=None, gold="STABLE",
          water="KNEE", trend="FALLING"),
        r(32, "T2", "en", "Water is just under a foot in the courtyard of Puthiyakavu temple. "
          "6 devotees want to leave. Evacuation truck please.",
          location="Puthiyakavu temple", headcount=6, asset="EVAC_TRUCK", gold="STABLE",
          water="KNEE"),  # "just under a foot": "nearly" is ignored, so 1 ft
        r(33, "T2", "en", "My grandmother is at home alone near Valiyaparambu church. Her neighbour "
          "says the water is 1.1 metres inside. I am in Chennai.",
          location="Valiyaparambu church", headcount=1, asset=None, gold="CRITICAL",
          groups=["ELDERLY"], water="CHEST", **REMOTE_FAMILY),
        r(34, "T2", "en", "Water is knee deep, around 2 feet, at Ambalappara colony. Seven of us, "
          "no one sick.",
          location="Ambalappara colony", headcount=7, asset=None, gold="STABLE",
          water="KNEE", **SELF),
        r(35, "T2", "ml", "വീട്ടിൽ രണ്ടടി വെള്ളമുണ്ട്. ഞങ്ങൾ നാലുപേർ, ആർക്കും പ്രശ്നമില്ല. "
          "തെക്കേക്കര ലൈബ്രറിക്ക് അടുത്ത്.",
          location="തെക്കേക്കര ലൈബ്രറി", headcount=4, asset=None, gold="STABLE",
          water="WAIST", **SELF),
        r(36, "T2", "ml", "നാലടി വെള്ളം കയറി, കൂടിക്കൊണ്ടിരിക്കുന്നു. ഞങ്ങൾ മൂന്നുപേർ പുത്തൻവിള "
          "സ്കൂളിന് സമീപം. ബോട്ട് വേണം.",
          location="പുത്തൻവിള സ്കൂൾ", headcount=3, asset="RESCUE_BOAT", gold="CRITICAL",
          water="CHEST", trend="RISING", **SELF),
        r(37, "T2", "ml", "അഞ്ചടിയിൽ കൂടുതൽ വെള്ളമുണ്ട്. രണ്ടുപേർ ഗോവണിയിൽ നിൽക്കുന്നു. "
          "കല്ലിങ്ങൽ കടവിന് അടുത്ത്.",
          location="കല്ലിങ്ങൽ കടവ്", headcount=2, asset=None, gold="CRITICAL",
          water="ABOVE_HEAD"),
        r(38, "T2", "ml", "ഒരടി വെള്ളം മാത്രം, കുറയുന്നുണ്ട്. ആറുപേർ ചക്കമാടം ജംഗ്ഷനിൽ ഉണ്ട്. "
          "ക്യാമ്പിലേക്ക് പോകാൻ ട്രക്ക് വേണം.",
          location="ചക്കമാടം ജംഗ്ഷൻ", headcount=6, asset="EVAC_TRUCK", gold="STABLE",
          water="KNEE", trend="FALLING"),
        r(39, "T2", "ml", "വെള്ളം ഒരു മീറ്ററോളം ഉണ്ട്. എന്റെ ഭാര്യ ഗർഭിണിയാണ്, ഞങ്ങൾ രണ്ടുപേർ "
          "ഇലഞ്ഞിക്കൽ അമ്പലത്തിന് അടുത്ത്.",
          location="ഇലഞ്ഞിക്കൽ അമ്പലം", headcount=2, asset=None, gold="URGENT",
          groups=["PREGNANT"], water="WAIST", **SELF),
        r(40, "T2", "ml", "ഒന്നര മീറ്റർ വെള്ളം, അച്ഛൻ കിടപ്പിലാണ്. ഞങ്ങൾ മൂന്നുപേർ ഒറ്റപ്ലാമൂട് "
          "മാർക്കറ്റിന് സമീപം. ബോട്ട് വേണം.",
          location="ഒറ്റപ്ലാമൂട് മാർക്കറ്റ്", headcount=3, asset="RESCUE_BOAT", gold="CRITICAL",
          groups=["BEDRIDDEN"], water="ABOVE_HEAD", **SELF),
        r(41, "T2", "ml", "മൂന്നോ നാലോ അടി വെള്ളം ഉണ്ട്. ഞങ്ങൾ ഏഴുപേർ വാഴയിൽ കോളനിയിൽ.",
          location="വാഴയിൽ കോളനി", headcount=7, asset=None, gold="URGENT",
          water="CHEST", **SELF),
        r(42, "T2", "manglish", "Randadi vellam und veettil, koodunnilla. Njangal 3 per Mullassery "
          "school il. Truck venam.",
          location="Mullassery school", headcount=3, asset="EVAC_TRUCK", gold="STABLE",
          water="WAIST", trend="STABLE", **SELF),
        r(43, "T2", "manglish", "Naaladi vellam, vegam koodunnu! Njangal 4 per, athil oru kunju, 2 "
          "vayassu. Varikkattu junction. Boat venam.",
          location="Varikkattu junction", headcount=4, asset="RESCUE_BOAT", gold="CRITICAL",
          groups=["CHILD"], water="CHEST", trend="RISING", **SELF),
        r(44, "T2", "manglish", "Oradi vellame ulloo, kurayunnundu. Njangal 8 per Kannampilly "
          "church il. Camp il pokan truck venam.",
          location="Kannampilly church", headcount=8, asset="EVAC_TRUCK", gold="STABLE",
          water="KNEE", trend="FALLING", **SELF),
        r(45, "T2", "manglish", "Anjadi vellam und veedinte chuttum, njangal 2 per mukalil. "
          "Kunnathukara bus stop.",
          location="Kunnathukara bus stop", headcount=2, asset=None, gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),
        r(46, "T2", "manglish", "Ammayude veettil 1.2 meter vellam und ennu ayalkkar paranju. "
          "Amma ottakku aanu, 75 vayassu, oru kaal vayya. Paduthode junction. Njan Bangalore il aanu.",
          location="Paduthode junction", headcount=1, asset=None, gold="CRITICAL",
          groups=["ELDERLY"], water="CHEST", **REMOTE_FAMILY),
        r(47, "T2", "manglish", "Muttolam vellam, around 1.5 adi. Njangal 5 per, aarkkum "
          "kuzhappam illa. Elanjikal school.",
          location="Elanjikal school", headcount=5, asset=None, gold="STABLE",
          water="KNEE", **SELF),
        r(48, "T2", "en", "Water is chest deep for the adults, which is about 4 feet, at Pallathu "
          "kadavu. Two of us and our 8-year-old. Need a boat now.",
          location="Pallathu kadavu", headcount=3, asset="RESCUE_BOAT", gold="CRITICAL",
          groups=["CHILD"], water="CHEST", **SELF),
        r(49, "T2", "en", "The street outside our shop near Kaithakkuzhi bridge has 2 metres of "
          "water. Five of us are upstairs in the shop. Send a boat.",
          location="Kaithakkuzhi bridge", headcount=5, asset="RESCUE_BOAT", gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),
        r(50, "T2", "en", "Water about 2 to 3 feet in Ayiramthengu church hall, and a wheelchair "
          "user is with us. 6 people. Rescue boat please.",
          location="Ayiramthengu church hall", headcount=6, asset="RESCUE_BOAT", gold="URGENT",
          groups=["DISABLED"], water="WAIST", **SELF),
    ]


def batch_2() -> list[dict[str, Any]]:
    """T3 Malayalam / Manglish medical phrasing; T4 rising / falling wording.

    T3 pairs each critical condition with near misses that must not get it:
    dizzy but talking (not UNCONSCIOUS), known asthma or heart disease with no
    episode now (CHRONIC_CARE), a cut that has stopped bleeding (INJURY only),
    and symptoms that resolved earlier today (no condition, gold URGENT). T4
    covers rising, falling and explicitly unchanged water in all three
    languages, plus hypotheticals ("if the water rises") that leave the trend
    null. Relatives named by role ("achan", "amma", "monu") add no age group.
    """
    def r(n, ft, lang, text, **kw):
        return rec(n, 2, ft, lang, text, **kw)

    return [
        # --- T3: UNCONSCIOUS ---
        r(1, "T3", "manglish", "Achan bodham ketti veenu, vilichittu unarunnilla. Njangal 4 per und, "
          "Karuvelikkunnu school road. Ambulance venam.",
          location="Karuvelikkunnu school road", headcount=4, asset="AMBULANCE", gold="CRITICAL",
          medical=["UNCONSCIOUS"], **SELF),
        r(2, "T3", "ml", "അമ്മയ്ക്ക് ബോധമില്ല, വിളിച്ചിട്ട് ഉണരുന്നില്ല. ഞങ്ങൾ മൂന്നുപേർ "
          "കൊച്ചുകടവ് പാലത്തിന് അടുത്തുള്ള വീട്ടിലാണ്. ആംബുലൻസ് വേണം.",
          location="കൊച്ചുകടവ് പാലം", headcount=3, asset="AMBULANCE", gold="CRITICAL",
          medical=["UNCONSCIOUS"], **SELF),
        r(3, "T3", "manglish", "Appooppan anangunnilla, bodham undo ennu ariyilla, shwasikkunnundu. "
          "Njangal 5 per, Thuruthipuram kavala.",
          location="Thuruthipuram kavala", headcount=5, asset=None, gold="CRITICAL",
          medical=["UNCONSCIOUS"], groups=["ELDERLY"], **SELF),
        r(4, "T3", "ml", "ഒരാളെ ഇപ്പോൾ വെള്ളത്തിൽ നിന്ന് വലിച്ചു കയറ്റി, ബോധമില്ല. കൊമ്പനാട് "
          "മാർക്കറ്റിന് മുന്നിലാണ്, ഞാൻ അടുത്ത് നിൽക്കുന്നുണ്ട്. ആംബുലൻസ് വേണം.",
          location="കൊമ്പനാട് മാർക്കറ്റ്", headcount=1, asset="AMBULANCE", gold="CRITICAL",
          medical=["UNCONSCIOUS"], other=["near-drowning"], **BYSTANDER),
        r(5, "T3", "manglish", "Ente chechi vilichu, avalude bharthavu bodham poyi kidakkunnu ennu "
          "paranju. Avar randu perum Pallikkunnu church nte aduthaanu. Njan Thrissur il aanu, "
          "avide ethan pattunnilla.",
          location="Pallikkunnu church", headcount=2, asset=None, gold="CRITICAL",
          medical=["UNCONSCIOUS"], **REMOTE_FAMILY),
        r(6, "T3", "ml", "ബോധം കെട്ടു വീണ കുട്ടി ഇതുവരെ ഉണർന്നിട്ടില്ല. ഞങ്ങൾ ആറുപേർ "
          "ഇടവനക്കാട് പള്ളിയുടെ ഹാളിലാണ്.",
          location="ഇടവനക്കാട് പള്ളി ഹാൾ", headcount=6, asset=None, gold="CRITICAL",
          medical=["UNCONSCIOUS"], groups=["CHILD"], **SELF),
        r(7, "T3", "manglish", "Ammachikku thala karakkam und, pakshe samsarikkunnundu. Njangal 3 "
          "per Kombanadu LP school il. Vellam muttolam.",
          location="Kombanadu LP school", headcount=3, asset=None, gold="URGENT",
          groups=["ELDERLY"], water="KNEE", **SELF),  # dizzy but talking: not UNCONSCIOUS
        r(8, "T3", "ml", "ഇന്ന് പുലർച്ചെ അച്ഛൻ ബോധം കെട്ടു വീണു, ഇപ്പോൾ കുഴപ്പമില്ല, "
          "സംസാരിക്കുന്നുണ്ട്. ഞങ്ങൾ നാലുപേർ വെള്ളാരപ്പിള്ളി റോഡിലാണ്.",
          location="വെള്ളാരപ്പിള്ളി റോഡ്", headcount=4, asset=None, gold="URGENT",
          **SELF),  # resolved within 24 h: no condition, gold URGENT
        # --- T3: BREATHING_DIFFICULTY ---
        r(9, "T3", "manglish", "Ammakku shwasam kittunnilla, valare budhimuttunnu. Njangal 2 per, "
          "Edakkunnu temple road. Ambulance please.",
          location="Edakkunnu temple road", headcount=2, asset="AMBULANCE", gold="CRITICAL",
          medical=["BREATHING_DIFFICULTY"], **SELF),
        r(10, "T3", "ml", "മോന് ശ്വാസം മുട്ടൽ കൂടുന്നു, ഇൻഹേലർ തീർന്നു. ഞങ്ങൾ മൂന്നുപേർ "
          "ചെറുകുന്നം കവലയ്ക്ക് അടുത്ത്.",
          location="ചെറുകുന്നം കവല", headcount=3, asset=None, gold="CRITICAL",
          medical=["BREATHING_DIFFICULTY"], **SELF),  # acute episode: not also CHRONIC_CARE
        r(11, "T3", "manglish", "Ammoommakku shwasam edukkan pattunnilla ennu thonnunnu, "
          "nenjil ninnu sound varunnu. Njangal ivide 4 perundu, Kuttippuzha bridge.",
          location="Kuttippuzha bridge", headcount=4, asset=None, gold="CRITICAL",
          medical=["BREATHING_DIFFICULTY"], groups=["ELDERLY"], **SELF),
        r(12, "T3", "ml", "അയൽവീട്ടിലെ വൃദ്ധന് ശ്വാസതടസ്സം, ഓക്സിജൻ സിലിണ്ടർ തീർന്നു. "
          "മാഞ്ഞാലിക്കര ലക്ഷം വീട് കോളനി. ആംബുലൻസ് വേണം.",
          location="മാഞ്ഞാലിക്കര ലക്ഷം വീട് കോളനി", headcount=1, asset="AMBULANCE",
          gold="CRITICAL", medical=["BREATHING_DIFFICULTY"], groups=["ELDERLY"],
          relation="THIRD_PARTY"),
        r(13, "T3", "manglish", "Achanu oxygen cylinder theernnu, ippo shwasam muttunnu ennu amma "
          "phone il paranju. Randu perum Chembarathy colony il aanu. Njan Muscat il aanu.",
          location="Chembarathy colony", headcount=2, asset=None, gold="CRITICAL",
          medical=["BREATHING_DIFFICULTY"], **REMOTE_FAMILY),
        r(14, "T3", "ml", "ആറുമാസമുള്ള കുഞ്ഞിന് ശ്വാസം എടുക്കാൻ ബുദ്ധിമുട്ട്, ചുണ്ട് "
          "നീലയാകുന്നു. ഞങ്ങൾ മൂന്നുപേർ പൂവത്തുംകടവ് ജംഗ്ഷനിൽ.",
          location="പൂവത്തുംകടവ് ജംഗ്ഷൻ", headcount=3, asset=None, gold="CRITICAL",
          medical=["BREATHING_DIFFICULTY"], groups=["INFANT"], **SELF),
        r(15, "T3", "manglish", "Ente bharyakku asthma attack, inhaler vellathil poyi. Njangal 3 "
          "per Kannankottu junction. Ambulance venam.",
          location="Kannankottu junction", headcount=3, asset="AMBULANCE", gold="CRITICAL",
          medical=["BREATHING_DIFFICULTY"], **SELF),
        r(16, "T3", "manglish", "Achanu asthma und, ippo kuzhappamilla, pakshe inhaler naale "
          "theerum. Njangal 4 per Eravathoor school il. Vellam aravolam.",
          location="Eravathoor school", headcount=4, asset=None, gold="URGENT",
          medical=["CHRONIC_CARE"], water="WAIST", **SELF),  # no attack now
        r(17, "T3", "ml", "ആർക്കും ശ്വാസം മുട്ടലോ പരിക്കോ ഇല്ല. ഞങ്ങൾ അഞ്ചുപേർ "
          "കോട്ടപ്പുറം വായനശാലയിൽ, മുട്ടോളം വെള്ളം. ക്യാമ്പിലേക്ക് മാറാൻ ട്രക്ക് വേണം.",
          location="കോട്ടപ്പുറം വായനശാല", headcount=5, asset="EVAC_TRUCK", gold="STABLE",
          water="KNEE", **SELF),  # negation
        # --- T3: CARDIAC ---
        r(18, "T3", "manglish", "Achanu nenju vedana, nannayi viyarkkunnu. Njangal 3 per "
          "Kizhakkekavu temple il. Ambulance venam vegam.",
          location="Kizhakkekavu temple", headcount=3, asset="AMBULANCE", gold="CRITICAL",
          medical=["CARDIAC"], **SELF),
        r(19, "T3", "ml", "അമ്മായിക്ക് നെഞ്ചുവേദന തുടങ്ങിയിട്ട് അര മണിക്കൂറായി. ഞങ്ങൾ "
          "രണ്ടുപേർ ചെമ്പകശ്ശേരി ലെയിനിൽ ആണ്.",
          location="ചെമ്പകശ്ശേരി ലെയിൻ", headcount=2, asset=None, gold="CRITICAL",
          medical=["CARDIAC"], **SELF),
        r(20, "T3", "manglish", "Heart attack aano ennu samshayam, ammavanu idathe kaiyil vedanayum "
          "nenjil bharavum. Njangal 5 per, Puthenchira kadavu.",
          location="Puthenchira kadavu", headcount=5, asset=None, gold="CRITICAL",
          medical=["CARDIAC"], **SELF),  # stated suspicion counts
        r(21, "T3", "ml", "ക്യാമ്പിലെ ഒരു വയോധികന് നെഞ്ചുവേദന. ഞാൻ ഇവിടത്തെ വളണ്ടിയർ ആണ്. "
          "കുന്നുകര ഗവൺമെന്റ് സ്കൂൾ ക്യാമ്പ്. ആംബുലൻസ് അയക്കണം.",
          location="കുന്നുകര ഗവൺമെന്റ് സ്കൂൾ ക്യാമ്പ്", headcount=1, asset="AMBULANCE",
          gold="CRITICAL", medical=["CARDIAC"], groups=["ELDERLY"], **BYSTANDER),
        r(22, "T3", "manglish", "Ammakku nenjil vedana ennu paranju vilichu, pinne phone cut aayi. "
          "Amma ottakku aanu, Kalappurakkal lane. Njan Kochi il aanu.",
          location="Kalappurakkal lane", headcount=1, asset=None, gold="CRITICAL",
          medical=["CARDIAC"], **REMOTE_FAMILY),
        r(23, "T3", "ml", "ഹൃദയാഘാതം ആണെന്ന് തോന്നുന്നു, അപ്പൂപ്പന് ശക്തമായ നെഞ്ചുവേദന. "
          "ഞങ്ങൾ നാലുപേർ മറ്റത്തിൽ കടവിന് അടുത്ത്. ബോട്ട് വേണം.",
          location="മറ്റത്തിൽ കടവ്", headcount=4, asset="RESCUE_BOAT", gold="CRITICAL",
          medical=["CARDIAC"], groups=["ELDERLY"], **SELF),
        r(24, "T3", "manglish", "Achan heart patient aanu, ippo kuzhappamilla, pakshe marunnu randu "
          "divasathekku kudi ulloo. Njangal 3 per Mattoorkunnu junction il.",
          location="Mattoorkunnu junction", headcount=3, asset=None, gold="URGENT",
          medical=["CHRONIC_CARE"], **SELF),  # known condition, no symptoms now
        r(25, "T3", "ml", "ഇന്ന് രാവിലെ അമ്മയ്ക്ക് നെഞ്ചുവേദന വന്നു, ഇപ്പോൾ മാറി. ഞങ്ങൾ "
          "മൂന്നുപേർ പാറക്കടവ് അമ്പലത്തിന് അടുത്ത്.",
          location="പാറക്കടവ് അമ്പലം", headcount=3, asset=None, gold="URGENT",
          **SELF),  # resolved within 24 h
        # --- T3: BLEEDING ---
        r(26, "T3", "manglish", "Aniyante kaalil valiya murivu, chora nilkkunnilla. Vellathinadiyil "
          "kambi kondu murinjathaanu. Njangal 4 per Mookkanparambu il.",
          location="Mookkanparambu", headcount=4, asset=None, gold="CRITICAL",
          medical=["BLEEDING", "INJURY"], **SELF),
        r(27, "T3", "ml", "അച്ഛൻ വീണു, തലയിൽ നിന്ന് രക്തം വരുന്നു. ഞങ്ങൾ മൂന്നുപേർ "
          "ആലങ്ങാട്ടുകര സ്കൂളിന് സമീപം. ആംബുലൻസ് വേണം.",
          location="ആലങ്ങാട്ടുകര സ്കൂൾ", headcount=3, asset="AMBULANCE", gold="CRITICAL",
          medical=["BLEEDING", "INJURY"], **SELF),
        r(28, "T3", "manglish", "Nadakkavu bridge inte aduthu oru chettan veenu, kaiyil ninnu "
          "orupaadu chora pokunnu. Njan avide thanne und.",
          location="Nadakkavu bridge", headcount=1, asset=None, gold="CRITICAL",
          medical=["BLEEDING", "INJURY"], **BYSTANDER),
        r(29, "T3", "ml", "പ്രസവം കഴിഞ്ഞിട്ട് മൂന്നു ദിവസം ആയി, ഭാര്യക്ക് രക്തസ്രാവം "
          "നിൽക്കുന്നില്ല. ഞാനും ഭാര്യയും കുഞ്ഞും ആണ്. വടക്കേമുറി പള്ളിക്ക് അടുത്ത്. "
          "ആംബുലൻസ് വേണം.",
          location="വടക്കേമുറി പള്ളി", headcount=3, asset="AMBULANCE", gold="CRITICAL",
          medical=["BLEEDING"], groups=["INFANT"], **SELF),  # newborn: INFANT; not PREGNANT now
        r(30, "T3", "manglish", "Ente aniyathikku mookkil ninnu chora nilkkunnilla ennu avalude "
          "bharthavu vilichu paranju. Avar randu perum Kuzhuppilly kadavu il aanu.",
          location="Kuzhuppilly kadavu", headcount=2, asset=None, gold="CRITICAL",
          medical=["BLEEDING"], relation="HOUSEHOLD"),  # reporter's position not stated
        r(31, "T3", "ml", "കുട്ടിയുടെ കാലിൽ കുപ്പിച്ചില്ല് കയറി, ചോര ഒഴുകുന്നു. ഞങ്ങൾ "
          "അഞ്ചുപേർ തിരുവാലൂർ റോഡിൽ.",
          location="തിരുവാലൂർ റോഡ്", headcount=5, asset=None, gold="CRITICAL",
          medical=["BLEEDING", "INJURY"], groups=["CHILD"], **SELF),
        r(32, "T3", "manglish", "Kaiyil cheriya oru murivu und, chora ninnu. Vere kuzhappam onnum "
          "illa. Njangal 6 per Kaithappara colony il, muttolam vellam.",
          location="Kaithappara colony", headcount=6, asset=None, gold="STABLE",
          medical=["INJURY"], water="KNEE", **SELF),  # minor cut, stopped: INJURY only
        r(33, "T3", "ml", "അനിയന്റെ കാൽ ഒടിഞ്ഞു, നീരുണ്ട്, ചോരയില്ല. ഞങ്ങൾ മൂന്നുപേർ "
          "പെരിങ്ങാല ജംഗ്ഷനിൽ. ആംബുലൻസ് വേണം.",
          location="പെരിങ്ങാല ജംഗ്ഷൻ", headcount=3, asset="AMBULANCE", gold="URGENT",
          medical=["INJURY"], **SELF),
        # --- T3: other medical phrasing ---
        r(34, "T3", "manglish", "Achante dialysis innale cheyyendathaayirunnu, mudangi. Achan "
          "ksheenichu, pakshe bodham und. Njangal 2 per Vellanadu church nte aduthu.",
          location="Vellanadu church", headcount=2, asset=None, gold="URGENT",
          medical=["CHRONIC_CARE"], **SELF),  # weak but conscious: not UNCONSCIOUS
        r(35, "T3", "ml", "മോനെ പാമ്പ് കടിച്ചോ എന്ന് സംശയം, കാലിൽ രണ്ട് പാടുണ്ട്. ഞങ്ങൾ "
          "നാലുപേർ ചൂർണിക്കര കനാൽ റോഡിൽ.",
          location="ചൂർണിക്കര കനാൽ റോഡ്", headcount=4, asset=None, gold="CRITICAL",
          other=["suspected snakebite"], **SELF),  # guideline change 3
        r(36, "T3", "manglish", "Bharyakku prasava vedana thudangi, 9 maasam aayi. Njangal 3 per "
          "Ponnarimangalam kadavu il. Boat venam.",
          location="Ponnarimangalam kadavu", headcount=3, asset="RESCUE_BOAT", gold="URGENT",
          other=["in labour"], groups=["PREGNANT"], **SELF),
        r(37, "T3", "ml", "വെള്ളത്തിലേക്ക് വൈദ്യുതി കമ്പി പൊട്ടിവീണു, ചേട്ടന് ഷോക്കടിച്ചു. "
          "ഇപ്പോൾ സംസാരിക്കുന്നുണ്ട്. ഞങ്ങൾ മൂന്നുപേർ തോട്ടുമുഖം പാലത്തിന് അടുത്ത്.",
          location="തോട്ടുമുഖം പാലം", headcount=3, asset=None, gold="CRITICAL",
          other=["electric shock"], **SELF),
        r(38, "T3", "manglish", "Kuttikku kadutha pani, virakkunnu. Njangal 4 per "
          "Chengamanadu school il.",
          location="Chengamanadu school", headcount=4, asset=None, gold="URGENT",
          other=["high fever"], groups=["CHILD"], **SELF),
        # --- T4: rising / falling / unchanged, all three languages ---
        r(39, "T4", "en", "The water is climbing fast. It was at our ankles an hour ago and now it "
          "is at our knees. 5 of us at Pazhayidam lane.",
          location="Pazhayidam lane", headcount=5, asset=None, gold="URGENT",
          water="KNEE", trend="RISING", **SELF),
        r(40, "T4", "en", "Water is going down slowly at Thannippara junction, now about ankle "
          "deep. Three of us, all okay, we can wait.",
          location="Thannippara junction", headcount=3, asset=None, gold="STABLE",
          water="ANKLE", trend="FALLING", **SELF),
        r(41, "T4", "en", "The level has not changed since morning, still waist deep in the house. "
          "Family of 4 near Kozhippilly church.",
          location="Kozhippilly church", headcount=4, asset=None, gold="URGENT",
          water="WAIST", trend="STABLE"),
        r(42, "T4", "en", "If the water keeps rising we will have to climb onto the roof. Right "
          "now it is at knee level. 3 adults at Olipuram colony.",
          location="Olipuram colony", headcount=3, asset=None, gold="STABLE",
          water="KNEE", **SELF),  # hypothetical: no trend, not stranded
        r(43, "T4", "en", "It rose all night but has been draining since 6 am. Chest deep earlier, "
          "now around the waist. Two of us and my mother, who is 80, at Maradikunnu.",
          location="Maradikunnu", headcount=3, asset=None, gold="URGENT",
          groups=["ELDERLY"], water="WAIST", trend="FALLING", **SELF),  # latest level
        r(44, "T4", "en", "My brother says the water is getting higher every few minutes at his "
          "house in Kalloorkkad. He is alone there.",
          location="Kalloorkkad", headcount=1, asset=None, gold="UNKNOWN",
          trend="RISING", relation="HOUSEHOLD"),
        r(45, "T4", "en", "The water is receding at Kottappuram ferry, but my father is bedridden "
          "and we cannot move him. It is still about 3 feet in the room. We are 3 people.",
          location="Kottappuram ferry", headcount=3, asset=None, gold="URGENT",
          groups=["BEDRIDDEN"], water="WAIST", trend="FALLING", **SELF),
        r(46, "T4", "en", "Water rising steadily at Kuzhikkattu lane, now chest high. 6 people "
          "including two kids.",
          location="Kuzhikkattu lane", headcount=6, asset=None, gold="CRITICAL",
          groups=["CHILD"], water="CHEST", trend="RISING"),
        r(47, "T4", "en", "No change in the water since last night, it is ankle deep in the hostel "
          "at Pottakuzhy. 15 students, all fine, want to go home.",
          location="Pottakuzhy", headcount=15, asset=None, gold="STABLE",
          water="ANKLE", trend="STABLE"),
        r(48, "T4", "en", "Water started coming up through the floor drains a few minutes ago. "
          "4 of us in the shop at Vattappara junction.",
          location="Vattappara junction", headcount=4, asset=None, gold="UNKNOWN",
          trend="RISING", **SELF),  # ongoing entry (guideline change 4)
        r(49, "T4", "en", "The water has been dropping for two hours, now about 1 foot in the "
          "house. 3 of us at Edathuruthy, no one hurt.",
          location="Edathuruthy", headcount=3, asset=None, gold="STABLE",
          water="KNEE", trend="FALLING", **SELF),
        r(50, "T4", "en", "Water is increasing very quickly at Thazhathangadi road, already 4 "
          "feet. My wife is pregnant. Two of us. Need a boat.",
          location="Thazhathangadi road", headcount=2, asset="RESCUE_BOAT", gold="CRITICAL",
          groups=["PREGNANT"], water="CHEST", trend="RISING", **SELF),
        r(51, "T4", "ml", "വെള്ളം പൊങ്ങിക്കൊണ്ടിരിക്കുന്നു, ഇപ്പോൾ അരയോളം ഉണ്ട്. ഞങ്ങൾ "
          "നാലുപേർ മണലിമുക്ക് ജംഗ്ഷനിൽ.",
          location="മണലിമുക്ക് ജംഗ്ഷൻ", headcount=4, asset=None, gold="URGENT",
          water="WAIST", trend="RISING", **SELF),
        r(52, "T4", "ml", "വെള്ളം ഇറങ്ങിത്തുടങ്ങി, ഇപ്പോൾ മുട്ടോളം മാത്രം. ഞങ്ങൾ ഏഴുപേർ "
          "കരിമ്പാടം സ്കൂളിൽ, ആർക്കും കുഴപ്പമില്ല.",
          location="കരിമ്പാടം സ്കൂൾ", headcount=7, asset=None, gold="STABLE",
          water="KNEE", trend="FALLING", **SELF),
        r(53, "T4", "ml", "രാവിലെ മുതൽ വെള്ളം അങ്ങനെ തന്നെ നിൽക്കുന്നു, നെഞ്ചോളം ഉണ്ട്. "
          "ഞങ്ങൾ മൂന്നുപേർ സ്റ്റെയർകേസിൽ നിൽക്കുന്നു. പുതുവാശ്ശേരി ലെയിൻ.",
          location="പുതുവാശ്ശേരി ലെയിൻ", headcount=3, asset=None, gold="URGENT",
          water="CHEST", trend="STABLE", **SELF),
        r(54, "T4", "ml", "വെള്ളം കൂടിയാൽ ഞങ്ങൾക്ക് പുറത്തിറങ്ങാൻ പറ്റില്ല. ഇപ്പോൾ "
          "കണങ്കാൽ വരെ മാത്രം. ആറുപേർ ഉണ്ട്, കുറുമശ്ശേരി ചന്തയ്ക്ക് അടുത്ത്.",
          location="കുറുമശ്ശേരി ചന്ത", headcount=6, asset=None, gold="STABLE",
          water="ANKLE", **SELF),  # hypothetical: no trend
        r(55, "T4", "ml", "വെള്ളം താഴുന്നുണ്ട്, പക്ഷേ അമ്മൂമ്മയ്ക്ക് നടക്കാൻ ബുദ്ധിമുട്ടാണ്. "
          "ഇപ്പോഴും അരയോളം വെള്ളം. ഞങ്ങൾ മൂന്നുപേർ എടയാർ പള്ളിക്ക് സമീപം.",
          location="എടയാർ പള്ളി", headcount=3, asset=None, gold="URGENT",
          groups=["ELDERLY"], water="WAIST", trend="FALLING", **SELF),  # change 6
        r(56, "T4", "manglish", "Vellam pongi varunnu, ippo nenjolam aayi! Njangal 3 per "
          "Kadungalloor kadavu il. Boat venam.",
          location="Kadungalloor kadavu", headcount=3, asset="RESCUE_BOAT", gold="CRITICAL",
          water="CHEST", trend="RISING", **SELF),
        r(57, "T4", "manglish", "Vellam irangi thudangi, ippo kanankaal vare ulloo. Njangal 4 per "
          "Uliyannoor junction il, aarkkum prashnamilla.",
          location="Uliyannoor junction", headcount=4, asset=None, gold="STABLE",
          water="ANKLE", trend="FALLING", **SELF),
        r(58, "T4", "manglish", "Vellam koodunnum illa kurayunnum illa, aravolam und. Ammoommayum "
          "njangalum aayi 4 per, Thottakkattukara il.",
          location="Thottakkattukara", headcount=4, asset=None, gold="URGENT",
          groups=["ELDERLY"], water="WAIST", trend="STABLE", **SELF),
        r(59, "T4", "manglish", "Vellam koodiyal ammaye kondu pokan pattilla, amma kidappilaanu. "
          "Ippo muttolam und. Njangal 2 per, Pallippuram road.",
          location="Pallippuram road", headcount=2, asset=None, gold="URGENT",
          groups=["BEDRIDDEN"], water="KNEE", **SELF),  # hypothetical: no trend
        r(60, "T4", "manglish", "Vellam ippozhum kayari varunnu, veettil muttolam ethi. Njangal 4 "
          "per, randu kuttikal und. Koonammavu road.",
          location="Koonammavu road", headcount=4, asset=None, gold="URGENT",
          groups=["CHILD"], water="KNEE", trend="RISING", **SELF),
    ]


BATCHES = {1: batch_1, 2: batch_2}

# Annotator notes shown on the owner review sheet (not part of the records).
NOTES = {
    "sft-v4-new-b2-007": "Dizzy but talking: not UNCONSCIOUS.",
    "sft-v4-new-b2-008": "Fainted this morning, fine now: resolved within 24 h, so no "
    "condition and gold URGENT (guideline, resolved conditions).",
    "sft-v4-new-b2-010": "Acute attack after the inhaler ran out: BREATHING_DIFFICULTY only, "
    "not also CHRONIC_CARE. 'Monu' (son) adds no CHILD.",
    "sft-v4-new-b2-012": "Neighbour reporting; position not stated, so on_scene null.",
    "sft-v4-new-b2-016": "Known asthma, no attack now: CHRONIC_CARE.",
    "sft-v4-new-b2-020": "Stated suspicion of a heart attack counts (principle 3).",
    "sft-v4-new-b2-021": "Camp volunteer on site: THIRD_PARTY, on scene.",
    "sft-v4-new-b2-024": "Known heart disease, medicine running low, no symptoms: CHRONIC_CARE.",
    "sft-v4-new-b2-025": "Chest pain this morning, gone now: no condition, gold URGENT.",
    "sft-v4-new-b2-029": "Postpartum bleeding. The newborn is INFANT; the mother is no longer "
    "PREGNANT.",
    "sft-v4-new-b2-030": "Told by phone, but the reporter's own position is not stated: "
    "on_scene null.",
    "sft-v4-new-b2-032": "Minor cut that has stopped bleeding, knee-deep water, adults: "
    "INJURY only; gold STABLE (safe for now). Owner check: STABLE or URGENT?",
    "sft-v4-new-b2-034": "Missed dialysis, weak but conscious: CHRONIC_CARE, not UNCONSCIOUS.",
    "sft-v4-new-b2-035": "Suspected snakebite counts (guideline change 3).",
    "sft-v4-new-b2-036": "Labour: gold URGENT; the rules make any other_conditions entry "
    "CRITICAL (accepted over-triage, rules v2).",
    "sft-v4-new-b2-039": "Fast-rising knee-deep water: gold URGENT; the rules cannot prove "
    "STABLE while rising, so UNKNOWN.",
    "sft-v4-new-b2-042": "'If the water keeps rising' is hypothetical: trend null, and not "
    "stranded at height.",
    "sft-v4-new-b2-043": "Several levels: use the latest (waist, draining).",
    "sft-v4-new-b2-044": "'My brother says': the reporter's position is not stated, so "
    "on_scene null.",
    "sft-v4-new-b2-045": "Bedridden father, 3 ft and falling: gold URGENT; the rules make "
    "BEDRIDDEN with water at WAIST CRITICAL. Owner check: URGENT or CRITICAL?",
    "sft-v4-new-b2-048": "Water coming up through the drains is ongoing entry: RISING "
    "(guideline change 4).",
    "sft-v4-new-b2-054": "'If the water rises' is hypothetical: trend null.",
    "sft-v4-new-b2-055": "'Difficulty walking' adds no group (guideline change 6); "
    "grandmother gives ELDERLY.",
    "sft-v4-new-b2-059": "'If the water rises' is hypothetical: trend null.",
}


def review_cases(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Records in the case shape the review sheet expects."""
    cases = []
    for record in records:
        target = record["target"]
        expected = {key: target[key] for key in (
            "victim_location", "headcount", "required_asset", "hazards", "needs_human_review",
            "victim_profile")}
        cases.append({"id": record["id"], "category": record["failure_type"],
                      "language": record["language"], "input": record["input"],
                      "expected": expected, "expected_urgency": record["expected_urgency"],
                      "annotator_note": NOTES.get(record["id"], "")})
    return cases


def build_records(batches: tuple[int, ...] = tuple(BATCHES)) -> list[dict[str, Any]]:
    return [record for number in batches for record in BATCHES[number]()]


def separation_problems(records: list[dict[str, Any]]) -> list[str]:
    """No input, place or 8-word sequence shared with any evaluation set."""
    inputs, places = set(), set()
    for path in EVAL_SETS:
        for case in load_cases(path):
            inputs.add(case["input"])
            places.add((case["expected"]["victim_location"] or "").casefold())
    places.discard("")
    eval_grams = set().union(*(ngrams(text, DATASET_NGRAM) for text in inputs))
    problems = []
    seen = set()
    for record in records:
        if record["input"] in inputs:
            problems.append(f"{record['id']}: input is in an evaluation set")
        if record["input"] in seen:
            problems.append(f"{record['id']}: duplicate input")
        seen.add(record["input"])
        place = (record["target"]["victim_location"] or "").casefold()
        if place in places:
            problems.append(f"{record['id']}: place used in an evaluation set: {place}")
        shared = ngrams(record["input"], DATASET_NGRAM) & eval_grams
        if shared:
            problems.append(f"{record['id']}: shares wording with an evaluation set")
    return problems


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--review-sheet", type=int, metavar="BATCH",
                        help="also write the owner review sheet for this batch")
    args = parser.parse_args()
    records = build_records()
    validate_records(records)
    problems = separation_problems(records)
    if problems:
        raise SystemExit("\n".join(problems))
    write_cases(records, args.output)
    print(f"Wrote {len(records)} records to {args.output}")
    print(dict(Counter((r["batch"], r["language"]) for r in records)))
    print(dict(Counter(r["expected_urgency"] for r in records)))
    if args.review_sheet:
        batch = [r for r in records if r["batch"] == args.review_sheet]
        sheet = DATA / f"triage-sft-v4-batch{args.review_sheet}-review-guide.md"
        sheet.write_text(render(
            review_cases(batch), f"SFT-V4 new records, batch {args.review_sheet}",
            f"**Batch {args.review_sheet} ({len(batch)} records): awaiting AI first pass and "
            "owner review.** Guideline changes 5 and 6 are in "
            "`triage-annotation-guidelines-v3.2-changes.md`.",
            "src/crisisgraph_ml/triage_sft_v4_new.py"), encoding="utf-8")
        print(f"Wrote review sheet to {sheet}")


if __name__ == "__main__":
    main()
