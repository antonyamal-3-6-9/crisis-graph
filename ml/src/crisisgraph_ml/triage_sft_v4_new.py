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


BATCHES = {1: batch_1}


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


if __name__ == "__main__":
    main()
