//! Victim profile (triage extraction V3) and the deterministic urgency rules.
//!
//! The model extracts categories into [`VictimProfile`]; [`derive_urgency`]
//! turns them into an urgency tier. The rules are `urgency-rules-v5`, the same
//! table as `ml/src/crisisgraph_ml/urgency.py`; both are checked against
//! `ml/data/urgency-rules-v5.jsonl` so they cannot drift apart. Earlier rule
//! versions exist only in Python, to reproduce earlier evaluation results.
//!
//! Tiers, evaluated top-down (the first tier with a matching rule wins):
//!
//! - CRITICAL: UNCONSCIOUS, BREATHING_DIFFICULTY, CARDIAC or BLEEDING; any
//!   `other_conditions`; water ABOVE_HEAD or ROOF; water >= CHEST and RISING;
//!   INFANT or BEDRIDDEN with water >= WAIST; a low-mobility group with
//!   water >= WAIST and RISING; a low-mobility group or PREGNANT with water >= CHEST;
//!   CHILD with water >= WAIST; BEDRIDDEN with water >= KNEE and RISING.
//! - URGENT: INJURY or CHRONIC_CARE; PREGNANT; any vulnerable group with
//!   water >= KNEE; water WAIST or CHEST.
//! - STABLE: no conditions, no vulnerable groups, water stated as ANKLE or
//!   KNEE, and not RISING.
//! - UNKNOWN: everything else. Sorted with URGENT, never below it, and sent
//!   to the dispatcher callback list ordered by `review_priority`.
//!
//! The model's `suggested_urgency` can only raise the result.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

pub const URGENCY_RULES_VERSION: &str = "urgency-rules-v5";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReporterRelation {
    #[serde(rename = "SELF")]
    Reporter,
    Household,
    ThirdParty,
    /// The text points to the reporter's family or home but does not settle
    /// whether the reporter is among the victims (finding F-1).
    UnderReview,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MedicalCondition {
    Unconscious,
    BreathingDifficulty,
    Cardiac,
    Bleeding,
    Injury,
    ChronicCare,
}

impl MedicalCondition {
    pub fn is_critical(self) -> bool {
        matches!(self, Self::Unconscious | Self::BreathingDifficulty | Self::Cardiac | Self::Bleeding)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VulnerableGroup {
    Child,
    Infant,
    Elderly,
    Pregnant,
    Disabled,
    Bedridden,
}

impl VulnerableGroup {
    /// Groups that cannot easily move themselves.
    pub fn is_low_mobility(self) -> bool {
        matches!(self, Self::Infant | Self::Bedridden | Self::Disabled | Self::Elderly)
    }
}

/// Water depth, in increasing order (the derived `Ord` follows declaration order).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WaterLevel {
    Ankle,
    Knee,
    Waist,
    Chest,
    AboveHead,
    Roof,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WaterTrend {
    Rising,
    Stable,
    Falling,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Urgency {
    Critical,
    Urgent,
    Unknown,
    Stable,
}

impl Urgency {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::Urgent => "URGENT",
            Self::Unknown => "UNKNOWN",
            Self::Stable => "STABLE",
        }
    }

    /// Priority rank: UNKNOWN shares URGENT's rank (sorted with it, never below it).
    pub fn rank(self) -> u8 {
        match self {
            Self::Stable => 0,
            Self::Unknown | Self::Urgent => 1,
            Self::Critical => 2,
        }
    }
}

/// Order of an UNKNOWN case in the dispatcher callback list.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewPriority {
    High,
    Medium,
    Low,
}

impl ReviewPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "HIGH" => Some(Self::High),
            "MEDIUM" => Some(Self::Medium),
            "LOW" => Some(Self::Low),
            _ => None,
        }
    }

    /// Higher is reviewed first.
    pub fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UrgencyFlag {
    /// Something life-threatening outside the categories: CRITICAL, and a
    /// dispatcher reviews it at once.
    OtherConditionImmediateReview,
    SuggestedUrgencyRaised,
    SuggestedUrgencyLowerIgnored,
    UnknownReviewRequired,
}

impl UrgencyFlag {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OtherConditionImmediateReview => "OTHER_CONDITION_IMMEDIATE_REVIEW",
            Self::SuggestedUrgencyRaised => "SUGGESTED_URGENCY_RAISED",
            Self::SuggestedUrgencyLowerIgnored => "SUGGESTED_URGENCY_LOWER_IGNORED",
            Self::UnknownReviewRequired => "UNKNOWN_REVIEW_REQUIRED",
        }
    }
}

/// Facts about the people and conditions (schema `triage-extraction-v3`).
/// Unverified model output: it sets priority, never routing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VictimProfile {
    pub reporter_relation: Option<ReporterRelation>,
    pub reporter_on_scene: Option<bool>,
    pub medical_conditions: Vec<MedicalCondition>,
    pub other_conditions: Vec<String>,
    pub vulnerable_groups: Vec<VulnerableGroup>,
    pub water_level: Option<WaterLevel>,
    pub water_trend: Option<WaterTrend>,
    pub suggested_urgency: Option<Urgency>,
}

/// Every key of `victim_profile`; the schema requires all of them (an
/// explicit null, never an omitted field).
pub const VICTIM_PROFILE_FIELDS: [&str; 8] = [
    "reporter_relation",
    "reporter_on_scene",
    "medical_conditions",
    "other_conditions",
    "vulnerable_groups",
    "water_level",
    "water_trend",
    "suggested_urgency",
];

impl VictimProfile {
    /// Schema limits serde cannot express: unique items, sizes and the
    /// suggested-urgency values. Trims `other_conditions`.
    pub fn validate(&mut self) -> Result<(), String> {
        if self.medical_conditions.len() > 6
            || self.medical_conditions.iter().collect::<HashSet<_>>().len() != self.medical_conditions.len()
        {
            return Err("victim_profile.medical_conditions must be unique, at most 6".into());
        }
        if self.vulnerable_groups.len() > 6
            || self.vulnerable_groups.iter().collect::<HashSet<_>>().len() != self.vulnerable_groups.len()
        {
            return Err("victim_profile.vulnerable_groups must be unique, at most 6".into());
        }
        if self.other_conditions.len() > 10 {
            return Err("victim_profile.other_conditions exceeds 10 items".into());
        }
        for condition in &mut self.other_conditions {
            *condition = condition.trim().to_string();
            if condition.is_empty() || condition.chars().count() > 300 {
                return Err("victim_profile.other_conditions entry is empty or exceeds 300 characters".into());
            }
        }
        if self.suggested_urgency == Some(Urgency::Unknown) {
            return Err("victim_profile.suggested_urgency cannot be UNKNOWN".into());
        }
        Ok(())
    }

    fn has_group(&self, group: VulnerableGroup) -> bool {
        self.vulnerable_groups.contains(&group)
    }

    fn water_at_least(&self, minimum: WaterLevel) -> bool {
        self.water_level.is_some_and(|level| level >= minimum)
    }
}

/// Derived urgency (schema `urgency-result-v2`, plus the rules version).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UrgencyResult {
    pub urgency: Urgency,
    pub rule_urgency: Urgency,
    pub review_priority: Option<ReviewPriority>,
    pub flags: Vec<UrgencyFlag>,
    pub rules_version: String,
}

impl UrgencyResult {
    /// No extraction is available (model failed or circuit open): UNKNOWN,
    /// reviewed first.
    pub fn unknown_without_extraction() -> Self {
        Self {
            urgency: Urgency::Unknown,
            rule_urgency: Urgency::Unknown,
            review_priority: Some(ReviewPriority::High),
            flags: vec![UrgencyFlag::UnknownReviewRequired],
            rules_version: URGENCY_RULES_VERSION.to_string(),
        }
    }

    /// CRITICAL, or flagged for immediate dispatcher attention.
    pub fn needs_immediate_attention(&self) -> bool {
        self.urgency == Urgency::Critical || self.flags.contains(&UrgencyFlag::OtherConditionImmediateReview)
    }
}

/// The rule table alone, without the model's suggestion.
pub fn rule_urgency(profile: &VictimProfile) -> Urgency {
    use MedicalCondition::*;
    use VulnerableGroup::*;
    use WaterLevel::*;

    let medical = &profile.medical_conditions;
    let groups = &profile.vulnerable_groups;
    let rising = profile.water_trend == Some(WaterTrend::Rising);
    let low_mobility = groups.iter().any(|g| g.is_low_mobility());

    let critical = medical.iter().any(|m| m.is_critical())
        || !profile.other_conditions.is_empty()
        || profile.water_at_least(AboveHead)
        || (profile.water_at_least(Chest) && rising)
        || ((profile.has_group(Infant) || profile.has_group(Bedridden)) && profile.water_at_least(Waist))
        || (low_mobility && profile.water_at_least(Waist) && rising)
        || ((low_mobility || profile.has_group(Pregnant)) && profile.water_at_least(Chest))
        || (profile.has_group(Child) && profile.water_at_least(Waist))
        || (profile.has_group(Bedridden) && profile.water_at_least(Knee) && rising);
    if critical {
        return Urgency::Critical;
    }

    let urgent = medical.iter().any(|m| matches!(m, Injury | ChronicCare))
        || profile.has_group(Pregnant)
        || (!groups.is_empty() && profile.water_at_least(Knee))
        || profile.water_at_least(Waist);
    if urgent {
        return Urgency::Urgent;
    }

    if medical.is_empty() && groups.is_empty() && matches!(profile.water_level, Some(Ankle | Knee)) && !rising {
        return Urgency::Stable;
    }
    Urgency::Unknown
}

/// Rule urgency raised (never lowered) by the model's `suggested_urgency`.
pub fn derive_urgency(profile: &VictimProfile) -> UrgencyResult {
    let base = rule_urgency(profile);
    let mut flags = Vec::new();
    if !profile.other_conditions.is_empty() {
        flags.push(UrgencyFlag::OtherConditionImmediateReview);
    }

    let mut urgency = base;
    if let Some(suggested) = profile.suggested_urgency {
        if suggested.rank() > base.rank() || (base == Urgency::Unknown && suggested == Urgency::Urgent) {
            // UNKNOWN -> URGENT keeps the same priority but replaces "unknown".
            urgency = suggested;
            if suggested.rank() > base.rank() {
                flags.push(UrgencyFlag::SuggestedUrgencyRaised);
            }
        } else if suggested.rank() < base.rank() {
            flags.push(UrgencyFlag::SuggestedUrgencyLowerIgnored);
        }
    }

    let review_priority = (urgency == Urgency::Unknown).then(|| {
        flags.push(UrgencyFlag::UnknownReviewRequired);
        unknown_review_priority(profile)
    });
    UrgencyResult {
        urgency,
        rule_urgency: base,
        review_priority,
        flags,
        rules_version: URGENCY_RULES_VERSION.to_string(),
    }
}

/// UNKNOWN never has conditions (they already make it URGENT or CRITICAL),
/// so the order comes from the remaining facts.
pub fn unknown_review_priority(profile: &VictimProfile) -> ReviewPriority {
    if profile.vulnerable_groups.iter().any(|g| g.is_low_mobility())
        || profile.water_trend == Some(WaterTrend::Rising)
    {
        ReviewPriority::High
    } else if !profile.vulnerable_groups.is_empty() {
        ReviewPriority::Medium
    } else {
        ReviewPriority::Low
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const RULE_VECTORS: &str = include_str!("../../ml/data/urgency-rules-v5.jsonl");

    fn profile(json: Value) -> VictimProfile {
        let mut full = serde_json::json!({
            "reporter_relation": null, "reporter_on_scene": null, "medical_conditions": [],
            "other_conditions": [], "vulnerable_groups": [], "water_level": null,
            "water_trend": null, "suggested_urgency": null
        });
        full.as_object_mut().unwrap().extend(json.as_object().unwrap().clone());
        serde_json::from_value(full).unwrap()
    }

    #[test]
    fn rust_rules_match_every_shared_vector() {
        let mut checked = 0;
        for line in RULE_VECTORS.lines().filter(|l| !l.trim().is_empty()) {
            let vector: Value = serde_json::from_str(line).unwrap();
            let id = vector["id"].as_str().unwrap();
            let profile: VictimProfile = serde_json::from_value(vector["profile"].clone())
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            let mut actual = serde_json::to_value(derive_urgency(&profile)).unwrap();
            actual.as_object_mut().unwrap().remove("rules_version");
            assert_eq!(actual, vector["expected"], "{id}: {}", vector["why"]);
            checked += 1;
        }
        assert_eq!(checked, 68, "urgency-rules-v5.jsonl has 68 vectors");
    }

    #[test]
    fn each_tier_is_reachable() {
        let cases = [
            (serde_json::json!({"medical_conditions": ["CARDIAC"]}), Urgency::Critical),
            (serde_json::json!({"vulnerable_groups": ["CHILD"], "water_level": "WAIST"}), Urgency::Critical),
            (serde_json::json!({"medical_conditions": ["INJURY"]}), Urgency::Urgent),
            (serde_json::json!({"water_level": "KNEE"}), Urgency::Stable),
            (serde_json::json!({}), Urgency::Unknown),
        ];
        for (json, expected) in cases {
            assert_eq!(derive_urgency(&profile(json.clone())).urgency, expected, "{json}");
        }
    }

    #[test]
    fn unknown_sorts_with_urgent_never_below() {
        assert_eq!(Urgency::Unknown.rank(), Urgency::Urgent.rank());
        assert!(Urgency::Unknown.rank() > Urgency::Stable.rank());
        assert!(Urgency::Critical.rank() > Urgency::Unknown.rank());
    }

    #[test]
    fn suggestion_raises_but_never_lowers() {
        let raised = derive_urgency(&profile(serde_json::json!({
            "water_level": "KNEE", "suggested_urgency": "CRITICAL"})));
        assert_eq!((raised.urgency, raised.rule_urgency), (Urgency::Critical, Urgency::Stable));
        assert!(raised.flags.contains(&UrgencyFlag::SuggestedUrgencyRaised));

        let ignored = derive_urgency(&profile(serde_json::json!({
            "medical_conditions": ["UNCONSCIOUS"], "suggested_urgency": "STABLE"})));
        assert_eq!(ignored.urgency, Urgency::Critical);
        assert!(ignored.flags.contains(&UrgencyFlag::SuggestedUrgencyLowerIgnored));
    }

    #[test]
    fn unknown_gets_review_priority_in_order() {
        let high = derive_urgency(&profile(serde_json::json!({"vulnerable_groups": ["ELDERLY"]})));
        let medium = derive_urgency(&profile(serde_json::json!({"vulnerable_groups": ["CHILD"]})));
        let low = derive_urgency(&profile(serde_json::json!({})));
        assert_eq!(high.review_priority, Some(ReviewPriority::High));
        assert_eq!(medium.review_priority, Some(ReviewPriority::Medium));
        assert_eq!(low.review_priority, Some(ReviewPriority::Low));
        assert!(low.flags.contains(&UrgencyFlag::UnknownReviewRequired));
        assert_eq!(derive_urgency(&profile(serde_json::json!({"water_level": "ANKLE"}))).review_priority, None);
    }

    #[test]
    fn validation_rejects_what_the_schema_forbids() {
        let mut duplicate = profile(serde_json::json!({"medical_conditions": ["INJURY", "INJURY"]}));
        assert!(duplicate.validate().is_err());
        let mut unknown = profile(serde_json::json!({"suggested_urgency": "UNKNOWN"}));
        assert!(unknown.validate().is_err());
        let mut blank = profile(serde_json::json!({"other_conditions": ["  "]}));
        assert!(blank.validate().is_err());
        assert!(serde_json::from_value::<VictimProfile>(serde_json::json!({"water_level": "NECK"})).is_err());
    }

    #[test]
    fn failed_extraction_is_unknown_and_reviewed_first() {
        let result = UrgencyResult::unknown_without_extraction();
        assert_eq!(result.urgency, Urgency::Unknown);
        assert_eq!(result.review_priority, Some(ReviewPriority::High));
    }
}
