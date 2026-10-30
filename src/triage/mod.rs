//! Triage workers (M3a): turn each report on the extraction stream into
//! exactly one incident, or a follow-up attached to one.
//!
//! Extraction runs behind [`TriageEngine`] so tests can script it; the real
//! engine reuses the existing stage 1 (LLM extraction, GPS snapping, landmark
//! lookup) without changing the legacy pipeline.

mod worker;

pub use worker::{TriageOutcome, TriageWorker, TRIAGE_GROUP};

use async_trait::async_trait;
use serde_json::json;

use crate::models::{
    suggest_asset, triage_is_dispatch_ready, CrisisState, ExtractionRecord, SosAlert, TriageReport,
    UrgencyResult,
};
use crate::pipeline::PipelineStages;

/// Output of a successful extraction. `errors` are stage findings that need
/// human review (e.g. location not resolved), not model failures.
#[derive(Debug, Clone)]
pub struct EngineOutput {
    pub triage: TriageReport,
    pub errors: Vec<String>,
}

#[async_trait]
pub trait TriageEngine: Send + Sync {
    /// Extract a candidate triage report. `Err` means the model call failed
    /// (counts toward the circuit breaker).
    async fn triage(&self, alert: &SosAlert) -> Result<EngineOutput, String>;
}

/// The production engine: the legacy pipeline's stage 1.
pub struct StageOneEngine {
    stages: PipelineStages,
}

impl StageOneEngine {
    pub fn new(stages: PipelineStages) -> Self {
        Self { stages }
    }
}

#[async_trait]
impl TriageEngine for StageOneEngine {
    async fn triage(&self, alert: &SosAlert) -> Result<EngineOutput, String> {
        let state = self
            .stages
            .stage_1_triage_extraction(CrisisState::new(alert.clone()))
            .await;
        match state.triage {
            Some(triage) => Ok(EngineOutput { triage, errors: state.errors }),
            None => Err(if state.errors.is_empty() {
                "triage extraction produced no report".to_string()
            } else {
                state.errors.join("; ")
            }),
        }
    }
}

/// Checkpoint record for a successful extraction.
pub fn record_from_output(output: &EngineOutput) -> ExtractionRecord {
    let triage = &output.triage;
    let mut review_reasons = triage.uncertainty_reasons.clone();
    review_reasons.extend(output.errors.iter().cloned());
    ExtractionRecord {
        succeeded: true,
        extraction_json: json!({ "triage": triage, "errors": output.errors }).to_string(),
        headcount: triage.headcount,
        required_asset: triage.required_asset.clone(),
        resolved_junction_id: triage.resolved_junction_id.clone(),
        dispatch_ready: output.errors.is_empty() && triage_is_dispatch_ready(triage),
        review_reasons,
        // A V3 extraction always carries urgency; anything else is UNKNOWN.
        urgency: triage.urgency.clone().unwrap_or_else(UrgencyResult::unknown_without_extraction),
        location_source: triage.resolved_junction_id.as_ref().and(triage.location_source),
        suggested_asset: match triage.required_asset {
            Some(_) => None,
            None => suggest_asset(triage),
        },
    }
}

/// Checkpoint record when no extraction is available. The report still
/// becomes an incident, for manual triage.
pub fn failed_record(reason: &str) -> ExtractionRecord {
    ExtractionRecord {
        succeeded: false,
        extraction_json: json!({ "error": reason }).to_string(),
        headcount: None,
        required_asset: None,
        resolved_junction_id: None,
        dispatch_ready: false,
        review_reasons: vec![reason.to_string()],
        urgency: UrgencyResult::unknown_without_extraction(),
        location_source: None,
        suggested_asset: None,
    }
}
