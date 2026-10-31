//! Matcher allocation (M5): the matcher's reads, transaction T2's commit
//! step, failed-verification release (T3) and escalations.
//!
//! T2 runs as hold → live route verification → commit. The hold
//! (`hold_vehicle`) leaves the task `QUEUED`; only the commit moves it to
//! `ASSIGNED`, so a hold that expires or fails verification needs to free
//! only the vehicle. The commit is fenced by the matcher's lease token, so a
//! former leader that lost its lease cannot assign anything.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use neo4rs::{query, Row, Txn};
use serde_json::json;

use super::beds::reserve_beds_query;
use super::{new_id, parse_asset, parse_time, txn_first_row, OpsRepoError, OpsRepository, SYSTEM_ACTOR};
use crate::models::{AssetType, DropoffShelter, OrphanedHold, QueuedTask};

/// Escalation reasons (each raised at most once per task).
pub const ESCALATION_OVERDUE: &str = "OVERDUE";
pub const ESCALATION_NO_ELIGIBLE_VEHICLE: &str = "NO_ELIGIBLE_VEHICLE";
pub const ESCALATION_ROUTE_VERIFICATION_FAILED: &str = "ROUTE_VERIFICATION_FAILED";

const QUEUED_TASK_FIELDS: &str = "t.task_id AS task_id, t.incident_id AS incident_id, t.version AS version, \
     t.required_asset AS required_asset, coalesce(t.priority_rank, 1) AS priority_rank, \
     toString(coalesce(t.queued_at, t.created_at)) AS queued_at, i.headcount AS headcount, \
     j.id AS junction_id, coalesce(t.verify_failures, 0) AS verify_failures, \
     coalesce(t.escalations, []) AS escalations";

/// Everything the T2 commit records.
#[derive(Debug, Clone)]
pub struct AssignmentCommit {
    pub reservation_id: String,
    pub task_id: String,
    pub vehicle_id: String,
    /// Task version the matcher read; a changed task is not assigned.
    pub task_version: u64,
    /// Lease name and fencing token of the matcher's tenure.
    pub fence_name: String,
    pub fence_token: u64,
    pub route_json: String,
    pub overlay_version: u64,
    pub eta_s: f64,
    /// Drop-off shelter and beds to reserve; `None` when no shelter fits.
    pub dropoff: Option<(String, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentOutcome {
    /// Shelter whose beds were reserved; `None` raised `DROPOFF_UNRESOLVED`.
    pub dropoff_shelter: Option<String>,
}

impl OpsRepository {
    /// `QUEUED` tasks in `dataset`, all of them or only `ids`. The dataset
    /// is the incident junction's, so other datasets' tasks are never seen.
    pub async fn queued_tasks(&self, dataset: &str, ids: Option<&[String]>) -> Result<Vec<QueuedTask>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (i:OpsIncident)-[:HAS_TASK]->(t:Task {{status: 'QUEUED'}}), \
                   (i)-[:LOCATED_AT]->(j:Junction {{dataset: $dataset}}) \
             WHERE $ids IS NULL OR t.task_id IN $ids \
             RETURN {QUEUED_TASK_FIELDS} ORDER BY t.task_id"
        ))
        .param("dataset", dataset)
        .param("ids", ids.map(|ids| ids.to_vec()));
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut tasks = Vec::new();
        while let Some(row) = result.next().await? {
            tasks.push(queued_task_from_row(&row)?);
        }
        Ok(tasks)
    }

    /// Largest capacity per asset type over the dataset's whole fleet (any
    /// status but out of service): decides whether a task can ever be served.
    pub async fn fleet_max_capacity(&self, dataset: &str) -> Result<HashMap<String, u32>, OpsRepoError> {
        let q = query(
            "MATCH (v:Vehicle)-[:BASED_AT]->(:Shelter {dataset: $dataset}) \
             WHERE v.status <> 'OUT_OF_SERVICE' \
             RETURN v.asset_type AS asset_type, max(v.capacity_persons) AS capacity",
        )
        .param("dataset", dataset);
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut capacities = HashMap::new();
        while let Some(row) = result.next().await? {
            capacities.insert(row.get::<String>("asset_type")?, row.get::<i64>("capacity")? as u32);
        }
        Ok(capacities)
    }

    /// Shelters in `dataset` with at least one free bed.
    pub async fn dropoff_shelters(&self, dataset: &str) -> Result<Vec<DropoffShelter>, OpsRepoError> {
        let q = query(
            "MATCH (s:Shelter {dataset: $dataset}) \
             WHERE s.junction_id IS NOT NULL AND s.capacity - s.current_occupancy > 0 \
             RETURN s.id AS shelter_id, s.junction_id AS junction_id, \
                    s.capacity - s.current_occupancy AS free_beds ORDER BY s.id",
        )
        .param("dataset", dataset);
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut shelters = Vec::new();
        while let Some(row) = result.next().await? {
            shelters.push(DropoffShelter {
                shelter_id: row.get("shelter_id")?,
                junction_id: row.get("junction_id")?,
                free_beds: row.get::<i64>("free_beds")?.max(0) as u32,
            });
        }
        Ok(shelters)
    }

    /// Vehicle holds in `dataset` that were never committed and have not
    /// expired, for tasks still `QUEUED` (the matcher stopped in between).
    pub async fn orphaned_holds(&self, dataset: &str) -> Result<Vec<OrphanedHold>, OpsRepoError> {
        let q = query(
            "MATCH (r:Reservation {status: 'HELD', resource_kind: 'VEHICLE'})-[:OF_VEHICLE]->(v:Vehicle), \
                   (v)-[:BASED_AT]->(:Shelter {dataset: $dataset}) \
             WHERE r.held_until > datetime() \
             MATCH (t:Task {task_id: r.task_id, status: 'QUEUED'}) \
             RETURN r.reservation_id AS reservation_id, r.task_id AS task_id, v.vehicle_id AS vehicle_id \
             ORDER BY r.created_at",
        )
        .param("dataset", dataset);
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut holds = Vec::new();
        while let Some(row) = result.next().await? {
            holds.push(OrphanedHold {
                reservation_id: row.get("reservation_id")?,
                task_id: row.get("task_id")?,
                vehicle_id: row.get("vehicle_id")?,
            });
        }
        Ok(holds)
    }

    /// T2 commit, one transaction: fence check · reservation `COMMITTED` ·
    /// vehicle `ASSIGNED` · task `QUEUED → ASSIGNED` (version-checked) with
    /// the route and its `overlay_version` · incident version + 1 · drop-off
    /// beds · audit · outbox `TASK_ASSIGNED` (+ `DROPOFF_UNRESOLVED`).
    ///
    /// `Fenced` if a newer matcher tenure exists; `NotAvailable` if the hold
    /// is no longer active or the task changed. Either way nothing is
    /// written, and the caller releases the hold.
    pub async fn commit_assignment(&self, commit: &AssignmentCommit) -> Result<AssignmentOutcome, OpsRepoError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut txn = self.neo4j.graph.start_txn().await?;
            let outcome = commit_in(&mut txn, commit).await;
            let outcome = match outcome {
                Ok(outcome) => txn.commit().await.map(|_| outcome).map_err(OpsRepoError::from),
                Err(error) => {
                    let _ = txn.rollback().await;
                    Err(error)
                }
            };
            match outcome {
                Err(OpsRepoError::Transient(_)) if attempt < super::MAX_WRITE_ATTEMPTS => {
                    tokio::time::sleep(super::jittered_backoff(attempt)).await;
                }
                other => return other,
            }
        }
    }

    /// Count a failed live route check on a still-queued task. When the count
    /// reaches `max_failures`, the task is escalated once
    /// (`ROUTE_VERIFICATION_FAILED`). Returns whether it was escalated now.
    pub async fn record_verify_failure(
        &self,
        task_id: &str,
        max_failures: u32,
        segments: &[String],
    ) -> Result<bool, OpsRepoError> {
        let q = query(
            "MATCH (t:Task {task_id: $task_id, status: 'QUEUED'}) \
             SET t.verify_failures = coalesce(t.verify_failures, 0) + 1, t.updated_at = datetime() \
             RETURN t.verify_failures AS failures, \
                    $reason IN coalesce(t.escalations, []) AS already",
        )
        .param("task_id", task_id)
        .param("reason", ESCALATION_ROUTE_VERIFICATION_FAILED);
        let Some(row) = self.write_first_row(q).await? else {
            return Ok(false);
        };
        let failures = row.get::<i64>("failures")? as u32;
        if failures < max_failures || row.get::<bool>("already")? {
            return Ok(false);
        }
        self.escalate_task(
            task_id,
            ESCALATION_ROUTE_VERIFICATION_FAILED,
            json!({ "verify_failures": failures, "segments": segments }),
        )
        .await
    }

    /// Raise one escalation for a queued task: the reason is recorded on the
    /// task, with audit and outbox `TASK_ESCALATED`. A reason already raised
    /// is not raised again. Returns whether it was raised now.
    pub async fn escalate_task(
        &self,
        task_id: &str,
        reason: &str,
        details: serde_json::Value,
    ) -> Result<bool, OpsRepoError> {
        let q = query(
            "MATCH (t:Task {task_id: $task_id, status: 'QUEUED'}) \
             WHERE NOT $reason IN coalesce(t.escalations, []) \
             SET t.escalations = coalesce(t.escalations, []) + $reason, t.escalated_at = datetime() \
             CREATE (t)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Task', aggregate_id: $task_id, \
                 action: 'TASK_ESCALATED', actor_id: $actor_id, reason: $reason, \
                 occurred_at: datetime(), details_json: $details }) \
             CREATE (:OutboxEvent { \
                 event_id: $outbox_id, aggregate_id: $task_id, type: 'TASK_ESCALATED', \
                 payload_json: '{\"task_id\":\"' + $task_id + '\",\"incident_id\":\"' + t.incident_id + \
                     '\",\"reason\":\"' + $reason + '\",\"details\":' + $details + '}', \
                 created_at: datetime(), published: false, attempts: 0 }) \
             RETURN t.task_id AS task_id",
        )
        .param("task_id", task_id)
        .param("reason", reason)
        .param("details", details.to_string())
        .param("audit_id", new_id())
        .param("outbox_id", new_id())
        .param("actor_id", SYSTEM_ACTOR);
        Ok(self.write_first_row(q).await?.is_some())
    }
}

async fn commit_in(txn: &mut Txn, commit: &AssignmentCommit) -> Result<AssignmentOutcome, OpsRepoError> {
    // Fence first: record our token, or stop if a newer tenure has written.
    let fenced = txn_first_row(
        txn,
        query(
            "MERGE (f:LeaseFence {name: $fence}) \
             ON CREATE SET f.token = $token \
             SET f._lock = true \
             REMOVE f._lock \
             WITH f \
             WHERE f.token <= $token \
             SET f.token = $token, f.updated_at = datetime() \
             RETURN f.token AS token",
        )
        .param("fence", commit.fence_name.clone())
        .param("token", commit.fence_token as i64),
    )
    .await?;
    if fenced.is_none() {
        return Err(OpsRepoError::Fenced(format!(
            "{} token {} is older than the current matcher",
            commit.fence_name, commit.fence_token
        )));
    }

    let assigned = txn_first_row(
        txn,
        query(
            "MATCH (r:Reservation {reservation_id: $reservation_id, status: 'HELD'})-[:OF_VEHICLE]->\
                   (v:Vehicle {vehicle_id: $vehicle_id}) \
             MATCH (t:Task {task_id: $task_id}) \
             WHERE r.task_id = t.task_id AND t.status = 'QUEUED' AND t.version = $task_version \
             MATCH (i:OpsIncident {incident_id: t.incident_id}) \
             SET r.status = 'COMMITTED', r.committed_at = datetime(), r.held_until = null, \
                 v.status = 'ASSIGNED', v.version = v.version + 1, v.updated_at = datetime(), \
                 t.status = 'ASSIGNED', t.version = t.version + 1, t.updated_at = datetime(), \
                 t.assigned_at = datetime(), t.assigned_vehicle_id = $vehicle_id, t.eta_s = $eta_s, \
                 t.route_json = $route_json, t.overlay_version = $overlay_version, t.routed_at = datetime(), \
                 i.version = i.version + 1, i.updated_at = datetime(), \
                 i.status = CASE WHEN i.status = 'TRIAGE_APPROVED' THEN 'ASSIGNED' ELSE i.status END \
             CREATE (v)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $vehicle_audit_id, aggregate_type: 'Vehicle', aggregate_id: $vehicle_id, \
                 action: 'VEHICLE_ASSIGNED', actor_id: $actor_id, from_state: 'HELD', to_state: 'ASSIGNED', \
                 previous_version: v.version - 1, new_version: v.version, \
                 reason: 'assigned to task ' + $task_id, occurred_at: datetime(), details_json: '{}' }) \
             CREATE (t)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $task_audit_id, aggregate_type: 'Task', aggregate_id: $task_id, \
                 action: 'TASK_ASSIGNED', actor_id: $actor_id, from_state: 'QUEUED', to_state: 'ASSIGNED', \
                 previous_version: $task_version, new_version: t.version, \
                 reason: 'matcher assigned vehicle ' + $vehicle_id, occurred_at: datetime(), \
                 details_json: '{\"overlay_version\":' + toString($overlay_version) + '}' }) \
             RETURN t.incident_id AS incident_id",
        )
        .param("reservation_id", commit.reservation_id.clone())
        .param("vehicle_id", commit.vehicle_id.clone())
        .param("task_id", commit.task_id.clone())
        .param("task_version", commit.task_version as i64)
        .param("eta_s", commit.eta_s)
        .param("route_json", commit.route_json.clone())
        .param("overlay_version", commit.overlay_version as i64)
        .param("vehicle_audit_id", new_id())
        .param("task_audit_id", new_id())
        .param("actor_id", SYSTEM_ACTOR),
    )
    .await?;
    let Some(assigned) = assigned else {
        return Err(OpsRepoError::NotAvailable(format!(
            "hold {} or task {} at version {} is no longer current",
            commit.reservation_id, commit.task_id, commit.task_version
        )));
    };
    let incident_id: String = assigned.get("incident_id")?;

    // Drop-off beds. A shelter that filled up meanwhile does not stop the
    // pickup (owner decision): the dispatcher resolves the drop-off.
    let mut dropoff_shelter = None;
    if let Some((shelter_id, beds)) = &commit.dropoff {
        let reserved = txn_first_row(
            txn,
            reserve_beds_query(shelter_id, *beds, &commit.task_id, &incident_id, &new_id()),
        )
        .await?;
        if reserved.is_some() {
            dropoff_shelter = Some(shelter_id.clone());
        }
    }

    let payload = json!({
        "task_id": commit.task_id,
        "incident_id": incident_id,
        "vehicle_id": commit.vehicle_id,
        "reservation_id": commit.reservation_id,
        "eta_s": commit.eta_s,
        "overlay_version": commit.overlay_version,
        "dropoff_shelter_id": dropoff_shelter,
    })
    .to_string();
    txn_first_row(
        txn,
        query(
            "CREATE (:OutboxEvent { \
                 event_id: $outbox_id, aggregate_id: $task_id, type: 'TASK_ASSIGNED', \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             FOREACH (_ IN CASE WHEN $unresolved THEN [1] ELSE [] END | \
                 CREATE (:OutboxEvent { \
                     event_id: $alert_id, aggregate_id: $task_id, type: 'DROPOFF_UNRESOLVED', \
                     payload_json: $payload, created_at: datetime(), published: false, attempts: 0 })) \
             RETURN $task_id AS task_id",
        )
        .param("outbox_id", new_id())
        .param("alert_id", new_id())
        .param("task_id", commit.task_id.clone())
        .param("payload", payload)
        .param("unresolved", dropoff_shelter.is_none()),
    )
    .await?;
    Ok(AssignmentOutcome { dropoff_shelter })
}

fn queued_task_from_row(row: &Row) -> Result<QueuedTask, OpsRepoError> {
    let queued_at: DateTime<Utc> = parse_time(&row.get::<String>("queued_at")?)?;
    let asset: AssetType = parse_asset(&row.get::<String>("required_asset")?)?;
    Ok(QueuedTask {
        task_id: row.get("task_id")?,
        incident_id: row.get("incident_id")?,
        version: row.get::<i64>("version")? as u64,
        required_asset: asset,
        priority_rank: row.get::<i64>("priority_rank")?.clamp(0, 2) as u8,
        queued_at,
        headcount: row.get::<Option<i64>>("headcount")?.map(|h| h.max(0) as u32),
        junction_id: row.get("junction_id")?,
        verify_failures: row.get::<i64>("verify_failures")?.max(0) as u32,
        escalations: row.get("escalations")?,
    })
}
