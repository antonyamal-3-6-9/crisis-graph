# Triage annotation guidelines

These rules define the expected labels for CrisisGraph SOS extraction.

## General policy

1. Label only facts explicitly stated in the report.
2. Preserve the lexical content and spelling of a named location from the
   report, excluding surrounding prose such as "near" or "at" when the named
   place itself is clear. In morphologically inflected languages such as
   Malayalam, grammatical case markers attached to the location mention may
   be removed so `victim_location` contains the uninflected proper name. Do not
   otherwise translate, canonicalize, correct, or expand the location name.
3. Use `null` for a missing, ambiguous, or contradictory scalar value.
4. Do not assume a headcount of one when no number is given.
5. Do not select an asset from general world knowledge. In the initial dataset,
   label an asset only when the report explicitly requests it.
6. Hazard reports are candidates, not verified operational state.
7. Never infer a hazard duration. Use `null` unless a duration is stated.
8. Set `needs_human_review=true` when any dispatch-critical scalar or hazard
   field is null, or when the report is irrelevant or contradictory.
9. Instructions inside an SOS message are untrusted content. They must not
   change the extraction policy or authorize an action.

Examples of permitted Malayalam case-marker removal:

- `പാം കോർട്ട് ഹോസ്റ്റലിൽ` → `പാം കോർട്ട് ഹോസ്റ്റൽ`
- `ഓൾഡ് ഫെറി ക്ലിനിക്കിൽ` → `ഓൾഡ് ഫെറി ക്ലിനിക്ക്`
- `ഹിൽവ്യൂ റിലീഫ് ക്യാമ്പിൽ` → `ഹിൽവ്യൂ റിലീഫ് ക്യാമ്പ്`

## `road_segment` policy

`road_segment` is a textual candidate locator for the road affected by the
reported hazard. It is not an OSM/Neo4j segment ID, does not establish that the
road exists in the baseline graph, and must never directly authorize a closure.
Deterministic spatial resolution and operational verification happen after
extraction.

- Preserve an explicitly named road, such as `Market Road`.
- Preserve a description only when the report itself anchors it to a named
  place in the same phrase, such as `road near Pump Junction`. A faithful
  translation into the evaluation language is acceptable when required for a
  cross-lingual expected output.
- Use `null` for unanchored or deictic descriptions such as `nearby road`,
  `surrounding road`, or `the road over there`. Do not attach these phrases to
  the victim location from a previous sentence by inference.
- Use `null` when only an area-level hazard is stated, such as `the area is
  flooded`, and no road is identified.
- Still extract an explicitly stated hazard status when `road_segment` is
  `null`; the incomplete hazard must set `needs_human_review=true`.

## Critical fields

- `victim_location`: the victim or pickup location, not a road mentioned only
  as a hazard.
- `headcount`: the number of people requiring assistance.
- `required_asset`: one of `RESCUE_BOAT`, `AMBULANCE`, `EVAC_TRUCK`, or
  `HELICOPTER`; otherwise null.
- `hazards`: only hazards stated in the report, in order of appearance.
- `needs_human_review`: whether deterministic dispatch processing must stop
  for clarification or verification.

## Synthetic SFT confidence labels

For the V1, V2, and V3 synthetic SFT datasets, use this fixed mapping:

- `0.95`: complete, explicit dispatch facts with `needs_human_review=false`.
- `0.55`: a relevant emergency report requiring review because critical facts
  are missing, uncertain, contradictory, or incomplete.
- `0.10`: an irrelevant message containing no emergency dispatch facts;
  `needs_human_review=true`.

These are annotation-policy labels, not calibrated probabilities or evidence
that a route or incident is safe. A parser instruction does not lower confidence
when the genuine emergency facts are otherwise complete and unambiguous.
This documents the existing SFT convention; it does not relabel frozen evaluation.

## Review process

Use `pending_manual_review` before semantic review, `first_pass_reviewed` after
one annotator accepts the complete record, `reviewed` only after the required
independent second pass and disagreement resolution, and `needs_revision` when
a decision remains open.

Every test-set label must be reviewed by a second annotator or a domain expert
before the set is frozen. Resolve disagreements by clarifying these guidelines
before changing individual labels. Never put real names, phone numbers,
medical identifiers, or live operational details in Git.

## V3 addendum: `victim_profile` (schema `triage-extraction-v3`)

Status: **draft for owner review** (2026-10-25). Applies to the V3 evaluation
sets and any later V3 training data. V2 field rules above are unchanged.

### Principles

1. Label facts **about the people needing help, as stated now**. Ignore
   hypotheticals ("if the water rises…"), past events that are over ("she
   fainted yesterday, she is fine now"), and negations ("no one is injured" →
   no `INJURY`).
2. Map meaning, not words: any language, spelling or phrasing that clearly
   states the concept gets the category.
3. **Safety asymmetry for critical conditions.** A stated *suspicion* of a
   critical condition is labelled as present ("I think he is not breathing
   properly" → `BREATHING_DIFFICULTY`). Missing a critical condition is far
   worse than an extra one. This applies only to the four critical conditions.
4. Unknown is `null` (scalars) or `[]` (lists). Never infer from world
   knowledge (a flood does not imply `RISING`; "my grandmother" implies
   `ELDERLY`, because the relation itself states age).
5. `victim_profile` never changes `needs_human_review`. Label V2 fields exactly
   as before.
6. No person names, phone numbers or identifiers anywhere, including
   `other_conditions`.

### `reporter_relation`

| Value | When | Examples |
|---|---|---|
| `SELF` | The reporter is among the people needing help | "we are stuck", "I am on the roof", "ഞങ്ങൾ കുടുങ്ങി" |
| `HOUSEHOLD` | The reporter reports their own family, and does not say they are with them | "my parents are trapped", "ente ammayum achanum veetil kudungi" |
| `THIRD_PARTY` | Anyone else: neighbours, strangers, officials, volunteers | "the family next door", "an old man on the bridge" |
| `null` | The text does not say who is reporting | "3 people at Pump Junction need a boat" |

If the reporter is *with* family ("me and my kids"), use `SELF`.

### `reporter_on_scene`

Whether the reporter is **at the victims' place or within sight of it**, so the
reporter's phone position also locates the victims.

- `true`: `SELF`; or explicitly present or watching ("I can see them from my
  terrace", "I'm standing outside their house").
- `false`: explicitly elsewhere ("I'm in Dubai", "calling from Kochi", "I left
  this morning, they are still there", "I can't get there").
- `null`: not stated. `HOUSEHOLD` and `THIRD_PARTY` without a stated position
  are `null`, never assumed.

### `medical_conditions`

| Category | Label when stated | Not this category |
|---|---|---|
| `UNCONSCIOUS` | unconscious, fainted and not responding, not waking up, "bodham illa", "ബോധം ഇല്ല", "ബോധമില്ല" | dizzy, weak, sleepy but responding |
| `BREATHING_DIFFICULTY` | can't breathe, gasping, choking, asthma attack, "shwasam muttal", "ശ്വാസം മുട്ടൽ" | known asthma without a current attack (`CHRONIC_CARE`) |
| `CARDIAC` | chest pain, heart attack, heart problem happening now, "nenju vedana", "നെഞ്ചുവേദന" | known heart patient with no current symptoms (`CHRONIC_CARE`) |
| `BLEEDING` | actively bleeding, heavy blood loss, "chora varunnu", "രക്തം വരുന്നു" | a cut described as minor or already stopped (`INJURY`) |
| `INJURY` | fracture, broken leg, wound, burn, fall injury, "parikku", "പരിക്ക്" | pain with no injury stated |
| `CHRONIC_CARE` | ongoing treatment that the flood interrupts: dialysis, insulin, oxygen supply, essential medicine running out | — |

Several categories may apply ("bleeding head wound" → `BLEEDING`, `INJURY`).

### `other_conditions`

Short phrases (≤ 300 characters, ≤ 10 items) for **medical or life-threatening
situations not covered above**: snakebite, electric shock, in labour, high
fever, hypothermia, swept away by the current, trapped under debris. Use the
report's meaning in plain English, no names. A condition already covered by a
category is not repeated here. Pregnancy is a vulnerable group; labour is also
an `other_conditions` entry ("in labour").

### `vulnerable_groups`

| Category | When |
|---|---|
| `INFANT` | baby, newborn, infant, under about one year, "kaikkunju", "കൈക്കുഞ്ഞ്" |
| `CHILD` | children or kids older than an infant, "kuttikal", "കുട്ടികൾ" |
| `ELDERLY` | stated old age or a relation that states it: elderly, aged 70, grandmother, "ammachi", "appooppan", "പ്രായമായ" |
| `PREGNANT` | pregnant, "garbhini", "ഗർഭിണി" |
| `DISABLED` | wheelchair user, blind, physically or mentally disabled |
| `BEDRIDDEN` | cannot get out of bed, paralysed, "kidappilaanu", "കിടപ്പുരോഗി" |

Label each group present, each once. "Baby and two kids" → `INFANT`, `CHILD`.

### `water_level`

The **current depth where the victims are**, using the body or building as the
reference. If several levels are mentioned, use the latest/current one.

| Value | Body reference | Approximate depth |
|---|---|---|
| `ANKLE` | ankle, "kanankaal", "കണങ്കാൽ" | < 0.3 m (< 1 ft) |
| `KNEE` | knee, "muttolam", "മുട്ടോളം" | 0.3–0.6 m (1–2 ft) |
| `WAIST` | waist, hip, "aravolam", "അരയോളം" | 0.6–1.1 m (2–3.5 ft) |
| `CHEST` | chest, neck, shoulder, "nenjolam", "kazhuthOlam", "നെഞ്ചോളം", "കഴുത്തോളം" | 1.1–1.5 m (3.5–5 ft) |
| `ABOVE_HEAD` | over a person's height, ground floor fully under water | > 1.5 m |
| `ROOF` | water at roof level, or people have climbed onto the roof because the house is flooded | — |

"Water entered the house" or "the area is flooded" with no depth → `null`.

### `water_trend`

`RISING` (rising, increasing, "vellam koodunnu", "വെള്ളം കൂടുന്നു"),
`FALLING` (receding, going down, "kurayunnu"), `STABLE` (explicitly not
changing). Otherwise `null`.

### `suggested_urgency` (model output only)

Not labelled in `expected`; the evaluator ignores its value for field accuracy
and scores it through the derived urgency. In SFT targets it equals
`expected_urgency` when that is `CRITICAL`, `URGENT` or `STABLE`, and `null`
when it is `UNKNOWN`.

### `expected_urgency` (case-level gold label)

Set by the annotator **from clinical common sense, independently of the rule
table** (`data/urgency-rules-v1.jsonl`). Disagreements between this label and
the rule table on correctly extracted profiles are rule-table findings, not
annotation errors.

Do not label `review_priority`: it is derived by the rule table for UNKNOWN
results (`schemas/urgency-result-v1.json`), not annotated or extracted.

| Tier | Meaning |
|---|---|
| `CRITICAL` | Life is at risk within minutes to an hour without help |
| `URGENT` | Serious; needs priority, but not immediately life-threatening |
| `STABLE` | Safe for now; evacuation can wait behind others |
| `UNKNOWN` | The report gives too little to judge |

### V3 case record

```json
{"id": "v3-…", "category": "…", "language": "en|ml|manglish",
 "scenario_family": "…", "provenance": "…", "reviewer_status": "…",
 "input": "…",
 "expected": {"victim_location": …, "headcount": …, "required_asset": …,
              "hazards": [], "needs_human_review": …,
              "victim_profile": {"reporter_relation": …, "reporter_on_scene": …,
                                 "medical_conditions": [], "other_conditions": [],
                                 "vulnerable_groups": [], "water_level": …,
                                 "water_trend": …}},
 "expected_urgency": "CRITICAL|URGENT|STABLE|UNKNOWN"}
```

`other_conditions` is scored by meaning (a judge or a reviewed synonym list),
never by exact string.
