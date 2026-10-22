use serde::{Deserialize, Serialize};
use super::triage::{AssetType, RoadStatus, SosAlert, TriageReport};

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
    /// True if required asset was inferred from environmental/keyword context
    pub asset_is_fallback: bool,
    pub original_asset: Option<AssetType>,
    pub resolved_asset: AssetType,
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

        // 3. Asset Waterfall: Infer asset from environmental and contextual cues
        let (asset_is_fallback, resolved_asset) = match &triage.required_asset {
            Some(asset) => (false, asset.clone()),
            None => {
                let lower = alert.raw_text.to_lowercase();
                let inferred = if lower.contains("flood")
                    || lower.contains("water")
                    || lower.contains("river")
                    || lower.contains("drown")
                    || lower.contains("submerged")
                    || lower.contains("vellam")
                    || lower.contains("munki")
                    || lower.contains("boat")
                    || triage.hazards.iter().any(|h| h.status == Some(RoadStatus::Flooded))
                {
                    AssetType::RescueBoat
                } else if lower.contains("injur")
                    || lower.contains("bleed")
                    || lower.contains("blood")
                    || lower.contains("patient")
                    || lower.contains("unconscious")
                    || lower.contains("cardiac")
                    || lower.contains("hospital")
                    || lower.contains("ambulance")
                {
                    AssetType::Ambulance
                } else if lower.contains("truck")
                    || lower.contains("lorry")
                    || lower.contains("heavy")
                    || lower.contains("debris")
                    || lower.contains("tree")
                {
                    AssetType::EvacTruck
                } else {
                    // Standard emergency medical response
                    AssetType::Ambulance
                };
                (true, inferred)
            }
        };

        // 4. Generate operational directives for field responders
        let mut field_operational_directives = Vec::new();
        if headcount_is_fallback {
            field_operational_directives.push(format!(
                "Headcount unconfirmed by caller. Dispatched standard baseline unit (capacity: {}). Driver: Verify exact count on arrival and request secondary transport if capacity exceeded.",
                DEFAULT_BASELINE_HEADCOUNT
            ));
        }
        if asset_is_fallback {
            field_operational_directives.push(format!(
                "Required asset unspecified by caller; inferred as '{}' from incident context. Responders: Conduct terrain and approach assessment.",
                resolved_asset
            ));
        }

        Self {
            location_missing,
            automated_callback_inquiry,
            dispatcher_dial_alert,
            headcount_is_fallback,
            original_headcount: triage.headcount,
            resolved_headcount,
            asset_is_fallback,
            original_asset: triage.required_asset.clone(),
            resolved_asset,
            field_operational_directives,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
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
        let alert = SosAlert {
            alert_id: "SOS-NO-LOC-1".into(),
            raw_text: "Water is rising fast, we need urgent help!".into(),
            timestamp: Utc::now(),
            source_channel: Some("web".into()),
        };
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
        };

        let wf = UncertaintyWaterfall::resolve(&alert, &triage);
        assert!(wf.location_missing);
        assert!(wf.automated_callback_inquiry.unwrap().contains("Location not identified"));
        assert!(wf.dispatcher_dial_alert.unwrap().contains("Immediate operator phone dial required"));
        assert!(!wf.headcount_is_fallback);
        assert!(!wf.asset_is_fallback);
    }

    #[test]
    fn test_unspecified_headcount_falls_back_to_baseline_unit() {
        let alert = SosAlert {
            alert_id: "SOS-NO-COUNT-2".into(),
            raw_text: "Family trapped at Bank Junction!".into(),
            timestamp: Utc::now(),
            source_channel: Some("web".into()),
        };
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
        };

        let wf = UncertaintyWaterfall::resolve(&alert, &triage);
        assert!(!wf.location_missing);
        assert!(wf.headcount_is_fallback);
        assert_eq!(wf.resolved_headcount, DEFAULT_BASELINE_HEADCOUNT);
        assert!(wf.automated_callback_inquiry.unwrap().contains("How many people require evacuation?"));
        assert!(wf.field_operational_directives.iter().any(|d| d.contains("standard baseline unit")));
    }

    #[test]
    fn test_unspecified_asset_infers_boat_from_flood_context() {
        let alert = SosAlert {
            alert_id: "SOS-FLOOD-3".into(),
            raw_text: "Periyar water entered our house, roads completely flooded and munki!".into(),
            timestamp: Utc::now(),
            source_channel: Some("web".into()),
        };
        let triage = TriageReport {
            victim_location_raw: Some("Aluva Manappuram".into()),
            resolved_junction_id: Some("node/7048449098".into()),
            headcount: Some(5),
            required_asset: None, // Missing
            hazards: vec![],
            confidence_score: 0.9,
            needs_human_review: true,
            uncertainty_reasons: vec!["Required asset was not specified".into()],
            inference: mock_metadata(),
        };

        let wf = UncertaintyWaterfall::resolve(&alert, &triage);
        assert!(!wf.location_missing);
        assert!(!wf.headcount_is_fallback);
        assert!(wf.asset_is_fallback);
        assert_eq!(wf.resolved_asset, AssetType::RescueBoat);
        assert!(wf.field_operational_directives.iter().any(|d| d.contains("RESCUE_BOAT")));
    }

    #[test]
    fn test_complete_triage_has_no_fallbacks() {
        let alert = SosAlert {
            alert_id: "SOS-CLEAN-4".into(),
            raw_text: "3 injured passengers at Aluva Station need ambulance.".into(),
            timestamp: Utc::now(),
            source_channel: Some("web".into()),
        };
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
        };

        let wf = UncertaintyWaterfall::resolve(&alert, &triage);
        assert!(!wf.location_missing);
        assert!(!wf.headcount_is_fallback);
        assert!(!wf.asset_is_fallback);
        assert_eq!(wf.resolved_headcount, 3);
        assert_eq!(wf.resolved_asset, AssetType::Ambulance);
        assert!(wf.automated_callback_inquiry.is_none());
        assert!(wf.dispatcher_dial_alert.is_none());
        assert!(wf.field_operational_directives.is_empty());
    }
}
