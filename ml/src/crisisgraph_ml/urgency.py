"""Deterministic urgency rule table V1 for triage extraction V3.

The model extracts categories into ``victim_profile``; this module derives the
urgency tier from them. The Rust triage worker implements the same table (M3b),
and both are checked against ``data/urgency-rules-v1.jsonl`` so they cannot
drift apart.

Rules are evaluated top-down; the first tier with any matching rule wins:

- CRITICAL: UNCONSCIOUS, BREATHING_DIFFICULTY, CARDIAC or BLEEDING; water
  ABOVE_HEAD or ROOF; water >= CHEST and RISING; INFANT or BEDRIDDEN with
  water >= WAIST; any low-mobility group (INFANT, BEDRIDDEN, DISABLED,
  ELDERLY) with water >= WAIST and RISING.
- URGENT: INJURY or CHRONIC_CARE; any other_conditions; PREGNANT; any
  vulnerable group with water >= KNEE; water WAIST or CHEST.
- STABLE: no conditions, no vulnerable groups, water stated as ANKLE or KNEE,
  and not RISING.
- UNKNOWN: everything else, i.e. not enough facts to prove STABLE. Sorted
  with URGENT for priority, never below it.

``suggested_urgency`` from the model can only raise the result.

urgency-rules-v2 (owner decision, 2026-10-26) moves ``other_conditions`` from
URGENT to CRITICAL, flagged ``OTHER_CONDITION_IMMEDIATE_REVIEW``: anything life
threatening outside the categories (snakebite, drowning risk, hypothermia) is
prioritised at once and a dispatcher reviews it immediately. Under v1 these
cases could only reach URGENT, and the evaluation showed the model's
raise-only suggestion did not cover the gap. Accepts over-triage (e.g. labour,
high fever) in exchange. v1 is kept unchanged to reproduce earlier results.

urgency-rules-v3 (owner decision, 2026-10-26) adds: a low-mobility group
(INFANT, BEDRIDDEN, DISABLED, ELDERLY) or PREGNANT with water at CHEST or above
is CRITICAL even when the water is not rising (blind test set review: an
elderly mother and a pregnant woman in chest-deep water). v1 and v2 are kept.

urgency-rules-v4 (owner decision, 2026-10-27) adds CHILD with water at CHEST
or above: about 4 ft is at a child's chin or neck (SFT-V4 training review).
INFANT was already covered by v3's low-mobility rule. v1–v3 are kept.

When the result is still UNKNOWN (rule and final urgency both UNKNOWN), the
case gets ``UNKNOWN_REVIEW_REQUIRED`` and a ``review_priority`` that orders the
dispatcher review/callback queue. UNKNOWN never has medical or other
conditions (those already make it URGENT or CRITICAL), so the priority comes
from the remaining facts:

- HIGH: a group that cannot easily move itself (INFANT, BEDRIDDEN, DISABLED,
  ELDERLY), or water RISING.
- MEDIUM: any other vulnerable group (in practice CHILD).
- LOW: nothing else stated. (A stated water level alone cannot reach here:
  UNKNOWN with a level and no group means the water is RISING, i.e. HIGH.)
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

RULES_V1 = "urgency-rules-v1"
RULES_V2 = "urgency-rules-v2"
RULES_V3 = "urgency-rules-v3"
RULES_V4 = "urgency-rules-v4"
RULES_VERSION = RULES_V4  # current rules
RULE_VERSIONS = (RULES_V1, RULES_V2, RULES_V3, RULES_V4)

WATER_ORDER = ("ANKLE", "KNEE", "WAIST", "CHEST", "ABOVE_HEAD", "ROOF")
CRITICAL_CONDITIONS = frozenset({"UNCONSCIOUS", "BREATHING_DIFFICULTY", "CARDIAC", "BLEEDING"})
URGENT_CONDITIONS = frozenset({"INJURY", "CHRONIC_CARE"})

# Priority rank: UNKNOWN shares URGENT's rank (sorted with it, never below it).
RANK = {"STABLE": 0, "UNKNOWN": 1, "URGENT": 1, "CRITICAL": 2}

FLAG_OTHER_CONDITION = "OTHER_CONDITION_REVIEW"  # v1
FLAG_OTHER_CONDITION_IMMEDIATE = "OTHER_CONDITION_IMMEDIATE_REVIEW"  # v2
FLAG_SUGGESTION_RAISED = "SUGGESTED_URGENCY_RAISED"
FLAG_SUGGESTION_LOWER = "SUGGESTED_URGENCY_LOWER_IGNORED"
FLAG_UNKNOWN_REVIEW = "UNKNOWN_REVIEW_REQUIRED"

LOW_MOBILITY_GROUPS = frozenset({"INFANT", "BEDRIDDEN", "DISABLED", "ELDERLY"})


@dataclass(frozen=True)
class UrgencyResult:
    urgency: str
    rule_urgency: str
    review_priority: str | None = None
    flags: list[str] = field(default_factory=list)

    def as_dict(self) -> dict[str, Any]:
        return {
            "urgency": self.urgency,
            "rule_urgency": self.rule_urgency,
            "review_priority": self.review_priority,
            "flags": self.flags,
        }


def _water_at_least(level: str | None, minimum: str) -> bool:
    return level is not None and WATER_ORDER.index(level) >= WATER_ORDER.index(minimum)


def _check_version(rules: str) -> None:
    if rules not in RULE_VERSIONS:
        raise ValueError(f"unknown urgency rules version: {rules}")


def rule_urgency(profile: dict[str, Any], rules: str = RULES_VERSION) -> str:
    _check_version(rules)
    medical = set(profile.get("medical_conditions") or [])
    other = profile.get("other_conditions") or []
    groups = set(profile.get("vulnerable_groups") or [])
    water = profile.get("water_level")
    rising = profile.get("water_trend") == "RISING"

    if (
        medical & CRITICAL_CONDITIONS
        or _water_at_least(water, "ABOVE_HEAD")
        or (_water_at_least(water, "CHEST") and rising)
        or (groups & {"INFANT", "BEDRIDDEN"} and _water_at_least(water, "WAIST"))
        or (groups & LOW_MOBILITY_GROUPS and _water_at_least(water, "WAIST") and rising)
        or (rules != RULES_V1 and other)
        or (
            rules in (RULES_V3, RULES_V4)
            and groups & (LOW_MOBILITY_GROUPS | {"PREGNANT"})
            and _water_at_least(water, "CHEST")
        )
        or (rules == RULES_V4 and "CHILD" in groups and _water_at_least(water, "CHEST"))
    ):
        return "CRITICAL"
    if (
        medical & URGENT_CONDITIONS
        or other
        or "PREGNANT" in groups
        or (groups and _water_at_least(water, "KNEE"))
        or _water_at_least(water, "WAIST")
    ):
        return "URGENT"
    if not medical and not groups and water in ("ANKLE", "KNEE") and not rising:
        return "STABLE"
    return "UNKNOWN"


def derive_urgency(profile: dict[str, Any], rules: str = RULES_VERSION) -> UrgencyResult:
    """Rule urgency raised (never lowered) by the model's ``suggested_urgency``."""
    base = rule_urgency(profile, rules)
    flags: list[str] = []
    if profile.get("other_conditions"):
        flags.append(FLAG_OTHER_CONDITION if rules == RULES_V1 else FLAG_OTHER_CONDITION_IMMEDIATE)

    final = base
    suggested = profile.get("suggested_urgency")
    if suggested is not None:
        if RANK[suggested] > RANK[base] or (base == "UNKNOWN" and suggested == "URGENT"):
            # UNKNOWN -> URGENT keeps the same priority but replaces "unknown".
            final = suggested
            if RANK[suggested] > RANK[base]:
                flags.append(FLAG_SUGGESTION_RAISED)
        elif RANK[suggested] < RANK[base]:
            flags.append(FLAG_SUGGESTION_LOWER)

    review_priority = None
    if final == "UNKNOWN":
        review_priority = unknown_review_priority(profile)
        flags.append(FLAG_UNKNOWN_REVIEW)
    return UrgencyResult(
        urgency=final, rule_urgency=base, review_priority=review_priority, flags=flags
    )


def unknown_review_priority(profile: dict[str, Any]) -> str:
    """Order of an UNKNOWN case in the dispatcher review queue."""
    groups = set(profile.get("vulnerable_groups") or [])
    if groups & LOW_MOBILITY_GROUPS or profile.get("water_trend") == "RISING":
        return "HIGH"
    if groups:
        return "MEDIUM"
    return "LOW"
