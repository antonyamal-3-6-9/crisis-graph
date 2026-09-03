use serde::{Deserialize, Serialize};
use super::triage::{SosAlert, TriageReport};
use super::dispatch::{Shelter, TacticalBrief};

/// Explicit typed state passed across all 4 stages of the pipeline (The Clipboard Pattern).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrisisState {
    pub sos: SosAlert,
    pub triage: Option<TriageReport>,
    pub assigned_shelter: Option<Shelter>,
    pub assigned_asset: Option<String>,
    pub computed_path: Option<Vec<String>>,
    pub total_distance_km: Option<f64>,
    pub detour_reason: Option<String>,
    pub brief: Option<TacticalBrief>,
    pub errors: Vec<String>,
}

impl CrisisState {
    pub fn new(sos: SosAlert) -> Self {
        Self {
            sos,
            triage: None,
            assigned_shelter: None,
            assigned_asset: None,
            computed_path: None,
            total_distance_km: None,
            detour_reason: None,
            brief: None,
            errors: Vec::new(),
        }
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}
