use tracing::info;
use crate::models::{CrisisState, SosAlert};
use super::stages::PipelineStages;

#[derive(Clone)]
pub struct CrisisOrchestrator {
    stages: PipelineStages,
}

impl CrisisOrchestrator {
    pub fn new(stages: PipelineStages) -> Self {
        Self { stages }
    }

    /// Process an emergency SOS alert through all 4 pipeline stages
    pub async fn process_alert(&self, alert: SosAlert) -> CrisisState {
        info!(">>> Beginning CrisisGraph pipeline for alert: {} <<<", alert.alert_id);

        let mut state = CrisisState::new(alert);

        // Stage 1: Triage Extraction (AI Listener)
        state = self.stages.stage_1_triage_extraction(state).await;

        // Stage 2: Database Clerk (Asset & Shelter Allocation)
        state = self.stages.stage_2_asset_allocation(state).await;

        // Stage 3: Route Calculator (Deterministic Pathfinder)
        state = self.stages.stage_3_deterministic_pathfinder(state).await;

        // Stage 4: Safety Verifier & Brief Generator
        state = self.stages.stage_4_verifier_and_brief(state).await;

        info!("<<< CrisisGraph pipeline completed for alert: {} >>>", state.sos.alert_id);
        state
    }
}
