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
OWNER_REVIEWED_BATCHES = {1, 2, 3}

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
          water="ABOVE_HEAD"),  # "shift them": coordinator, relation unknown (change 5)
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

    Owner policy (batch 2 review): bleeding is gold CRITICAL when it is heavy,
    will not stop, is postpartum or is a fresh head wound, otherwise URGENT
    (guideline change 9; the label stays BLEEDING and the rules still give
    CRITICAL). Labour is gold CRITICAL, as
    in the frozen v3.1.0 set (guideline change 7). A bedridden person who
    cannot be moved in water at bed height is CRITICAL even if it is falling.
    """
    def r(n, ft, lang, text, **kw):
        return rec(n, 2, ft, lang, text, **kw)

    return [
        # --- T3: UNCONSCIOUS ---
        r(1, "T3", "manglish", "Achan bodham kettu veenu, vilichittu unarunnilla. Njangal 4 per und, "
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
          location="ഇടവനക്കാട് പള്ളിയുടെ ഹാൾ", headcount=6, asset=None, gold="CRITICAL",
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
          location="Kuzhuppilly kadavu", headcount=2, asset=None, gold="URGENT",
          medical=["BLEEDING"], relation="HOUSEHOLD"),  # reporter's position not stated
        r(31, "T3", "ml", "കുട്ടിയുടെ കാലിൽ കുപ്പിച്ചില്ല് കയറി, ചോര ഒഴുകുന്നു. ഞങ്ങൾ "
          "അഞ്ചുപേർ തിരുവാലൂർ റോഡിൽ.",
          location="തിരുവാലൂർ റോഡ്", headcount=5, asset=None, gold="URGENT",
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
          location="Ponnarimangalam kadavu", headcount=3, asset="RESCUE_BOAT", gold="CRITICAL",
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
          location="Olipuram colony", headcount=3, asset=None, gold="URGENT",
          water="KNEE", trend="RISING", **SELF),  # "keeps rising": rising now (change 8)
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
          location="Kottappuram ferry", headcount=3, asset=None, gold="CRITICAL",
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


def batch_3() -> list[dict[str, Any]]:
    """T5 groups in the right field; T6 groups only when stated; T7 INJURY next
    to a more serious condition.

    T5: bedridden, wheelchair, blind, "cannot walk without help" and pregnancy
    are groups, never ``other_conditions``; dialysis, insulin and oxygen are
    CHRONIC_CARE, not groups. T6 pairs a bare relation ("my father", "my son",
    "makkal") with the same relation plus a stated age or a relation that states
    age (grandmother, old age home); "old house" or "college students" give no
    group. T7 keeps INJURY beside the critical category and drops it under
    negation ("fell but not hurt"). Bleeding gold follows guideline change 9.

    Owner policy (batch 3 review): autism is DISABLED (guideline change 10);
    ankle-deep water with only a vulnerable group stated (wheelchair user,
    children, pregnancy) is gold STABLE, while pregnancy in knee-deep water
    stays URGENT.
    """
    def r(n, ft, lang, text, **kw):
        return rec(n, 3, ft, lang, text, **kw)

    return [
        # --- T5: groups in the right field ---
        r(1, "T5", "en", "My mother has been bedridden since her stroke. The water is knee deep in "
          "her room now. We are 3 at Pallarimangalam lane.",
          location="Pallarimangalam lane", headcount=3, asset=None, gold="URGENT",
          groups=["BEDRIDDEN"], water="KNEE", **SELF),  # past stroke, no treatment: no CHRONIC_CARE
        r(2, "T5", "manglish", "Ammoomma kidappilaanu, vellam muttolam aayi. Njangal 4 per, "
          "Chovvara kadavu.",
          location="Chovvara kadavu", headcount=4, asset=None, gold="URGENT",
          groups=["ELDERLY", "BEDRIDDEN"], water="KNEE", **SELF),
        r(3, "T5", "ml", "അച്ഛൻ കിടപ്പുരോഗിയാണ്, അരയോളം വെള്ളം ഉണ്ട്. ഞങ്ങൾ മൂന്നുപേർ "
          "കൂവപ്പടി റോഡിൽ.",
          location="കൂവപ്പടി റോഡ്", headcount=3, asset=None, gold="CRITICAL",
          groups=["BEDRIDDEN"], water="WAIST", **SELF),  # bed height (batch 2 policy)
        r(4, "T5", "en", "My brother uses a wheelchair. Two of us at Thekkumbhagam colony, the "
          "water is ankle deep.",
          location="Thekkumbhagam colony", headcount=2, asset=None, gold="STABLE",
          groups=["DISABLED"], water="ANKLE", **SELF),
        r(5, "T5", "en", "My uncle is blind and lives alone at Kappoor junction. Water has come into "
          "his house and he cannot tell how deep it is. I am in Kollam.",
          location="Kappoor junction", headcount=1, asset=None, gold="URGENT",
          groups=["DISABLED"], **REMOTE_FAMILY),
        r(6, "T5", "manglish", "Ente makan autistic aanu, vellam kandu pedichu irikkukayaanu. "
          "Njangal 3 per Pazhamthottam church nte aduthu.",
          location="Pazhamthottam church", headcount=3, asset=None, gold="UNKNOWN",
          groups=["DISABLED"], **SELF),  # developmental disability; "makan" adds no CHILD
        r(7, "T5", "en", "I am the warden of the Karimugal old age home. 12 residents, three of them "
          "in wheelchairs. Water is waist deep on the ground floor and rising. Send an evacuation "
          "truck.",
          location="Karimugal old age home", headcount=12, asset="EVAC_TRUCK", gold="CRITICAL",
          groups=["ELDERLY", "DISABLED"], water="WAIST", trend="RISING",
          relation="THIRD_PARTY"),  # role stated, position not: on_scene null
        r(8, "T5", "ml", "എട്ടുമാസം ഗർഭിണിയായ ഭാര്യയും ഞാനും ആണ്. മുട്ടോളം വെള്ളം. "
          "ഐരാപുരം പള്ളിക്ക് അടുത്ത്.",
          location="ഐരാപുരം പള്ളി", headcount=2, asset=None, gold="URGENT",
          groups=["PREGNANT"], water="KNEE", **SELF),
        r(9, "T5", "en", "We have a 10-day-old baby with us. The water is at the doorstep but not "
          "inside yet. 3 of us at Ayyampilly lane.",
          location="Ayyampilly lane", headcount=3, asset=None, gold="STABLE",
          groups=["INFANT"], **SELF),  # no depth where they are: water null
        r(10, "T5", "manglish", "Achan dialysis patient aanu, naale dialysis und, pakshe road "
          "muzhuvan vellathil. Njangal 2 per, Vengola kavala.",
          location="Vengola kavala", headcount=2, asset=None, gold="URGENT",
          hazards=[h(None, "FLOODED", None)], medical=["CHRONIC_CARE"], **SELF),
        r(11, "T5", "en", "My grandfather is 91, bedridden and on oxygen. The power is gone and the "
          "cylinder has about 3 hours left. We are 4 at Mudickal school road.",
          location="Mudickal school road", headcount=4, asset=None, gold="URGENT",
          medical=["CHRONIC_CARE"], groups=["ELDERLY", "BEDRIDDEN"], **SELF),
        r(12, "T5", "ml", "അമ്മൂമ്മയ്ക്ക് കാഴ്ചയില്ല. ഞങ്ങൾ രണ്ടുപേർ, നെഞ്ചോളം വെള്ളം. "
          "ഓണമ്പിള്ളി കടവിന് അടുത്ത്.",
          location="ഓണമ്പിള്ളി കടവ്", headcount=2, asset=None, gold="CRITICAL",
          groups=["ELDERLY", "DISABLED"], water="CHEST", **SELF),
        r(13, "T5", "en", "There is a diabetic man here whose insulin got washed away. With him we "
          "are 5 at the Vallathode ferry shed.",
          location="Vallathode ferry shed", headcount=5, asset=None, gold="URGENT",
          medical=["CHRONIC_CARE"], **SELF),
        r(14, "T5", "manglish", "Oru kaikkunjum randu kuttikalum und, vellam aravolam. Njangal 5 "
          "per Kavalangad kavala.",
          location="Kavalangad kavala", headcount=5, asset=None, gold="CRITICAL",
          groups=["INFANT", "CHILD"], water="WAIST", **SELF),
        r(15, "T5", "en", "My father can't walk without help since his knee surgery. The water is "
          "knee deep and rising slowly. 3 of us at Nellimattom road.",
          location="Nellimattom road", headcount=3, asset=None, gold="URGENT",
          groups=["DISABLED"], water="KNEE", trend="RISING", **SELF),  # change 6
        r(16, "T5", "ml", "ഭിന്നശേഷിക്കാരനായ മകൻ ഉണ്ട്. താഴത്തെ നില മുങ്ങി, ഞങ്ങൾ "
          "നാലുപേർ മുകളിൽ. പുന്നോർക്കോട് ജംഗ്ഷൻ.",
          location="പുന്നോർക്കോട് ജംഗ്ഷൻ", headcount=4, asset=None, gold="CRITICAL",
          groups=["DISABLED"], water="ABOVE_HEAD", **SELF),
        r(17, "T5", "en", "An elderly couple, both over 80, at Pindimana church lane. Water at waist "
          "height. Their neighbour called me; I'm the ward member and I can't reach them.",
          location="Pindimana church lane", headcount=2, asset=None, gold="URGENT",
          groups=["ELDERLY"], water="WAIST", relation="THIRD_PARTY", on_scene=False),
        # --- T6: groups only when stated ---
        r(18, "T6", "en", "My son is stuck at his office at Thottuva junction with 3 colleagues. "
          "Water is all around the building. I'm at home in Kalady.",
          location="Thottuva junction", headcount=4, asset=None, gold="UNKNOWN",
          **REMOTE_FAMILY),  # "son": no CHILD
        r(19, "T6", "en", "My 6-year-old son and I are on the first floor; the ground floor is under "
          "water. Mekkadambu lane.",
          location="Mekkadambu lane", headcount=2, asset=None, gold="CRITICAL",
          groups=["CHILD"], water="ABOVE_HEAD", **SELF),
        r(20, "T6", "en", "My father is alone at home near Manjapra temple and the water is knee "
          "deep. I can't get there.",
          location="Manjapra temple", headcount=1, asset=None, gold="STABLE",
          water="KNEE", **REMOTE_FAMILY),  # "father": no ELDERLY
        r(21, "T6", "en", "My 78-year-old father is alone at home near Iravipuram temple and the "
          "water is knee deep. I can't get there.",
          location="Iravipuram temple", headcount=1, asset=None, gold="URGENT",
          groups=["ELDERLY"], water="KNEE", **REMOTE_FAMILY),
        r(22, "T6", "manglish", "Ente makal ulpade 30 college students hostel il kudungi kidakkunnu, "
          "ground floor il vellam keri. Kizhakkedathu hostel, Vazhappilly.",
          location="Kizhakkedathu hostel, Vazhappilly", headcount=30, asset=None, gold="UNKNOWN",
          relation="HOUSEHOLD"),  # college students, "makal": no CHILD
        r(23, "T6", "ml", "അച്ഛനും അമ്മയും വീട്ടിൽ ഒറ്റയ്ക്കാണ്, മുട്ടോളം വെള്ളം. "
          "തുറവുങ്കര ജംഗ്ഷൻ. ഞാൻ ബാംഗ്ലൂരിൽ ആണ്.",
          location="തുറവുങ്കര ജംഗ്ഷൻ", headcount=2, asset=None, gold="STABLE",
          water="KNEE", **REMOTE_FAMILY),
        r(24, "T6", "ml", "എൺപത് വയസ്സുള്ള അമ്മ വീട്ടിൽ ഒറ്റയ്ക്കാണ്, അരയോളം വെള്ളം. "
          "ചേലാമറ്റം പള്ളിക്ക് സമീപം. ഞാൻ ദുബായിൽ ആണ്.",
          location="ചേലാമറ്റം പള്ളി", headcount=1, asset=None, gold="URGENT",
          groups=["ELDERLY"], water="WAIST", **REMOTE_FAMILY),
        r(25, "T6", "en", "Our house is very old and leaks badly. 4 of us inside at Kottayil lane, "
          "the water is at our knees.",
          location="Kottayil lane", headcount=4, asset=None, gold="STABLE",
          water="KNEE", **SELF),  # "old house": no ELDERLY
        r(26, "T6", "en", "20 residents of the Karunalayam old age home at Kizhakkumuri. Water is "
          "entering the hall.",
          location="Karunalayam old age home", headcount=20, asset=None,
          gold="URGENT", groups=["ELDERLY"], trend="RISING"),  # change 4
        r(27, "T6", "manglish", "Ente bharya garbhini aanu, 7 maasam. Njangal 2 per Odakkali "
          "junction il, vellam kanankaal vare.",
          location="Odakkali junction", headcount=2, asset=None, gold="STABLE",
          groups=["PREGNANT"], water="ANKLE", **SELF),
        r(28, "T6", "manglish", "Ente bharyayum njanum mathram, vellam muttolam, kuzhappamilla. "
          "Rayamangalam LP school.",
          location="Rayamangalam LP school", headcount=2, asset=None, gold="STABLE",
          water="KNEE", **SELF),  # "bharya": no group
        r(29, "T6", "en", "We are 3 adults and 2 kids at Edamalayar bus stop. The water is ankle "
          "deep.",
          location="Edamalayar bus stop", headcount=5, asset=None, gold="STABLE",
          groups=["CHILD"], water="ANKLE", **SELF),
        r(30, "T6", "en", "My little brother and I are stuck at the tuition centre near Panichayam. "
          "The water is up to my waist.",
          location="Panichayam", headcount=2, asset=None, gold="URGENT",
          water="WAIST", **SELF),  # "little brother": no stated age, no CHILD
        r(31, "T6", "ml", "മക്കൾ രണ്ടുപേരും ഞാനും ആണ് വീട്ടിൽ. അരയോളം വെള്ളം ഉണ്ട്. "
          "മാറമ്പിള്ളി കനാൽ റോഡ്.",
          location="മാറമ്പിള്ളി കനാൽ റോഡ്", headcount=3, asset=None, gold="URGENT",
          water="WAIST", **SELF),  # "makkal" (offspring) states no age
        r(32, "T6", "manglish", "Valiyachan ottakku aanu, vellam muttolam und. Asamannoor "
          "church nte aduthulla veedu.",
          location="Asamannoor church", headcount=1, asset=None, gold="STABLE",
          water="KNEE", relation="HOUSEHOLD"),  # uncle: relation states no age
        r(33, "T6", "en", "Two schoolgirls are stuck at the bus shelter near Kombanattu junction, "
          "water at their knees. I can see them from my shop.",
          location="Kombanattu junction", headcount=2, asset=None, gold="URGENT",
          groups=["CHILD"], water="KNEE", **BYSTANDER),
        r(34, "T6", "en", "Me, my husband and our 3-month-old daughter. Water is waist deep and "
          "rising. Kodanad road.",
          location="Kodanad road", headcount=3, asset=None, gold="CRITICAL",
          groups=["INFANT"], water="WAIST", trend="RISING", **SELF),
        # --- T7: INJURY next to a more serious condition ---
        r(35, "T7", "en", "My husband just fell from the ladder and his head is bleeding. We are 2 "
          "at Kurichilakode. Need an ambulance.",
          location="Kurichilakode", headcount=2, asset="AMBULANCE", gold="CRITICAL",
          medical=["BLEEDING", "INJURY"], **SELF),  # fresh head wound (change 9)
        r(36, "T7", "en", "A wall collapsed on a man at Vayalkara market. His leg is crushed and he "
          "is not responding. I'm one of the people trying to lift the wall.",
          location="Vayalkara market", headcount=1, asset=None, gold="CRITICAL",
          medical=["UNCONSCIOUS", "INJURY"], other=["trapped under debris"], **BYSTANDER),
        r(37, "T7", "manglish", "Achante kaal odinju, pinne shwasam muttalum und. Njangal 3 per "
          "Mazhuvannoor kavala.",
          location="Mazhuvannoor kavala", headcount=3, asset=None, gold="CRITICAL",
          medical=["BREATHING_DIFFICULTY", "INJURY"], **SELF),
        r(38, "T7", "ml", "ഗ്യാസ് സിലിണ്ടർ പൊട്ടി, ചേട്ടന് പൊള്ളലേറ്റു, ശ്വാസം "
          "കിട്ടുന്നില്ല. ഞങ്ങൾ മൂന്നുപേർ കീരംപാറ റോഡിൽ.",
          location="കീരംപാറ റോഡ്", headcount=3, asset=None, gold="CRITICAL",
          medical=["INJURY", "BREATHING_DIFFICULTY"], **SELF),  # burn is INJURY
        r(39, "T7", "en", "My neighbour has a deep cut on his arm from a tin sheet and it won't stop "
          "bleeding. I'm with him at Ramallur lane.",
          location="Ramallur lane", headcount=1, asset=None, gold="CRITICAL",
          medical=["BLEEDING", "INJURY"], **BYSTANDER),
        r(40, "T7", "en", "My son broke his arm when he slipped on the stairs. Otherwise we are "
          "fine. 4 of us at Nedungapra, water at the knees.",
          location="Nedungapra", headcount=4, asset=None, gold="URGENT",
          medical=["INJURY"], water="KNEE", **SELF),  # fracture only; "son": no CHILD
        r(41, "T7", "manglish", "Ammakku thalayil murivu und, chora ninnu, pakshe ippo nenju "
          "vedana thudangi. Njangal 2 per Kuttampuzha road.",
          location="Kuttampuzha road", headcount=2, asset=None, gold="CRITICAL",
          medical=["INJURY", "CARDIAC"], **SELF),  # bleeding stopped: INJURY, not BLEEDING
        r(42, "T7", "ml", "അച്ഛൻ വീണു, പക്ഷേ പരിക്കൊന്നുമില്ല. ഞങ്ങൾ നാലുപേർ, മുട്ടോളം "
          "വെള്ളം. വേങ്ങൂർ ലൈബ്രറിക്ക് അടുത്ത്.",
          location="വേങ്ങൂർ ലൈബ്രറി", headcount=4, asset=None, gold="STABLE",
          water="KNEE", **SELF),  # negation: no INJURY
        r(43, "T7", "en", "My cousin was hit by floating debris and is unconscious, with a gash on "
          "his forehead that is bleeding a lot. We are 2 at Pulluvazhy junction.",
          location="Pulluvazhy junction", headcount=2, asset=None, gold="CRITICAL",
          medical=["UNCONSCIOUS", "BLEEDING", "INJURY"], **SELF),
        r(44, "T7", "manglish", "Aniyanu shock adichu, kaiyil pollalum und. Njangal 3 per "
          "Thrikkariyoor kavala.",
          location="Thrikkariyoor kavala", headcount=3, asset=None, gold="CRITICAL",
          medical=["INJURY"], other=["electric shock"], **SELF),
        r(45, "T7", "en", "My mother fell and we think her hip is broken. She is 82 and in a lot of "
          "pain. We are 3 at Vadavucode church road.",
          location="Vadavucode church road", headcount=3, asset=None, gold="URGENT",
          medical=["INJURY"], groups=["ELDERLY"], **SELF),
        r(46, "T7", "ml", "മരം വീണ് ആന്റിയുടെ കാലിൽ മുറിവ്, രക്തം നിൽക്കുന്നില്ല. ഞങ്ങൾ "
          "മൂന്നുപേർ കോട്ടപ്പടി കവലയിൽ.",
          location="കോട്ടപ്പടി കവല", headcount=3, asset=None, gold="CRITICAL",
          medical=["BLEEDING", "INJURY"], **SELF),
        r(47, "T7", "en", "Our boat tipped over near Chelad ferry. One of us has a broken leg and "
          "another is struggling to breathe. We are 6 on the bank.",
          location="Chelad ferry", headcount=6, asset=None, gold="CRITICAL",
          medical=["INJURY", "BREATHING_DIFFICULTY"], **SELF),
        r(48, "T7", "manglish", "Ammakku kaiyil cheriya murivu, chora ninnu. Pakshe amma bodham "
          "ketta pole kidakkunnu, vilichittu mindunnilla. Njangal 3 per Keezhillam school.",
          location="Keezhillam school", headcount=3, asset=None, gold="CRITICAL",
          medical=["INJURY", "UNCONSCIOUS"], **SELF),
        r(49, "T7", "en", "He sprained his ankle, nothing serious, he can still walk. 3 of us at "
          "Payyal junction, water ankle deep.",
          location="Payyal junction", headcount=3, asset=None, gold="STABLE",
          medical=["INJURY"], water="ANKLE", **SELF),  # minor injury (as batch 2, 032)
        r(50, "T7", "ml", "കല്ല് വീണ് അപ്പൂപ്പന്റെ തല പൊട്ടി, ചോര ഒഴുകുന്നു. ഞങ്ങൾ "
          "നാലുപേർ ചെറുവട്ടൂർ കാവിന് അടുത്ത്.",
          location="ചെറുവട്ടൂർ കാവ്", headcount=4, asset=None, gold="CRITICAL",
          medical=["BLEEDING", "INJURY"], groups=["ELDERLY"], **SELF),  # fresh head wound
    ]


def batch_4() -> list[dict[str, Any]]:
    """T8 stranded at height; T9 stranded somewhere that is not a height; T10
    who is reporting.

    T8 follows guideline change 1: a roof, terrace, tree or car roof gets an
    ``other_conditions`` entry, and ``water_level`` stays null unless depth or
    flooding of the house is stated (then ROOF). A roof with no water stated is
    gold URGENT, as in the frozen set; a tree, or a roof over a flooded house,
    is CRITICAL. Going up only to reach phone signal, or "if the water rises we
    will go to the terrace", is not stranded. T9: stranded at a clinic, school,
    camp, station or upper floor gets no ``other_conditions``. T10 covers every
    relation value and on_scene only when the position is stated.
    """
    def r(n, ft, lang, text, **kw):
        return rec(n, 4, ft, lang, text, **kw)

    roof = ["stranded on a roof"]
    terrace = ["stranded on a terrace"]
    tree = ["stranded in a tree"]
    return [
        # --- T8: stranded at height ---
        r(1, "T8", "en", "Our house flooded and the 4 of us have climbed onto the roof. Kizhakkepuram "
          "chapel, please hurry.",
          location="Kizhakkepuram chapel", headcount=4, asset=None, gold="CRITICAL",
          other=roof, water="ROOF", **SELF),
        r(2, "T8", "en", "Five people are on the roof of the Padamugal reading room waiting for a "
          "boat.",
          location="Padamugal reading room", headcount=5, asset="RESCUE_BOAT", gold="URGENT",
          other=roof),  # no water stated: water null, gold as frozen 105
        r(3, "T8", "en", "A man has climbed a coconut tree near Vadakkumpuram canal to get out of "
          "the water. He has been up there for two hours. I'm watching from the bridge.",
          location="Vadakkumpuram canal", headcount=1, asset=None, gold="CRITICAL",
          other=tree, **BYSTANDER),
        r(4, "T8", "manglish", "Njangal 3 per terrace il aanu, chuttum vellam, thazhe ellam mungi. "
          "Kunnelpadi junction.",
          location="Kunnelpadi junction", headcount=3, asset=None, gold="CRITICAL",
          other=terrace, water="ROOF", **SELF),
        r(5, "T8", "ml", "ഞങ്ങൾ ആറുപേർ ടെറസിൽ കുടുങ്ങി, വീടിനുള്ളിൽ കഴുത്തോളം വെള്ളം. "
          "ആനിക്കാട് കവലയ്ക്ക് സമീപം.",
          location="ആനിക്കാട് കവല", headcount=6, asset=None, gold="CRITICAL",
          other=terrace, water="ROOF", **SELF),  # climbed up because the house is flooded
        r(6, "T8", "en", "Two kids and their grandmother have climbed up onto their roof at Chempu "
          "lane, the water is up to the eaves. I'm their neighbour and I can't get to them.",
          location="Chempu lane", headcount=3, asset=None, gold="CRITICAL",
          other=roof, groups=["CHILD", "ELDERLY"], water="ROOF",
          relation="THIRD_PARTY", on_scene=False),
        r(7, "T8", "manglish", "Oru appooppan maavinte mukalil kayari irikkunnu, thazhe nalla "
          "ozhukku. Njan bridge il ninnu kaanunnundu. Kottuvally kadavu.",
          location="Kottuvally kadavu", headcount=1, asset=None, gold="CRITICAL",
          other=tree, groups=["ELDERLY"], **BYSTANDER),
        r(8, "T8", "en", "Two of us are sitting on the roof of our car at Pallikkara junction, the "
          "water is above the doors.",
          location="Pallikkara junction", headcount=2, asset=None, gold="CRITICAL",
          other=["stranded on a car roof"], **SELF),  # "above the doors": no band
        r(9, "T8", "ml", "അഞ്ചുപേർ മാമ്പ്ര പള്ളിയുടെ മേൽക്കൂരയിൽ ബോട്ടിനായി "
          "കാത്തിരിക്കുന്നു.",
          location="മാമ്പ്ര പള്ളി", headcount=5, asset="RESCUE_BOAT", gold="URGENT",
          other=roof),  # no water stated
        r(10, "T8", "en", "I'm alone on the terrace of my house at Thanikkal. The stairs are under "
          "water and it's still rising.",
          location="Thanikkal", headcount=1, asset=None, gold="CRITICAL",
          other=terrace, water="ROOF", trend="RISING", **SELF),
        r(11, "T8", "manglish", "Ammayum njanum roof inte mele aanu, vellam ippo roof vare ethi. "
          "Pottenkadu colony.",
          location="Pottenkadu colony", headcount=2, asset=None, gold="CRITICAL",
          other=roof, water="ROOF", **SELF),
        r(12, "T8", "en", "My mother and father have gone up onto their roof at Vettampara. The ground floor "
          "is completely under water. I'm in Bahrain.",
          location="Vettampara", headcount=2, asset=None, gold="CRITICAL",
          other=roof, water="ROOF", **REMOTE_FAMILY),
        r(13, "T8", "ml", "ഒരു കുടുംബം മരത്തിൽ കയറി ഇരിക്കുകയാണ്, മൂന്നുപേർ, ഒരു "
          "കൈക്കുഞ്ഞും ഉണ്ട്. ഞാൻ എന്റെ വീടിന്റെ മുകളിൽ നിന്ന് കാണുന്നു. "
          "ചെങ്ങൽ തോടിന് അടുത്ത്.",
          location="ചെങ്ങൽ തോട്", headcount=3, asset=None, gold="CRITICAL",
          other=tree, groups=["INFANT"], **BYSTANDER),
        r(14, "T8", "en", "The water is at roof level in Kaduvettoor colony and at least 10 families "
          "are on their roofs. Send boats.",
          location="Kaduvettoor colony", headcount=None, asset="RESCUE_BOAT", gold="CRITICAL",
          other=["stranded on roofs"], water="ROOF"),
        r(15, "T8", "manglish", "Vellam koodiyal terrace il kayaraam ennu vicharikkunnu, ippo first "
          "floor il aanu. Njangal 5 per, thazhathe nila mungi. Nayathode kavala.",
          location="Nayathode kavala", headcount=5, asset=None, gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),  # terrace is hypothetical: not stranded
        r(16, "T8", "en", "We went up to the terrace only to get a phone signal; the water is ankle "
          "deep downstairs. 4 of us at Mattappilly.",
          location="Mattappilly", headcount=4, asset=None, gold="STABLE",
          water="ANKLE", **SELF),  # not stranded
        r(17, "T8", "ml", "താഴത്തെ നില മുങ്ങി, ഞങ്ങൾ മൂന്നുപേർ മുകളിലത്തെ നിലയിലാണ്. "
          "മേൽക്കൂരയിലേക്ക് കയറാൻ വഴിയില്ല. കുറ്റിപ്പുഴ ചന്ത.",
          location="കുറ്റിപ്പുഴ ചന്ത", headcount=3, asset=None, gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),  # upper floor, not on the roof
        # --- T9: stranded somewhere that is not a height ---
        r(18, "T9", "en", "20 people are stranded at the Kanjirathanam primary health centre, water "
          "knee deep in the building. Need an evacuation truck.",
          location="Kanjirathanam primary health centre", headcount=20, asset="EVAC_TRUCK",
          gold="STABLE", water="KNEE"),
        r(19, "T9", "manglish", "Ezhippuram school il 40 kuttikal kudungi, vellam school nte "
          "chuttum und, ullil kayariyittilla.",
          location="Ezhippuram school", headcount=40, asset=None, gold="STABLE",
          groups=["CHILD"]),  # not inside: water null (as batch 3, 009)
        r(20, "T9", "ml", "ഞങ്ങൾ പന്ത്രണ്ടുപേർ ക്യാമ്പിൽ കുടുങ്ങി, റോഡ് മുഴുവൻ വെള്ളത്തിൽ. "
          "ഭക്ഷണം തീരാറായി. കോടനാട് ഹൈസ്കൂൾ ക്യാമ്പ്.",
          location="കോടനാട് ഹൈസ്കൂൾ ക്യാമ്പ്", headcount=12, asset=None, gold="STABLE",
          hazards=[h(None, "FLOODED", None)], **SELF),
        r(21, "T9", "en", "About 60 passengers are stuck at Mattoor halt station; the road out is "
          "flooded.",
          location="Mattoor halt station", headcount=None, asset=None, gold="UNKNOWN",
          hazards=[h(None, "FLOODED", None)]),  # "about 60": not exact
        r(22, "T9", "en", "We are stuck inside the Kurumpilavu church hall. The water is chest deep "
          "and rising. 9 of us.",
          location="Kurumpilavu church hall", headcount=9, asset=None, gold="CRITICAL",
          water="CHEST", trend="RISING", **SELF),
        r(23, "T9", "manglish", "Njangal 30 per Vengoor mission hospital il aanu, generator nilachu. "
          "Ground floor il muttolam vellam.",
          location="Vengoor mission hospital", headcount=30, asset=None, gold="STABLE",
          water="KNEE", **SELF),
        r(24, "T9", "ml", "പുല്ലുവഴി ബസ് സ്റ്റാൻഡിൽ ഇരുപത് യാത്രക്കാർ കുടുങ്ങി, ചുറ്റും "
          "വെള്ളം.",
          location="പുല്ലുവഴി ബസ് സ്റ്റാൻഡ്", headcount=20, asset=None, gold="UNKNOWN"),
        r(25, "T9", "en", "25 of us are on the first floor of the Kaippattoor community hall; the "
          "ground floor is flooded to the ceiling.",
          location="Kaippattoor community hall", headcount=25, asset=None, gold="CRITICAL",
          water="ABOVE_HEAD", **SELF),
        r(26, "T9", "en", "Six of us are stranded at the petrol pump at Kalloor junction, with knee "
          "deep water around us.",
          location="Kalloor junction", headcount=6, asset=None, gold="STABLE",
          water="KNEE", **SELF),
        r(27, "T9", "manglish", "Chembarakki mosque il 50 per kudungi, chuttum vellam, ellarum safe "
          "aanu.",
          location="Chembarakki mosque", headcount=50, asset=None, gold="STABLE"),
        r(28, "T9", "ml", "ഇരിങ്ങോലിലെ അങ്കണവാടിയിൽ പത്ത് കുട്ടികളും രണ്ട് "
          "ടീച്ചർമാരും കുടുങ്ങി, മുട്ടോളം വെള്ളം.",
          location="ഇരിങ്ങോലിലെ അങ്കണവാടി", headcount=12, asset=None, gold="URGENT",
          groups=["CHILD"], water="KNEE"),
        r(29, "T9", "ml", "ഞങ്ങൾ പത്തുപേർ ഓഫീസിന്റെ രണ്ടാം നിലയിൽ ഉണ്ട്, താഴെ "
          "നെഞ്ചോളം വെള്ളം. വല്ലം കടവ് റോഡ്.",
          location="വല്ലം കടവ് റോഡ്", headcount=10, asset=None, gold="URGENT",
          water="CHEST", **SELF),
        r(30, "T9", "manglish", "Njangal 8 per Thrikkapady temple il aanu, vellam kanankaal vare, "
          "pedikkaanilla.",
          location="Thrikkapady temple", headcount=8, asset=None, gold="STABLE",
          water="ANKLE", **SELF),
        r(31, "T9", "en", "Eight workers are stuck at the Kuzhivelippady industrial estate. The water is waist "
          "deep in the factory and the access road is flooded.",
          location="Kuzhivelippady industrial estate", headcount=8, asset=None, gold="URGENT",
          hazards=[h(None, "FLOODED", None)], water="WAIST"),
        r(32, "T9", "en", "We moved to the Kottamam LP school camp and now the camp is cut off. 80 "
          "people, nobody is sick.",
          location="Kottamam LP school camp", headcount=80, asset=None, gold="STABLE",
          **SELF),
        r(33, "T9", "ml", "ഞങ്ങൾ നാലുപേർ കടയുടെ മുകളിലത്തെ നിലയിൽ ആണ്, താഴെ അരയോളം "
          "വെള്ളം. ഒക്കൽ ചന്തക്ക് അടുത്ത്.",
          location="ഒക്കൽ ചന്ത", headcount=4, asset=None, gold="URGENT",
          water="WAIST", **SELF),
        # --- T10: who is reporting ---
        r(34, "T10", "en", "Me and my two kids are stuck at home near Kodikuthimala temple, the water "
          "is knee deep.",
          location="Kodikuthimala temple", headcount=3, asset=None, gold="URGENT",
          groups=["CHILD"], water="KNEE", **SELF),  # with family: SELF
        r(35, "T10", "en", "My sister and her 2 kids are in their house at Kidangoor lane. I'm "
          "standing outside, I can't wade in, the water is chest deep.",
          location="Kidangoor lane", headcount=3, asset=None, gold="CRITICAL",
          groups=["CHILD"], water="CHEST", relation="HOUSEHOLD", on_scene=True),
        r(36, "T10", "manglish", "Ente veettil vellam keri, ammayum randu kuttikalum avide und. "
          "Kuruvilassery road.",
          location="Kuruvilassery road", headcount=3, asset=None, gold="UNKNOWN",
          groups=["CHILD"], relation="UNDER_REVIEW"),  # own home, reporter there?
        r(37, "T10", "ml", "ഞങ്ങളുടെ വീട്ടിൽ അച്ഛനും അമ്മയും ഉണ്ട്, അരയോളം വെള്ളം. "
          "മഞ്ഞപ്പെട്ടി കവല.",
          location="മഞ്ഞപ്പെട്ടി കവല", headcount=2, asset=None, gold="URGENT",
          water="WAIST", relation="UNDER_REVIEW"),
        r(38, "T10", "en", "This is the Vengola village office. 14 families at Plamudi colony need "
          "evacuation; the water is waist deep there.",
          location="Plamudi colony", headcount=None, asset=None, gold="URGENT",
          water="WAIST", relation="THIRD_PARTY"),  # official, position not stated
        r(39, "T10", "en", "I left the house this morning; my wife and mother are still there at "
          "Pulimoodu junction. The water is rising.",
          location="Pulimoodu junction", headcount=2, asset=None, gold="UNKNOWN",
          trend="RISING", **REMOTE_FAMILY),  # "mother": no ELDERLY
        r(40, "T10", "en", "I can't reach my brother by phone. He lives alone near Kannamthoppu "
          "church and his last message said water was inside.",
          location="Kannamthoppu church", headcount=1, asset=None, gold="UNKNOWN",
          **REMOTE_FAMILY),
        r(41, "T10", "manglish", "Ayalpakkathe veettile chettanum chechiyum kudungi, njan ente "
          "veettinte mukalil ninnu kaanunnu. Vellam nenjolam. Kottapuram lane.",
          location="Kottapuram lane", headcount=2, asset=None, gold="URGENT",
          water="CHEST", **BYSTANDER),
        r(42, "T10", "ml", "മൂന്നുപേർ അത്താണി ബോട്ടുജെട്ടിക്ക് അടുത്ത് കുടുങ്ങി. ബോട്ട് "
          "വേണം.",
          location="അത്താണി ബോട്ടുജെട്ടി", headcount=3, asset="RESCUE_BOAT", gold="UNKNOWN"),
        r(43, "T10", "en", "We have 22 people at the Moozhikkulam shelter who need an evacuation "
          "truck.",
          location="Moozhikkulam shelter", headcount=22, asset="EVAC_TRUCK",
          gold="UNKNOWN"),  # coordinator phrasing (change 5)
        r(44, "T10", "en", "We are 22 at the Thuruthy shelter and we need an evacuation truck.",
          location="Thuruthy shelter", headcount=22, asset="EVAC_TRUCK", gold="UNKNOWN",
          **SELF),  # "we are 22": SELF
        r(45, "T10", "manglish", "Ente kootukaran vilichu, avante veettil 5 per kudungi, vellam "
          "aravolam. Mudakkuzha il aanu. Njan avide illa.",
          location="Mudakkuzha", headcount=5, asset=None, gold="URGENT",
          water="WAIST", relation="THIRD_PARTY", on_scene=False),
        r(46, "T10", "ml", "ഞാനും ഭാര്യയും അമ്മയും ആണ്, മുട്ടോളം വെള്ളം. അമ്മയ്ക്ക് "
          "എഴുപത്തഞ്ച് വയസ്സ്. നെടുമ്പാറ റോഡ്.",
          location="നെടുമ്പാറ റോഡ്", headcount=3, asset=None, gold="URGENT",
          groups=["ELDERLY"], water="KNEE", **SELF),
        r(47, "T10", "en", "There's an old woman alone in the house opposite mine at Kallil lane. I "
          "can see her at the window and the water is chest deep.",
          location="Kallil lane", headcount=1, asset=None, gold="CRITICAL",
          groups=["ELDERLY"], water="CHEST", **BYSTANDER),
        r(48, "T10", "manglish", "Ammachi ottakku aanu veettil, vellam kayari varunnu. Veedu "
          "Panangad colony il aanu.",
          location="Panangad colony", headcount=1, asset=None, gold="URGENT",
          groups=["ELDERLY"], trend="RISING", relation="HOUSEHOLD"),
        r(49, "T10", "en", "My neighbour's family of 5 is stuck next door at Anappara lane. The water "
          "is above their windows. I've gone to my sister's place in Kakkanad.",
          location="Anappara lane", headcount=5, asset=None, gold="CRITICAL",
          water="ABOVE_HEAD", relation="THIRD_PARTY", on_scene=False),
        r(50, "T10", "ml", "ഞാൻ ഇവിടെ ഒറ്റയ്ക്കാണ്, അരയോളം വെള്ളം, കാലിന് പരിക്കുണ്ട്. "
          "ചെറുകുന്നത്തുപടി.",
          location="ചെറുകുന്നത്തുപടി", headcount=1, asset=None, gold="URGENT",
          medical=["INJURY"], water="WAIST", **SELF),
    ]


BATCHES = {1: batch_1, 2: batch_2, 3: batch_3, 4: batch_4}

# Annotator notes shown on the owner review sheet (not part of the records).
NOTES = {
    "sft-v4-new-b2-006": "Owner review: only the outer locative is removed; the inner "
    "genitive stays (guideline general rule 2).",
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
    "sft-v4-new-b2-027": "Fresh head wound from a fall just now: gold CRITICAL (guideline "
    "change 9).",
    "sft-v4-new-b2-029": "Postpartum bleeding. The newborn is INFANT; the mother is no longer "
    "PREGNANT.",
    "sft-v4-new-b2-030": "Reporter's position not stated: on_scene null. Owner review: "
    "BLEEDING label, gold URGENT (a nosebleed that will not stop is rarely life-threatening "
    "within an hour).",
    "sft-v4-new-b2-031": "Owner review: gold URGENT; bleeding not described as heavy or not "
    "stopping, and five people are there to apply pressure.",
    "sft-v4-new-b2-032": "Minor cut that has stopped bleeding, knee-deep water, nothing else "
    "wrong: INJURY only. Owner review: gold STABLE.",
    "sft-v4-new-b2-034": "Missed dialysis, weak but conscious: CHRONIC_CARE, not UNCONSCIOUS.",
    "sft-v4-new-b2-035": "Suspected snakebite counts (guideline change 3).",
    "sft-v4-new-b2-036": "Owner review: labour is gold CRITICAL, aligned with the frozen "
    "v3.1.0 labels (guideline change 7).",
    "sft-v4-new-b2-037": "Shock just now, talking: kept as a current condition (not resolved; "
    "delayed heart-rhythm risk, live wire still in the water).",
    "sft-v4-new-b2-039": "Fast-rising knee-deep water: gold URGENT; the rules cannot prove "
    "STABLE while rising, so UNKNOWN.",
    "sft-v4-new-b2-042": "Owner review: 'if the water keeps rising' presupposes it is rising "
    "now, so RISING, gold URGENT as 039 (guideline change 8). The conditional covers only the "
    "roof.",
    "sft-v4-new-b2-043": "Several levels: use the latest (waist, draining).",
    "sft-v4-new-b2-044": "'My brother says': the reporter's position is not stated, so "
    "on_scene null.",
    "sft-v4-new-b2-045": "Owner review: CRITICAL. 3 ft is above a normal mattress and he "
    "cannot be moved; falling water does not remove the drowning risk yet.",
    "sft-v4-new-b2-048": "Water coming up through the drains is ongoing entry: RISING "
    "(guideline change 4).",
    "sft-v4-new-b2-054": "'If the water rises' is hypothetical: trend null.",
    "sft-v4-new-b2-055": "'Difficulty walking' adds no group (guideline change 6); "
    "grandmother gives ELDERLY.",
    "sft-v4-new-b2-059": "'If the water rises' is hypothetical: trend null.",
    "sft-v4-new-b3-001": "Stroke in the past, no treatment stated: BEDRIDDEN only, no "
    "CHRONIC_CARE.",
    "sft-v4-new-b3-003": "Bedridden with water at bed height: gold CRITICAL (batch 2 owner "
    "policy, 045).",
    "sft-v4-new-b3-004": "Wheelchair user in ankle-deep water: gold STABLE (safe for now); the "
    "rules give UNKNOWN. Owner review: confirmed.",
    "sft-v4-new-b3-006": "Owner review: autism is DISABLED (guideline change 10). 'Makan' "
    "(son) adds no CHILD.",
    "sft-v4-new-b3-007": "Warden: THIRD_PARTY. Says who, not where, so on_scene null (AI "
    "review). Headcount counts residents only.",
    "sft-v4-new-b3-027": "Owner review: gold STABLE. Ankle-deep water with only pregnancy "
    "stated is safe for now, like 004, 029 and an infant at the same depth; 008 (knee) stays "
    "URGENT.",
    "sft-v4-new-b3-009": "Water at the doorstep, not inside: no depth where they are, so null. "
    "Gold STABLE (safe for now). Owner review: confirmed.",
    "sft-v4-new-b3-011": "Oxygen with about 3 hours left: CHRONIC_CARE (no acute episode), gold "
    "URGENT.",
    "sft-v4-new-b3-017": "Ward member who cannot reach them: THIRD_PARTY, on_scene false.",
    "sft-v4-new-b3-018": "'Son' adds no CHILD; he is at his office.",
    "sft-v4-new-b3-020": "Pair with 021: 'my father' adds no ELDERLY; '78-year-old father' does.",
    "sft-v4-new-b3-022": "College students and 'makal' (daughter): no CHILD. Reporter's "
    "position not stated: on_scene null.",
    "sft-v4-new-b3-025": "'Old house' describes the building: no ELDERLY.",
    "sft-v4-new-b3-026": "Location is the named building; 'at Kizhakkumuri' is dropped, no "
    "comma added (owner review). Old age home residents: ELDERLY. 'Water is entering': RISING (change "
    "4). Relation not stated.",
    "sft-v4-new-b3-030": "'Little brother' states no age: no CHILD.",
    "sft-v4-new-b3-031": "'Makkal' (offspring) states no age: no CHILD.",
    "sft-v4-new-b3-032": "'Valiyachan' (uncle) states no age: no ELDERLY. Reporter's position "
    "not stated.",
    "sft-v4-new-b3-035": "Fresh head wound: gold CRITICAL (change 9).",
    "sft-v4-new-b3-038": "Burn is INJURY, kept beside BREATHING_DIFFICULTY.",
    "sft-v4-new-b3-041": "Head wound has stopped bleeding: INJURY, not BLEEDING; chest pain "
    "now: CARDIAC.",
    "sft-v4-new-b3-042": "'Fell, but not hurt': negation, no INJURY.",
    "sft-v4-new-b3-049": "Minor sprain: INJURY only, gold STABLE (as batch 2, 032).",
    "sft-v4-new-b3-050": "Fresh head wound with blood flowing: gold CRITICAL (change 9).",
    "sft-v4-new-b4-002": "Roof with no water stated: water null, gold URGENT as frozen 105 "
    "and t273.",
    "sft-v4-new-b4-003": "In a tree for two hours with water below: gold CRITICAL (more "
    "precarious than a roof). Owner check.",
    "sft-v4-new-b4-005": "Climbed to the terrace because the house is flooded: ROOF, not the "
    "inside depth (guideline ROOF row).",
    "sft-v4-new-b4-008": "Car roof is a height (change 1). 'Above the doors' gives no band: "
    "water null.",
    "sft-v4-new-b4-010": "Stairs to the terrace under water: house flooded, so ROOF.",
    "sft-v4-new-b4-015": "Terrace only if the water rises: hypothetical, not stranded.",
    "sft-v4-new-b4-016": "On the terrace only for phone signal: not stranded.",
    "sft-v4-new-b4-017": "Upper floor, cannot reach the roof: not stranded at height.",
    "sft-v4-new-b4-019": "Water around the school but not inside: water null, gold STABLE (as "
    "batch 3, 009).",
    "sft-v4-new-b4-020": "Food running out is not a medical condition; gold STABLE.",
    "sft-v4-new-b4-021": "'About 60' is not an exact count: headcount null.",
    "sft-v4-new-b4-029": "Upper floor of an office, chest-deep water below: no "
    "other_conditions; gold URGENT for adults.",
    "sft-v4-new-b4-035": "Sister standing outside: HOUSEHOLD, on scene.",
    "sft-v4-new-b4-036": "'Ente veettil' (my house) with mother and children there: whether "
    "the reporter is among them is open, so UNDER_REVIEW.",
    "sft-v4-new-b4-037": "'Our house' with the parents there: UNDER_REVIEW.",
    "sft-v4-new-b4-038": "Village office: THIRD_PARTY; '14 families' gives no headcount.",
    "sft-v4-new-b4-039": "'I left this morning': on_scene false. 'Mother' adds no ELDERLY.",
    "sft-v4-new-b4-040": "'Can't reach him by phone': on_scene false. 'Water was inside' has no "
    "depth.",
    "sft-v4-new-b4-043": "Coordinator phrasing (change 5): relation null. Paired with 044.",
    "sft-v4-new-b4-045": "Friend: THIRD_PARTY; 'njan avide illa' (I'm not there): on_scene "
    "false.",
    "sft-v4-new-b4-048": "'Ammachi ottakku' (alone at home) implies the reporter is elsewhere "
    "but does not say where: HOUSEHOLD, on_scene null. Owner check.",
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
            f"**Batch {args.review_sheet} ({len(batch)} records): "
            + ("AI first pass and owner review complete.** "
               if args.review_sheet in OWNER_REVIEWED_BATCHES
               else "awaiting AI first pass and owner review.** ")
            + "Guideline changes 5–8 are in "
            "`triage-annotation-guidelines-v3.2-changes.md`.",
            "src/crisisgraph_ml/triage_sft_v4_new.py"), encoding="utf-8")
        print(f"Wrote review sheet to {sheet}")


if __name__ == "__main__":
    main()
