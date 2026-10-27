# Annotation guideline changes after triage-eval-v3.1.0

`triage-annotation-guidelines-v3.1-changes.md` is pinned by checksum in the
`triage-eval-v3.1.0` manifest, so rules decided after that freeze are recorded
here. They apply to every dataset created after v3.1.0 (SFT-V4 training data
onward) and agree with the v3.1.0 labels.

## 5. "We have N people who need …" (owner decision, 2026-10-26)

Coordinator or relay phrasing that counts people without placing the speaker
among them gets `reporter_relation: null`, `reporter_on_scene: null`: "We have
37 people who urgently need a rescue boat", "we have 95 people who need an
ambulance". Shelter staff, volunteers and control rooms talk this way, and the
caller's phone may be elsewhere. The speaker is placed in the group, so `SELF`,
by "we are 4 at …", "we are still here", "njangal ivide 15 perundu", or "we
have 4 children and 2 adults stuck here" (the speaker's own group). Identified
officials ("this is the village office") stay `THIRD_PARTY`.

## 6. "Cannot walk without help" vs "difficulty walking" (owner decision, 2026-10-26)

"Cannot walk without help" states a lasting functional inability:
`vulnerable_groups: ["DISABLED"]`, whatever the cause, because dispatch needs
the same help. "Difficulty walking" alone adds no group; label only the other
stated facts (e.g. ELDERLY for "elderly people with difficulty walking").

## 7. Labour is gold CRITICAL (owner decision, 2026-10-27)

A woman in labour (`other_conditions: ["in labour"]`, plus `PREGNANT`) is gold
`expected_urgency: CRITICAL`, as in the frozen v3.1.0 labels (cases with labour
and no other complication). Training data follows the frozen set, as for the
roof case (t273 and test 105), so the model is not taught a tier the benchmark
scores as wrong. The urgency rules already give CRITICAL for any
`other_conditions` entry.

## 8. "If the water keeps rising" states that it is rising (owner decision, 2026-10-27)

"If it keeps rising", "if it goes on rising" and "vellam iniyum koodiyal"
presuppose that the water is rising now: `water_trend: RISING`. The
conditional covers only what follows ("we will have to climb onto the roof"),
which is not labelled. A plain "if the water rises" ("vellam koodiyal",
"വെള്ളം കൂടിയാൽ") states no trend: `null`.

## 9. Gold tier for bleeding (owner decision, 2026-10-27)

Any stated active bleeding is labelled `BLEEDING` (safety asymmetry governs
the label, not the gold). Gold `expected_urgency` is `CRITICAL` when the
bleeding is heavy ("a lot", "orupaadu chora"), will not stop ("chora
nilkkunnilla"), is postpartum, or is a **fresh** head wound (a fall or blow
just now). Otherwise it is `URGENT`: a nosebleed that will not stop, a foot
wound with blood flowing and people there to apply pressure, or a head wound
that has bled for hours in someone who is talking (frozen v3.1.0 case 033).
The urgency rules still give `CRITICAL` for any `BLEEDING`.

## 10. Autism and developmental disability are DISABLED (owner decision, 2026-10-27)

"Autistic", "autism", "intellectual disability" and other developmental
disabilities state a disability: `vulnerable_groups: ["DISABLED"]` (they are
specified disabilities under India's Rights of Persons with Disabilities Act,
2016). A relation word beside them ("my son is autistic") still adds no age
group.
