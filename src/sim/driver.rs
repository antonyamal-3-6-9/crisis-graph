//! Simulated driver (M6a, **demo only**): stands in for the responder app.
//!
//! It completes assigned tasks after a simulated mission time, through the
//! same task-complete command (T5) a responder or dispatcher uses, so the
//! demo exercises the real path. It keeps no state: each pass reads the
//! assigned tasks from Neo4j, so a restart loses nothing. The command's
//! request ID is derived from the task and its version, so two drivers (or
//! a restart) replay a completion instead of repeating it.

use std::time::Duration;

use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use tracing::{info, warn, Instrument};

use crate::config::PipelineParams;
use crate::db::{CompleteTaskCommand, OpsRepoError, OpsRepository, RedisClient, TriggerCommand};
use crate::models::{ActorContext, ActorRole, AssignedTask};
use crate::runtime::Shutdown;

pub const DRIVER_ACTOR: &str = "sim-driver";
const PASS_EVERY: Duration = Duration::from_secs(2);
const HEARTBEAT_TTL_SECS: u64 = 10;

/// Simulated seconds from assignment to completion: drive to the victims
/// (`eta_s`), time on scene, and the drive to the drop-off (approximated by
/// `eta_s` again), scaled by `scale`.
pub fn mission_secs(eta_s: f64, on_scene_secs: u64, scale: f64) -> f64 {
    (eta_s.max(0.0) * 2.0 + on_scene_secs as f64) * scale
}

/// Redis key the driver refreshes while it runs (shown in the console).
pub fn heartbeat_key(dataset: &str) -> String {
    format!("sim:driver:{dataset}")
}

pub struct SimDriver {
    repo: OpsRepository,
    redis: RedisClient,
    params: PipelineParams,
    actor: ActorContext,
}

impl SimDriver {
    pub fn new(repo: OpsRepository, redis: RedisClient, params: PipelineParams) -> Self {
        Self { repo, redis, params, actor: ActorContext::new(DRIVER_ACTOR, [ActorRole::Responder]) }
    }

    /// Complete every assigned task whose mission time has passed at `now`.
    /// Returns the tasks completed by this pass (replays are not counted).
    pub async fn run_once(&self, now: DateTime<Utc>) -> Result<Vec<String>, OpsRepoError> {
        let mut completed = Vec::new();
        for task in self.repo.assigned_tasks(&self.params.graph_dataset).await? {
            // A span (in the task's trace) only when the crew acts, not for
            // every poll while the mission is still under way.
            if !self.action_due(&task, now) {
                continue;
            }
            let span = tracing::info_span!("driver.mission", task_id = %task.task_id, kind = %task.kind);
            crate::runtime::telemetry::set_parent(&span, task.traceparent.as_deref());
            if let Some(id) = self.mission_step(task, now).instrument(span).await? {
                completed.push(id);
            }
        }
        Ok(completed)
    }

    /// One assigned task: report "victim aboard" halfway through a boat
    /// mission, complete it when due. Returns the task ID if completed now.
    /// Whether the crew reports something now: completion, or "victim
    /// aboard" from halfway through a boat mission.
    fn action_due(&self, task: &AssignedTask, now: DateTime<Utc>) -> bool {
        let due = mission_secs(task.eta_s, self.params.sim_on_scene_secs, self.params.sim_time_scale);
        let elapsed = (now - task.assigned_at).num_milliseconds() as f64 / 1000.0;
        elapsed >= due || (task.kind == "WATER_EXTRACTION" && elapsed >= due / 2.0)
    }

    async fn mission_step(&self, task: AssignedTask, now: DateTime<Utc>) -> Result<Option<String>, OpsRepoError> {
        let AssignedTask { task_id, version, assigned_at, eta_s, kind, .. } = task;
        {
            let due = mission_secs(eta_s, self.params.sim_on_scene_secs, self.params.sim_time_scale);
            let elapsed = (now - assigned_at).num_milliseconds() as f64 / 1000.0;
            // A boat crew reports "victim aboard" halfway through, so the
            // road leg is sent to meet it (M6b). Replays are harmless.
            if kind == "WATER_EXTRACTION" && elapsed >= due / 2.0 {
                let aboard = TriggerCommand {
                    request_id: format!("sim:aboard:{task_id}"),
                    task_id: task_id.clone(),
                    expected_version: version,
                    reason: "VICTIM_ABOARD".into(),
                };
                match self.repo.trigger_road_leg(&self.actor, &aboard).await {
                    Ok(outcome) if !outcome.replayed => info!(task_id = %task_id, "Simulated crew: victim aboard"),
                    Ok(_) | Err(OpsRepoError::NotAvailable(_) | OpsRepoError::InvalidData(_) | OpsRepoError::NotFound(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            if elapsed < due {
                return Ok(None);
            }
            let command = CompleteTaskCommand {
                request_id: format!("sim:{task_id}:{version}"),
                task_id: task_id.clone(),
                expected_version: version,
                note: Some(format!("simulated mission completed ({due:.0} s simulated)")),
            };
            match self.repo.complete_task(&self.actor, &command).await {
                Ok(outcome) if !outcome.replayed => {
                    info!(task_id = %task_id, vehicle = ?outcome.outcome["vehicle_id"], "Simulated mission completed");
                    Ok(Some(task_id))
                }
                Ok(_) => Ok(None),
                // Changed meanwhile (dispatcher completed it, breakdown): skip.
                Err(OpsRepoError::NotAvailable(_) | OpsRepoError::NotFound(_)) => Ok(None),
                Err(error) => Err(error),
            }
        }
    }

    /// Run passes until shutdown, refreshing the heartbeat key.
    pub async fn run(self, shutdown: Shutdown) {
        let key = heartbeat_key(&self.params.graph_dataset);
        warn!(
            dataset = %self.params.graph_dataset,
            time_scale = self.params.sim_time_scale,
            "Simulated driver running: tasks will be completed automatically (demo only)"
        );
        while !shutdown.is_triggered() {
            let mut conn = self.redis.manager.clone();
            let _: Result<(), _> = conn.set_ex(&key, DRIVER_ACTOR, HEARTBEAT_TTL_SECS).await;
            if let Err(error) = self.run_once(Utc::now()).await {
                warn!(%error, "Simulated driver pass failed");
            }
            tokio::select! {
                _ = tokio::time::sleep(PASS_EVERY) => {}
                _ = shutdown.wait() => break,
            }
        }
        let mut conn = self.redis.manager.clone();
        let _: Result<(), _> = conn.del(&key).await;
        info!("Simulated driver stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::mission_secs;

    #[test]
    fn mission_time_covers_both_drives_and_the_scene() {
        assert_eq!(mission_secs(100.0, 300, 1.0), 500.0);
        assert_eq!(mission_secs(100.0, 300, 0.1), 50.0);
        assert_eq!(mission_secs(-5.0, 60, 1.0), 60.0, "a missing ETA counts as zero");
    }
}
