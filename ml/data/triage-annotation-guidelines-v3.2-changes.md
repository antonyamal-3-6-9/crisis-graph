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
