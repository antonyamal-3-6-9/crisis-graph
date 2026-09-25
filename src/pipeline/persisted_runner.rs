use chrono::Utc;
use tracing::{error, info};

use super::PipelineStages;
use crate::db::IncidentStore;
use crate::models::{
    triage_is_dispatch_ready, ActorContext, ActorRole, CrisisState, DispatchStatus, Incident,
    IncidentStatus, SosAlert, TriageReport,
};

#[derive(Debug, Clone)]
pub struct PersistedProcessResult {
    pub state: CrisisState,
    /// Last incident version known to have been persisted successfully.
    pub incident: Incident,
    pub persistence_healthy: bool,
    /// True when a durable incident with the same ID already existed. No
    /// pipeline stages or resource mutations were repeated.
    pub duplicate_suppressed: bool,
}

#[derive(Clone)]
pub struct PersistedCrisisOrchestrator<S>
where
    S: IncidentStore,
{
    stages: PipelineStages,
    store: S,
    system_actor: ActorContext,
}

impl<S> PersistedCrisisOrchestrator<S>
where
    S: IncidentStore,
{
    pub fn new(stages: PipelineStages, store: S) -> Self {
        Self {
            stages,
            store,
            system_actor: ActorContext::new("crisisgraph-pipeline", [ActorRole::SystemService]),
        }
    }

    /// Runs the four-stage pipeline while persisting every control-plane state
    /// transition. A persistence failure stops further operational work.
    pub async fn process_alert(&self, alert: SosAlert) -> PersistedProcessResult {
        info!(alert_id = %alert.alert_id, "Starting persisted CrisisGraph pipeline");
        let mut state = CrisisState::new(alert.clone());
        let (mut incident, receipt) = Incident::receive(alert, &self.system_actor, Utc::now())
            .expect("static system actor must be authorized to receive incidents");

        if let Err(error) = self.store.create_incident(&incident, &receipt).await {
            if matches!(error, crate::db::IncidentRepositoryError::AlreadyExists(_)) {
                match self.store.load_incident(&incident.incident_id).await {
                    Ok(Some(existing)) => {
                        info!(
                            incident_id = %existing.incident_id,
                            version = existing.version,
                            "Suppressing duplicate incident delivery"
                        );
                        return PersistedProcessResult {
                            state,
                            incident: existing,
                            persistence_healthy: true,
                            duplicate_suppressed: true,
                        };
                    }
                    Ok(None) => {}
                    Err(load_error) => {
                        return self
                            .stop_on_persistence_failure(state, incident, load_error.to_string())
                            .await;
                    }
                }
            }
            return self
                .stop_on_persistence_failure(state, incident, error.to_string())
                .await;
        }

        state = self.stages.stage_1_triage_extraction(state).await;
        let Some(triage) = state.triage.clone() else {
            if let Err(error) = self
                .persist_transition(
                    &mut incident,
                    IncidentStatus::ReviewRequired,
                    review_reason(&state, "Triage extraction failed"),
                )
                .await
            {
                return self
                    .stop_on_persistence_failure(state, incident, error)
                    .await;
            }
            state = self.stages.stage_4_verifier_and_brief(state).await;
            return persisted_result(state, incident);
        };

        if let Err(error) = self
            .persist_model_extraction(&mut incident, triage.clone())
            .await
        {
            return self
                .stop_on_persistence_failure(state, incident, error)
                .await;
        }

        if state.has_errors() || !triage_is_dispatch_ready(&triage) {
            if let Err(error) = self
                .persist_transition(
                    &mut incident,
                    IncidentStatus::ReviewRequired,
                    review_reason(&state, "Triage requires dispatcher review"),
                )
                .await
            {
                return self
                    .stop_on_persistence_failure(state, incident, error)
                    .await;
            }
            state = self.stages.stage_4_verifier_and_brief(state).await;
            return persisted_result(state, incident);
        }

        if let Err(error) = self
            .persist_transition(
                &mut incident,
                IncidentStatus::TriageApproved,
                "Deterministic triage guard accepted all dispatch-critical fields",
            )
            .await
        {
            return self
                .stop_on_persistence_failure(state, incident, error)
                .await;
        }
        if let Err(error) = self
            .persist_transition(
                &mut incident,
                IncidentStatus::Allocating,
                "Beginning atomic shelter and asset allocation",
            )
            .await
        {
            return self
                .stop_on_persistence_failure(state, incident, error)
                .await;
        }

        state = self.stages.stage_2_asset_allocation(state).await;
        if state.has_errors() {
            return self
                .persist_review_or_stop(state, incident, "Resource allocation failed")
                .await;
        }

        if let Err(error) = self
            .persist_transition(
                &mut incident,
                IncidentStatus::Routing,
                "Resource allocation completed; beginning deterministic routing",
            )
            .await
        {
            return self
                .stop_on_persistence_failure(state, incident, error)
                .await;
        }

        state = self.stages.stage_3_deterministic_pathfinder(state).await;
        if state.has_errors() {
            return self
                .persist_review_or_stop(state, incident, "Deterministic routing failed")
                .await;
        }

        if let Err(error) = self
            .persist_transition(
                &mut incident,
                IncidentStatus::Verifying,
                "Route calculated; beginning independent safety verification",
            )
            .await
        {
            return self
                .stop_on_persistence_failure(state, incident, error)
                .await;
        }

        state = self.stages.stage_4_verifier_and_brief(state).await;
        let verified = state
            .brief
            .as_ref()
            .is_some_and(|brief| brief.status == DispatchStatus::RoutedVerified);
        if !verified {
            state.errors.push(
                "RouteVerificationFailed: independent safety verification failed".to_string(),
            );
            state = self
                .stages
                .compensate_resource_allocation(state, "Independent route verification failed")
                .await;
            if let Err(error) = self
                .persist_transition(
                    &mut incident,
                    IncidentStatus::ReviewRequired,
                    "Independent route verification failed",
                )
                .await
            {
                return self
                    .stop_on_persistence_failure(state, incident, error)
                    .await;
            }
            state = self.stages.stage_4_verifier_and_brief(state).await;
            return persisted_result(state, incident);
        }

        if let Err(error) = self
            .persist_transition(
                &mut incident,
                IncidentStatus::RouteVerified,
                "Independent route verification passed",
            )
            .await
        {
            return self
                .stop_on_persistence_failure(state, incident, error)
                .await;
        }

        persisted_result(state, incident)
    }

    async fn persist_model_extraction(
        &self,
        incident: &mut Incident,
        triage: TriageReport,
    ) -> Result<(), String> {
        let mut candidate = incident.clone();
        let event = candidate
            .append_model_extraction(&self.system_actor, incident.version, triage, Utc::now())
            .map_err(|error| error.to_string())?;
        self.store
            .save_incident_transition(&candidate, &event)
            .await
            .map_err(|error| error.to_string())?;
        *incident = candidate;
        Ok(())
    }

    async fn persist_transition(
        &self,
        incident: &mut Incident,
        next: IncidentStatus,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        let mut candidate = incident.clone();
        let event = candidate
            .transition(
                &self.system_actor,
                incident.version,
                next,
                reason,
                Utc::now(),
            )
            .map_err(|error| error.to_string())?;
        self.store
            .save_incident_transition(&candidate, &event)
            .await
            .map_err(|error| error.to_string())?;
        *incident = candidate;
        Ok(())
    }

    async fn persist_review_or_stop(
        &self,
        mut state: CrisisState,
        mut incident: Incident,
        fallback_reason: &str,
    ) -> PersistedProcessResult {
        state = self
            .stages
            .compensate_resource_allocation(state, fallback_reason)
            .await;
        let reason = review_reason(&state, fallback_reason);
        if let Err(error) = self
            .persist_transition(&mut incident, IncidentStatus::ReviewRequired, reason)
            .await
        {
            return self
                .stop_on_persistence_failure(state, incident, error)
                .await;
        }
        let state = self.stages.stage_4_verifier_and_brief(state).await;
        persisted_result(state, incident)
    }

    async fn stop_on_persistence_failure(
        &self,
        mut state: CrisisState,
        incident: Incident,
        error_message: String,
    ) -> PersistedProcessResult {
        error!(
            incident_id = %incident.incident_id,
            error = %error_message,
            "Control-plane persistence failed; stopping dispatch"
        );
        state
            .errors
            .push(format!("IncidentPersistenceError: {error_message}"));
        state = self
            .stages
            .compensate_resource_allocation(state, "Control-plane persistence failed")
            .await;
        state = self.stages.stage_4_verifier_and_brief(state).await;
        PersistedProcessResult {
            state,
            incident,
            persistence_healthy: false,
            duplicate_suppressed: false,
        }
    }
}

fn review_reason(state: &CrisisState, fallback: &str) -> String {
    if state.errors.is_empty() {
        fallback.to_string()
    } else {
        state.errors.join("; ")
    }
}

fn persisted_result(state: CrisisState, incident: Incident) -> PersistedProcessResult {
    PersistedProcessResult {
        state,
        incident,
        persistence_healthy: true,
        duplicate_suppressed: false,
    }
}
