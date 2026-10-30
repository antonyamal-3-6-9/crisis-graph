use serde::{Deserialize, Serialize};
use super::triage::{AssetType, RoadStatus, SosAlert, TriageReport};
use super::victim::WaterLevel;

pub const DEFAULT_BASELINE_HEADCOUNT: u32 = 4;

/// Operational resolution of triage ambiguities via the Uncertainty Waterfall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncertaintyWaterfall {
    /// True if caller location could not be resolved (strictly fails closed)
    pub location_missing: bool,
    /// Automated follow-up callback text queued for transmission to the caller
    pub automated_callback_inquiry: Option<String>,
    /// High-priority console alert indicating that immediate dispatcher phone contact is required
    pub dispatcher_dial_alert: Option<String>,
    /// True if headcount was inferred via standard unit baseline
    pub headcount_is_fallback: bool,
    pub original_headcount: Option<u32>,
    pub resolved_headcount: u32,
    /// The asset the caller asked for. Never guessed: a missing asset blocks
    /// automatic dispatch (finding F-2).
    pub resolved_asset: Option<AssetType>,
    /// When no asset was given: a suggestion from the extracted facts, shown
    /// to the dispatcher to confirm. Never dispatched on its own.
    pub suggested_asset: Option<AssetType>,
    /// Operational directives for field responders en route
    pub field_operational_directives: Vec<String>,
}

impl UncertaintyWaterfall {
    /// Resolve an ambiguous or incomplete triage report through the operational waterfall.
    pub fn resolve(alert: &SosAlert, triage: &TriageReport) -> Self {
        let location_missing = triage.resolved_junction_id.is_none();

        // 1. Location Rule: Never guess location without GPS or spatial coordinates
        let (automated_callback_inquiry, dispatcher_dial_alert) = if location_missing {
            (
                Some(
                    "CRISISGRAPH ALERT: Location not identified in your report. Reply immediately with your nearest landmark, street name, or GPS pin drop.".to_string(),
                ),
                Some(
                    format!(
                        "URGENT: Caller location unknown for alert '{}'. Immediate operator phone dial required.",
                        alert.alert_id
                    ),
                ),
            )
        } else if triage.headcount.is_none() {
            (
                Some(
                    "CRISISGRAPH DISPATCH: Response unit initialized. How many people require evacuation? Please reply with count.".to_string(),
                ),
                None,
            )
        } else {
            (None, None)
        };

        // 2. Headcount Waterfall: Fall back to standard scalable baseline unit (4 persons)
        let (headcount_is_fallback, resolved_headcount) = match triage.headcount {
            Some(count) if count > 0 => (false, count),
            _ => (true, DEFAULT_BASELINE_HEADCOUNT),
        };

        // 3. Asset: never guessed. Without one, suggest from the extracted
        //    facts for the dispatcher to confirm.
        let suggested_asset = match triage.required_asset {
            Some(_) => None,
            None => suggest_asset(triage),
        };

        // 4. Generate operational directives for field responders
        let mut field_operational_directives = Vec::new();
        if headcount_is_fallback {
            field_operational_directives.push(format!(
                "Headcount unconfirmed by caller. Dispatched standard baseline unit (capacity: {}). Driver: Verify exact count on arrival and request secondary transport if capacity exceeded.",
                DEFAULT_BASELINE_HEADCOUNT
            ));
        }
        if let Some(asset) = &suggested_asset {
            field_operational_directives.push(format!(
                "Required asset unspecified by caller; '{asset}' suggested from the extracted facts. Dispatcher: confirm the asset before dispatch."
            ));
        }

        Self {
            location_missing,
            automated_callback_inquiry,
            dispatcher_dial_alert,
            headcount_is_fallback,
            original_headcount: triage.headcount,
            resolved_headcount,
            resolved_asset: triage.required_asset.clone(),
            suggested_asset,
            field_operational_directives,
        }
    }
}

/// Asset suggestion from extracted categories, not raw-text keywords: water
/// at waist depth or deeper, or a flooded road, suggests a boat; a medical
/// condition suggests an ambulance; anything else gives no suggestion.
pub fn suggest_asset(triage: &TriageReport) -> Option<AssetType> {
    let profile = triage.victim_profile.as_ref();
    let deep_water = profile
        .and_then(|p| p.water_level)
        .is_some_and(|level| level >= WaterLevel::Waist);
    let flooded_road = triage.hazards.iter().any(|h| h.status == Some(RoadStatus::Flooded));
    if deep_water || flooded_road {
        return Some(AssetType::RescueBoat);
    }
    if profile.is_some_and(|p| !p.medical_conditions.is_empty()) {
        return Some(AssetType::Ambulance);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MedicalCondition, VictimProfile};
    use crate::models::triage::TriageInferenceMetadata;

    fn mock_metadata() -> TriageInferenceMetadata {
        TriageInferenceMetadata {
            backend: "test".into(),
            model: "test-model".into(),
            adapter_id: None,
            adapter_sha256: None,
            prompt_version: "v2".into(),
            schema_version: "triage-extraction-v2".into(),
            latency_ms: 10,
            tokens_per_second: None,
            prompt_eval_ms: None,
            completion_tokens: None,
        }
    }

    #[test]
    fn test_missing_location_never_guessed_triggers_callback_and_dial_alert() {
        let alert = SosAlert::new("SOS-NO-LOC-1", "Water is rising fast, we need urgent help!");
        let triage = TriageReport {
            victim_location_raw: None,
            resolved_junction_id: None,
            headcount: Some(2),
            required_asset: Some(AssetType::RescueBoat),
            hazards: vec![],
            confidence_score: 0.8,
            needs_human_review: true,
            uncertainty_reasons: vec!["Victim location was not provided".into()],
            inference: mock_metadata(),
            victim_profile: None,
            urgency: None,
            location_source: None,
        };

        let wf = UncertaintyWaterfall::resolve(&alert, &triage);
        assert!(wf.location_missing);
        assert!(wf.automated_callback_inquiry.unwrap().contains("Location not identified"));
        assert!(wf.dispatcher_dial_alert.unwrap().contains("Immediate operator phone dial required"));
        assert!(!wf.headcount_is_fallback);
        assert!(wf.suggested_asset.is_none());
    }

    #[test]
    fn test_unspecified_headcount_falls_back_to_baseline_unit() {
        let alert = SosAlert::new("SOS-NO-COUNT-2", "Family trapped at Bank Junction!");
        let triage = TriageReport {
            victim_location_raw: Some("Bank Junction".into()),
            resolved_junction_id: Some("node/4742583964".into()),
            headcount: None, // Missing
            required_asset: Some(AssetType::Ambulance),
            hazards: vec![],
            confidence_score: 0.9,
            needs_human_review: true,
            uncertainty_reasons: vec!["Headcount was not provided".into()],
            inference: mock_metadata(),
            victim_profile: None,
            urgency: None,
            location_source: None,
        };

        let wf = UncertaintyWaterfall::resolve(&alert, &triage);
        assert!(!wf.location_missing);
        assert!(wf.headcount_is_fallback);
        assert_eq!(wf.resolved_headcount, DEFAULT_BASELINE_HEADCOUNT);
        assert!(wf.automated_callback_inquiry.unwrap().contains("How many people require evacuation?"));
        assert!(wf.field_operational_directives.iter().any(|d| d.contains("standard baseline unit")));
    }

    fn triage_without_asset(profile: Option<VictimProfile>) -> TriageReport {
        TriageReport {
            victim_location_raw: Some("Aluva Manappuram".into()),
            resolved_junction_id: Some("node/7048449098".into()),
            headcount: Some(5),
            required_asset: None,
            hazards: vec![],
            confidence_score: 0.9,
            needs_human_review: true,
            uncertainty_reasons: vec!["Required asset was not specified".into()],
            inference: mock_metadata(),
            victim_profile: profile,
            urgency: None,
            location_source: None,
        }
    }

    #[test]
    fn unspecified_asset_is_suggested_from_categories_never_resolved() {
        let alert = SosAlert::new("SOS-FLOOD-3", "Periyar water entered our house");
        let deep = VictimProfile { water_level: Some(WaterLevel::Chest), ..Default::default() };
        let wf = UncertaintyWaterfall::resolve(&alert, &triage_without_asset(Some(deep)));
        assert_eq!(wf.resolved_asset, None);
        assert_eq!(wf.suggested_asset, Some(AssetType::RescueBoat));
        assert!(wf.field_operational_directives.iter().any(|d| d.contains("confirm the asset")));

        let medical = VictimProfile { medical_conditions: vec![MedicalCondition::Injury], ..Default::default() };
        let wf = UncertaintyWaterfall::resolve(&alert, &triage_without_asset(Some(medical)));
        assert_eq!(wf.suggested_asset, Some(AssetType::Ambulance));
    }

    #[test]
    fn raw_text_keywords_no_longer_pick_an_asset() {
        // The old keyword rule picked a boat from "water"/"munki" in the text.
        let alert = SosAlert::new("SOS-FLOOD-4", "water everywhere, munki, flood, boat please");
        let wf = UncertaintyWaterfall::resolve(&alert, &triage_without_asset(Some(VictimProfile::default())));
        assert_eq!(wf.suggested_asset, None);
        let wf = UncertaintyWaterfall::resolve(&alert, &triage_without_asset(None));
        assert_eq!(wf.suggested_asset, None);
    }

    #[test]
    fn test_complete_triage_has_no_fallbacks() {
        let alert = SosAlert::new("SOS-CLEAN-4", "3 injured passengers at Aluva Station need ambulance.");
        let triage = TriageReport {
            victim_location_raw: Some("Aluva Railway Station".into()),
            resolved_junction_id: Some("node/4664235699".into()),
            headcount: Some(3),
            required_asset: Some(AssetType::Ambulance),
            hazards: vec![],
            confidence_score: 0.95,
            needs_human_review: false,
            uncertainty_reasons: vec![],
            inference: mock_metadata(),
            victim_profile: None,
            urgency: None,
            location_source: None,
        };

        let wf = UncertaintyWaterfall::resolve(&alert, &triage);
        assert!(!wf.location_missing);
        assert!(!wf.headcount_is_fallback);
        assert!(wf.suggested_asset.is_none());
        assert_eq!(wf.resolved_headcount, 3);
        assert_eq!(wf.resolved_asset, Some(AssetType::Ambulance));
        assert!(wf.automated_callback_inquiry.is_none());
        assert!(wf.dispatcher_dial_alert.is_none());
        assert!(wf.field_operational_directives.is_empty());
    }
}
