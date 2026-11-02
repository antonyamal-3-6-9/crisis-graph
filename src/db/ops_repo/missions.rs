//! Boat-then-road missions (M6b): converting a task whose victims are cut
//! off by flood water into a boat leg and a road leg that waits for its
//! trigger (allocation design §3.7, relay design §4).
//!
//! The conversion is done by the matcher (owner decision 2026-11-01:
//! automatic, with a dispatcher alert). The water leg is never verified by
//! the system: there is no water network, so it is recorded as an estimate
//! for the crew to confirm.

use neo4rs::query;
use serde_json::{json, Value};

use super::{new_id, txn_first_row, OpsRepoError, OpsRepository, MAX_WRITE_ATTEMPTS, SYSTEM_ACTOR};
use crate::models::{AssetType, QueuedTask};

/// Escalation when no road and no boat-then-road plan reaches the victims.
pub const ESCALATION_NO_SURFACE_ROUTE: &str = "NO_SURFACE_ROUTE";

/// A planned conversion, built by the matcher from the multi-modal plan.
#[derive(Debug, Clone)]
pub struct MissionConversion {
    pub task: QueuedTask,
    pub staging_junction_id: String,
    pub water_legs: Value,
    /// Estimated water time (out to the victims), seconds.
    pub water_eta_s: f64,
    pub topology: String,
    pub road_asset: AssetType,
    pub overlay_version: u64,
}

impl OpsRepository {
    /// Convert a cut-off `ROAD_PICKUP` (one transaction, guarded by the task's
    /// version, so it happens once): the task is cancelled; a QUEUED
    /// `WATER_EXTRACTION` (boat, to the staging point) and a `PENDING_TRIGGER`
    /// `ROAD_EVACUATION` (road vehicle, from the staging point) replace it.
    /// Returns `(water_task_id, road_task_id)`, or `None` if the task changed.
    pub async fn convert_to_mission(&self, conversion: &MissionConversion) -> Result<Option<(String, String)>, OpsRepoError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut txn = self.neo4j.graph.start_txn().await?;
            let outcome = self.convert_in(&mut txn, conversion).await;
            let outcome = match outcome {
                Ok(Some(ids)) => txn.commit().await.map(|_| Some(ids)).map_err(OpsRepoError::from),
                other => {
                    let _ = txn.rollback().await;
                    other
                }
            };
            match outcome {
                Err(OpsRepoError::Transient(_)) if attempt < MAX_WRITE_ATTEMPTS => {
                    tokio::time::sleep(super::jittered_backoff(attempt)).await;
                }
                other => return other,
            }
        }
    }

    async fn convert_in(&self, txn: &mut neo4rs::Txn, c: &MissionConversion) -> Result<Option<(String, String)>, OpsRepoError> {
        let water_id = new_id();
        let road_id = new_id();
        let payload = json!({
            "incident_id": c.task.incident_id,
            "cancelled_task_id": c.task.task_id,
            "water_task_id": water_id,
            "road_task_id": road_id,
            "staging_junction_id": c.staging_junction_id,
            "topology": c.topology,
            "road_asset": c.road_asset.to_string(),
            "water_eta_s": c.water_eta_s,
            "water_leg": "UNVERIFIED_ESTIMATE",
            "overlay_version": c.overlay_version,
        })
        .to_string();
        let row = txn_first_row(
            txn,
            query(
                "MATCH (t:Task {task_id: $task_id}) \
                 SET t._lock = true \
                 REMOVE t._lock \
                 WITH t \
                 WHERE t.status = 'QUEUED' AND t.version = $version AND t.kind = 'ROAD_PICKUP' \
                 MATCH (i:OpsIncident {incident_id: t.incident_id}) \
                 WHERE NOT coalesce(i.boat_plan_declined, false) \
                 SET t.status = 'CANCELLED', t.version = t.version + 1, t.updated_at = datetime(), \
                     t.cancelled_at = datetime(), t.cancel_reason = 'cut off by flood water: boat-then-road mission' \
                 CREATE (i)-[:HAS_TASK {seq: 2}]->(:Task { \
                     task_id: $water_id, incident_id: i.incident_id, kind: 'WATER_EXTRACTION', status: 'QUEUED', \
                     required_asset: 'RESCUE_BOAT', urgency: t.urgency, priority_rank: t.priority_rank, version: 0, \
                     target_junction_id: $staging, staging_junction_id: $staging, water_legs_json: $legs, \
                     water_eta_s: $water_eta, created_at: datetime(), queued_at: t.queued_at }) \
                 CREATE (i)-[:HAS_TASK {seq: 3}]->(:Task { \
                     task_id: $road_id, incident_id: i.incident_id, kind: 'ROAD_EVACUATION', status: 'PENDING_TRIGGER', \
                     required_asset: $road_asset, urgency: t.urgency, priority_rank: t.priority_rank, version: 0, \
                     target_junction_id: $staging, staging_junction_id: $staging, depends_on: $water_id, \
                     created_at: datetime() }) \
                 SET i.mission = 'BOAT_THEN_ROAD', i.staging_junction_id = $staging, \
                     i.version = i.version + 1, i.updated_at = datetime() \
                 CREATE (t)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                     event_id: $task_audit, aggregate_type: 'Task', aggregate_id: t.task_id, action: 'TASK_CANCELLED', \
                     actor_id: $actor, from_state: 'QUEUED', to_state: 'CANCELLED', new_version: t.version, \
                     reason: 'cut off by flood water: converted to a boat-then-road mission', \
                     occurred_at: datetime(), details_json: $payload }) \
                 CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                     event_id: $incident_audit, aggregate_type: 'Incident', aggregate_id: i.incident_id, \
                     action: 'MISSION_CONVERTED', actor_id: $actor, new_version: i.version, \
                     reason: 'boat to staging ' + $staging + ', road leg on victim aboard', \
                     occurred_at: datetime(), details_json: $payload }) \
                 CREATE (:OutboxEvent { event_id: $alert_id, aggregate_id: i.incident_id, type: 'MISSION_CONVERTED', \
                     payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
                 CREATE (:OutboxEvent { event_id: $queued_id, aggregate_id: $water_id, type: 'TASK_QUEUED', \
                     payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
                 RETURN $water_id AS water",
            )
            .param("task_id", c.task.task_id.clone())
            .param("version", c.task.version as i64)
            .param("water_id", water_id.clone())
            .param("road_id", road_id.clone())
            .param("staging", c.staging_junction_id.clone())
            .param("legs", c.water_legs.to_string())
            .param("water_eta", c.water_eta_s)
            .param("road_asset", c.road_asset.to_string())
            .param("actor", SYSTEM_ACTOR)
            .param("payload", payload)
            .param("task_audit", new_id())
            .param("incident_audit", new_id())
            .param("alert_id", new_id())
            .param("queued_id", new_id()),
        )
        .await?;
        Ok(row.map(|_| (water_id, road_id)))
    }

    /// Junctions from which the dataset's boats start (any status): their
    /// positions, else their home shelters.
    pub async fn boat_starts(&self, dataset: &str) -> Result<Vec<String>, OpsRepoError> {
        let q = query(
            "MATCH (v:Vehicle {asset_type: 'RESCUE_BOAT'})-[:BASED_AT]->(s:Shelter {dataset: $dataset}) \
             WHERE v.status <> 'OUT_OF_SERVICE' \
             RETURN collect(DISTINCT coalesce(v.junction_id, s.junction_id)) AS starts",
        )
        .param("dataset", dataset);
        Ok(match self.first_row(q).await? {
            Some(row) => row.get::<Vec<String>>("starts")?,
            None => Vec::new(),
        })
    }

    /// Junctions of the dataset's depots (every shelter with vehicles): the
    /// road network the fleet can reach from.
    pub async fn depot_junctions(&self, dataset: &str) -> Result<Vec<String>, OpsRepoError> {
        let q = query(
            "MATCH (v:Vehicle)-[:BASED_AT]->(s:Shelter {dataset: $dataset}) \
             WHERE v.asset_type <> 'RESCUE_BOAT' \
             RETURN collect(DISTINCT s.junction_id) AS depots",
        )
        .param("dataset", dataset);
        Ok(match self.first_row(q).await? {
            Some(row) => row.get::<Vec<String>>("depots")?,
            None => Vec::new(),
        })
    }

    /// Whether any report of the incident extracted a medical condition.
    pub async fn incident_has_medical_need(&self, incident_id: &str) -> Result<bool, OpsRepoError> {
        let q = query(
            "MATCH (r:Report)-[:REPORTS]->(:OpsIncident {incident_id: $id}) \
             RETURN collect(r.extraction_json) AS extractions",
        )
        .param("id", incident_id);
        let extractions: Vec<Option<String>> = match self.first_row(q).await? {
            Some(row) => row.get("extractions")?,
            None => Vec::new(),
        };
        Ok(extractions.into_iter().flatten().any(|text| {
            serde_json::from_str::<Value>(&text).ok().is_some_and(|v| {
                v["triage"]["victim_profile"]["medical_conditions"].as_array().is_some_and(|m| !m.is_empty())
                    || v["triage"]["victim_profile"]["other_conditions"].as_array().is_some_and(|m| !m.is_empty())
            })
        }))
    }

    async fn first_row(&self, q: neo4rs::Query) -> Result<Option<neo4rs::Row>, OpsRepoError> {
        let mut result = self.neo4j.graph.execute(q).await?;
        Ok(result.next().await?)
    }
}
