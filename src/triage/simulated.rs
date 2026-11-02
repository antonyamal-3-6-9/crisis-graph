//! Simulated extraction (M8 load tests only; `INFERENCE_BACKEND=simulated`).
//!
//! On this CPU the real model extracts about one report a minute, so a surge
//! test through it would only measure the model. The simulated engine stands
//! in for the model so the load test measures CrisisGraph itself: the surge
//! tool appends the scenario's ground truth to each report as a tag,
//! `#sim{...}`, and the engine returns those facts after a seeded,
//! log-normally distributed latency. Everything after extraction (Rust
//! urgency, location policy, deduplication, T1, the matcher) is real.
//!
//! Results produced with it are labelled "extraction: simulated". A report
//! without a valid tag goes to review, as an unreadable report would.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{EngineOutput, TriageEngine};
use crate::models::{
    derive_urgency, AssetType, LocationSource, SosAlert, TriageInferenceMetadata, TriageReport, VictimProfile,
};

/// Marker that starts the ground-truth tag in a simulated report.
pub const SIM_TAG: &str = "#sim";

/// Ground truth the surge tool embeds in a simulated report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SimTruth {
    pub headcount: Option<u32>,
    pub asset: Option<AssetType>,
    /// The victims' junction.
    pub junction: Option<String>,
    pub profile: VictimProfile,
}

impl SimTruth {
    /// The tag to append to a report's text.
    pub fn tag(&self) -> String {
        format!("{SIM_TAG}{}", serde_json::to_string(self).expect("serialisable"))
    }

    /// Read the tag back from a report's text.
    pub fn parse(text: &str) -> Option<Self> {
        let start = text.rfind(SIM_TAG)? + SIM_TAG.len();
        serde_json::from_str(text[start..].trim()).ok()
    }
}

pub struct SimulatedEngine {
    median_ms: u64,
}

impl SimulatedEngine {
    pub fn new(median_ms: u64) -> Self {
        Self { median_ms }
    }

    /// Log-normal latency (sigma 0.5) around the median, seeded by the
    /// report ID so a rerun of the same scenario sees the same latencies.
    pub fn latency(&self, report_id: &str) -> Duration {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in report_id.bytes() {
            h = (h ^ byte as u64).wrapping_mul(0x0100_0000_01b3);
        }
        // Box-Muller from two uniforms drawn from the hash.
        let u1 = ((h >> 11) as f64 / (1u64 << 53) as f64).max(1e-12);
        let h2 = h.rotate_left(29).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let u2 = (h2 >> 11) as f64 / (1u64 << 53) as f64;
        let normal = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        Duration::from_secs_f64(self.median_ms as f64 / 1000.0 * (0.5 * normal).exp())
    }
}

#[async_trait]
impl TriageEngine for SimulatedEngine {
    async fn triage(&self, alert: &SosAlert) -> Result<EngineOutput, String> {
        let latency = self.latency(&alert.alert_id);
        tokio::time::sleep(latency).await;
        let truth = SimTruth::parse(&alert.raw_text);
        let complete = truth.as_ref().is_some_and(|t| t.headcount.is_some() && t.asset.is_some() && t.junction.is_some());
        let truth = truth.unwrap_or(SimTruth { headcount: None, asset: None, junction: None, profile: VictimProfile::default() });
        let urgency = derive_urgency(&truth.profile);
        let mut reasons = Vec::new();
        if !complete {
            reasons.push("Simulated extraction: report has no complete ground-truth tag".to_string());
        }
        Ok(EngineOutput {
            triage: TriageReport {
                victim_location_raw: truth.junction.clone(),
                resolved_junction_id: truth.junction.clone(),
                headcount: truth.headcount,
                required_asset: truth.asset.clone(),
                hazards: vec![],
                confidence_score: if complete { 0.9 } else { 0.0 },
                needs_human_review: !complete,
                uncertainty_reasons: reasons,
                inference: TriageInferenceMetadata {
                    backend: "simulated".into(),
                    model: "simulated-ground-truth".into(),
                    adapter_id: None,
                    adapter_sha256: None,
                    prompt_version: "none".into(),
                    schema_version: "triage-extraction-v3".into(),
                    latency_ms: latency.as_millis() as u64,
                    tokens_per_second: None,
                    prompt_eval_ms: None,
                    completion_tokens: None,
                },
                victim_profile: Some(truth.profile),
                urgency: Some(urgency),
                location_source: truth.junction.is_some().then_some(LocationSource::Described),
            },
            errors: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MedicalCondition, WaterLevel};

    #[test]
    fn tag_round_trips_and_latency_is_seeded() {
        let truth = SimTruth {
            headcount: Some(3),
            asset: Some(AssetType::Ambulance),
            junction: Some("node/1".into()),
            profile: VictimProfile {
                medical_conditions: vec![MedicalCondition::Unconscious],
                water_level: Some(WaterLevel::Chest),
                ..Default::default()
            },
        };
        let text = format!("My father is unconscious near the temple, 3 of us {}", truth.tag());
        assert_eq!(SimTruth::parse(&text), Some(truth));
        assert_eq!(SimTruth::parse("no tag here"), None);

        let engine = SimulatedEngine::new(800);
        assert_eq!(engine.latency("report-1"), engine.latency("report-1"), "seeded by report");
        let mut samples: Vec<f64> = (0..2000).map(|i| engine.latency(&format!("r{i}")).as_secs_f64()).collect();
        samples.sort_by(f64::total_cmp);
        let median = samples[samples.len() / 2];
        assert!((0.65..0.95).contains(&median), "median near 0.8 s, got {median}");
    }
}
