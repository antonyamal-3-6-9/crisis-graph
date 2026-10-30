# SFT-V4 owner policies (decisions on top of the guidelines)

Gold-tier and labelling decisions made in the owner reviews of SFT-V4
batches 1–7, given to the zero-context authors of batches 6–7. Gold labels
are clinical judgement; the urgency rules are scored separately.

Gold urgency:
- Ground floor fully submerged: CRITICAL, even with people safe upstairs.
- About 2 ft (WAIST by band) with healthy adults, nothing else: STABLE.
- Ankle- or knee-deep water with nothing else wrong: STABLE. Ankle-deep with
  only one vulnerable group stated (wheelchair user, children, pregnancy):
  STABLE. Pregnancy at knee depth: URGENT. Elderly at knee depth: URGENT.
- Children (CHILD) in waist-deep water: CRITICAL.
- Elderly at waist depth, nothing else: URGENT.
- Adults in chest-deep water, no trend: URGENT (losing phone contact is not a
  clinical fact).
- Pregnancy at knee depth with water falling: STABLE (frozen 067).
- Bedridden at knee depth with water still coming in: CRITICAL (reaches bed
  height within the hour). Infant or bedridden person
  with water at WAIST or above: CRITICAL.
- Dry on an upper floor, no group or trend: ground floor submerged CRITICAL,
  chest-deep below URGENT, waist-deep below STABLE.
- Stranded at height: flooding stated CRITICAL, nothing else stated URGENT.
- Bleeding: CRITICAL when heavy, will not stop, postpartum, or a fresh head
  wound; otherwise URGENT (label BLEEDING either way).
- Labour: CRITICAL.
- Minor injury (small cut that stopped, sprain) with nothing else: STABLE.
- Known condition with no episode now (asthma, epilepsy, heart disease):
  STABLE unless treatment is running out soon or other facts raise it.
- Treatment interrupted (dialysis missed, insulin gone, oxygen running out
  in hours): URGENT.
- A critical symptom or life-threatening event resolved within 24 h: no
  condition label, gold URGENT. Resolved more than 24 h ago: nothing.
- "Nobody is sick" alone does not prove safety (UNKNOWN); "everyone is safe"
  does (STABLE when nothing else).

Labels:
- One person at an incident counts as 1 without a numeral ("the old man next
  door slipped", "chettan"); null when the person is at home and others may be
  there.
- When reporter_relation is UNDER_REVIEW, headcount is null unless the text
  states the total ("aake 5 per").
- "About N people" is N. "N people, and a baby too" is ambiguous: null.
- "We have N people who need …" (coordinator): relation null.
- A distal "avide" (there) about the victims places the reporter away:
  HOUSEHOLD, on_scene null.
- Instructions inside a message (to "the AI", "system", "admin") are ignored.
- Water depth at the victims' own location is not a V2 hazard; a flooded or
  blocked road or area is.
- A truck requested to move people is EVAC_TRUCK.
