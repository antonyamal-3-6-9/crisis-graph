//! Dispatcher commands: review of a `REVIEW_REQUIRED` incident (approve or
//! reject) and task completion (T5), pulled into M7; merge and split of
//! duplicates (T4, rule D-4) and vehicle status, M6a.
//!
//! Each command is one transaction: role check (by the caller), version
//! check, the change, audit with the actor, outbox events, and a `:Command`
//! record keyed by `request_id`. A repeated `request_id` returns the stored
//! outcome and changes nothing.

use neo4rs::{query, Txn};
use serde_json::{json, Value};

use super::{new_id, txn_first_row, OpsRepoError, OpsRepository, MAX_WRITE_ATTEMPTS};
use crate::models::{
    active_task_key, merge_plan, ActiveTask, ActorContext, AssetType, MergePlan, Permission, ResourceKind, Urgency,
};

const MAX_REASON_CHARS: usize = 500;
pub const MAX_HEADCOUNT: u32 = 500;

/// What a dispatcher decided about a review incident.
#[derive(Debug, Clone)]
pub enum ReviewVerdict {
    /// Dispatch with these facts. `urgency: None` keeps the incident's.
    Approve {
        required_asset: AssetType,
        headcount: u32,
        junction_id: String,
        urgency: Option<Urgency>,
    },
    Reject,
}

#[derive(Debug, Clone)]
pub struct ReviewCommand {
    pub request_id: String,
    pub dataset: String,
    pub incident_id: String,
    pub expected_version: u64,
    pub decision: ReviewVerdict,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct CompleteTaskCommand {
    pub request_id: String,
    pub task_id: String,
    pub expected_version: u64,
    pub note: Option<String>,
}

/// Merge `absorbed_id` into `survivor_id` (T4, rule D-4).
#[derive(Debug, Clone)]
pub struct MergeCommand {
    pub request_id: String,
    pub survivor_id: String,
    pub survivor_version: u64,
    pub absorbed_id: String,
    pub absorbed_version: u64,
    pub reason: String,
}

/// Split a wrongly attached report off an incident into its own incident.
#[derive(Debug, Clone)]
pub struct SplitCommand {
    pub request_id: String,
    pub incident_id: String,
    pub expected_version: u64,
    pub report_id: String,
    pub reason: String,
}

/// Take a vehicle out of service (also while on a task: a breakdown) or
/// return it to service.
#[derive(Debug, Clone)]
pub struct VehicleStatusCommand {
    pub request_id: String,
    pub vehicle_id: String,
    pub expected_version: u64,
    pub out_of_service: bool,
    pub reason: String,
}

/// A command's result: the outcome, and whether it was replayed.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandOutcome {
    pub outcome: Value,
    pub replayed: bool,
}

impl OpsRepository {
    /// Approve or reject a `REVIEW_REQUIRED` incident. Needs `ReviewIncident`.
    /// Approve creates the QUEUED `ROAD_PICKUP` task the matcher picks up.
    pub async fn review_incident(&self, actor: &ActorContext, command: &ReviewCommand) -> Result<CommandOutcome, OpsRepoError> {
        require(actor, &[Permission::ReviewIncident])?;
        validate_review(command)?;
        self.run_command(&command.request_id, Body::Review(actor, command)).await
    }

    /// Mark an assigned task complete (T5): the vehicle is free again at the
    /// drop-off. Needs `CloseIncident` or `UpdateResponderStatus`.
    pub async fn complete_task(&self, actor: &ActorContext, command: &CompleteTaskCommand) -> Result<CommandOutcome, OpsRepoError> {
        require(actor, &[Permission::CloseIncident, Permission::UpdateResponderStatus])?;
        check_text(&command.request_id, "request_id", 128)?;
        if let Some(note) = &command.note {
            check_text(note, "note", MAX_REASON_CHARS)?;
        }
        self.run_command(&command.request_id, Body::Complete(actor, command)).await
    }

    /// Merge a possible duplicate into another incident (T4). Needs
    /// `ReviewIncident`. Exactly one vehicle stays sent (`merge_plan`).
    pub async fn merge_incidents(&self, actor: &ActorContext, command: &MergeCommand) -> Result<CommandOutcome, OpsRepoError> {
        require(actor, &[Permission::ReviewIncident])?;
        check_text(&command.request_id, "request_id", 128)?;
        check_text(&command.reason, "reason", MAX_REASON_CHARS)?;
        if command.survivor_id == command.absorbed_id {
            return Err(OpsRepoError::InvalidData("an incident cannot be merged into itself".into()));
        }
        self.run_command(&command.request_id, Body::Merge(actor, command)).await
    }

    /// Split a follow-up or merged report into its own `REVIEW_REQUIRED`
    /// incident. Needs `ReviewIncident`.
    pub async fn split_report(&self, actor: &ActorContext, command: &SplitCommand) -> Result<CommandOutcome, OpsRepoError> {
        require(actor, &[Permission::ReviewIncident])?;
        check_text(&command.request_id, "request_id", 128)?;
        check_text(&command.reason, "reason", MAX_REASON_CHARS)?;
        self.run_command(&command.request_id, Body::Split(actor, command)).await
    }

    /// Change a vehicle's service status. Needs `AssignDispatch` or
    /// `UpdateResponderStatus`.
    pub async fn set_vehicle_status(&self, actor: &ActorContext, command: &VehicleStatusCommand) -> Result<CommandOutcome, OpsRepoError> {
        require(actor, &[Permission::AssignDispatch, Permission::UpdateResponderStatus])?;
        check_text(&command.request_id, "request_id", 128)?;
        check_text(&command.reason, "reason", MAX_REASON_CHARS)?;
        self.run_command(&command.request_id, Body::VehicleStatus(actor, command)).await
    }

    /// Run one command transaction with replay and transient retry.
    async fn run_command(&self, request_id: &str, body: Body<'_>) -> Result<CommandOutcome, OpsRepoError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut txn = self.neo4j.graph.start_txn().await?;
            let result = async {
                if let Some(previous) = stored_outcome(&mut txn, request_id).await? {
                    return Ok(CommandOutcome { outcome: previous, replayed: true });
                }
                let outcome = match body {
                    Body::Review(actor, command) => review_in(&mut txn, actor, command).await?,
                    Body::Complete(actor, command) => complete_in(&mut txn, actor, command).await?,
                    Body::Merge(actor, command) => merge_in(&mut txn, actor, command).await?,
                    Body::Split(actor, command) => split_in(&mut txn, actor, command).await?,
                    Body::VehicleStatus(actor, command) => vehicle_status_in(&mut txn, actor, command).await?,
                };
                txn_first_row(
                    &mut txn,
                    query(
                        "CREATE (:Command {request_id: $request_id, outcome_json: $outcome, created_at: datetime()}) \
                         RETURN $request_id AS request_id",
                    )
                    .param("request_id", request_id)
                    .param("outcome", outcome.to_string()),
                )
                .await?;
                Ok(CommandOutcome { outcome, replayed: false })
            }
            .await;
            let result = match result {
                Ok(outcome) if !outcome.replayed => txn.commit().await.map(|_| outcome).map_err(OpsRepoError::from),
                other => {
                    let _ = txn.rollback().await;
                    other
                }
            };
            match result {
                Err(OpsRepoError::Transient(_)) if attempt < MAX_WRITE_ATTEMPTS => {
                    tokio::time::sleep(super::jittered_backoff(attempt)).await;
                }
                // A concurrent command with the same request_id committed first.
                Err(OpsRepoError::Conflict(message)) => {
                    let mut txn = self.neo4j.graph.start_txn().await?;
                    let previous = stored_outcome(&mut txn, request_id).await;
                    let _ = txn.rollback().await;
                    return match previous? {
                        Some(outcome) => Ok(CommandOutcome { outcome, replayed: true }),
                        None => Err(OpsRepoError::Conflict(message)),
                    };
                }
                other => return other,
            }
        }
    }
}

/// The command a transaction runs.
#[derive(Clone, Copy)]
enum Body<'a> {
    Review(&'a ActorContext, &'a ReviewCommand),
    Complete(&'a ActorContext, &'a CompleteTaskCommand),
    Merge(&'a ActorContext, &'a MergeCommand),
    Split(&'a ActorContext, &'a SplitCommand),
    VehicleStatus(&'a ActorContext, &'a VehicleStatusCommand),
}

async fn stored_outcome(txn: &mut Txn, request_id: &str) -> Result<Option<Value>, OpsRepoError> {
    let row = txn_first_row(
        txn,
        query("MATCH (c:Command {request_id: $request_id}) RETURN c.outcome_json AS outcome").param("request_id", request_id),
    )
    .await?;
    match row {
        Some(row) => {
            let text: String = row.get("outcome")?;
            Ok(Some(serde_json::from_str(&text).map_err(|e| OpsRepoError::InvalidData(e.to_string()))?))
        }
        None => Ok(None),
    }
}

async fn review_in(txn: &mut Txn, actor: &ActorContext, command: &ReviewCommand) -> Result<Value, OpsRepoError> {
    let row = match &command.decision {
        ReviewVerdict::Approve { required_asset, headcount, junction_id, urgency } => {
            let task_id = new_id();
            txn_first_row(
                txn,
                query(
                    "MATCH (i:OpsIncident {incident_id: $id}) \
                     SET i._lock = true \
                     REMOVE i._lock \
                     WITH i \
                     WHERE i.status = 'REVIEW_REQUIRED' AND i.version = $version \
                       AND NOT EXISTS { MATCH (i)-[:HAS_TASK]->(:Task) } \
                     MATCH (j:Junction {id: $junction, dataset: $dataset}) \
                     OPTIONAL MATCH (i)-[old:LOCATED_AT]->() \
                     DELETE old \
                     WITH DISTINCT i, j, coalesce($urgency, i.urgency, 'UNKNOWN') AS urgency \
                     WITH i, j, urgency, CASE urgency WHEN 'CRITICAL' THEN 2 WHEN 'STABLE' THEN 0 ELSE 1 END AS rank \
                     SET i.status = 'TRIAGE_APPROVED', i.version = i.version + 1, i.updated_at = datetime(), \
                         i.required_asset = $asset, i.headcount = $headcount, i.junction_id = j.id, \
                         i.location = point({latitude: j.lat, longitude: j.lon}), i.location_source = 'DISPATCHER', \
                         i.urgency = urgency, i.priority_rank = rank, i.callback_required = (urgency = 'UNKNOWN'), \
                         i.reviewed_by = $actor, i.reviewed_at = datetime() \
                     CREATE (i)-[:LOCATED_AT]->(j) \
                     CREATE (i)-[:HAS_TASK {seq: 1}]->(:Task { \
                         task_id: $task_id, incident_id: i.incident_id, kind: 'ROAD_PICKUP', status: 'QUEUED', \
                         required_asset: $asset, urgency: urgency, priority_rank: rank, version: 0, \
                         created_at: datetime(), queued_at: datetime() }) \
                     CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                         event_id: $audit_id, aggregate_type: 'Incident', aggregate_id: i.incident_id, \
                         action: 'INCIDENT_REVIEW_APPROVED', actor_id: $actor, from_state: 'REVIEW_REQUIRED', \
                         to_state: 'TRIAGE_APPROVED', previous_version: $version, new_version: i.version, \
                         reason: $reason, occurred_at: datetime(), details_json: $details }) \
                     CREATE (:OutboxEvent { \
                         event_id: $outbox_id, aggregate_id: i.incident_id, type: 'INCIDENT_READY', \
                         payload_json: '{\"incident_id\":\"' + i.incident_id + '\",\"task_id\":\"' + $task_id + \
                             '\",\"status\":\"TRIAGE_APPROVED\",\"urgency\":\"' + urgency + '\",\"source\":\"DISPATCHER\"}', \
                         created_at: datetime(), published: false, attempts: 0 }) \
                     RETURN i.version AS version, urgency, $task_id AS task_id",
                )
                .param("id", command.incident_id.clone())
                .param("version", command.expected_version as i64)
                .param("dataset", command.dataset.clone())
                .param("junction", junction_id.clone())
                .param("urgency", urgency.map(|u| u.as_str()))
                .param("asset", required_asset.to_string())
                .param("headcount", *headcount as i64)
                .param("task_id", task_id)
                .param("actor", actor.actor_id.clone())
                .param("reason", command.reason.trim())
                .param(
                    "details",
                    json!({ "required_asset": required_asset.to_string(), "headcount": headcount,
                            "junction_id": junction_id, "urgency_override": urgency.map(|u| u.as_str()) })
                    .to_string(),
                )
                .param("audit_id", new_id())
                .param("outbox_id", new_id()),
            )
            .await?
        }
        ReviewVerdict::Reject => {
            txn_first_row(
                txn,
                query(
                    "MATCH (i:OpsIncident {incident_id: $id}) \
                     SET i._lock = true \
                     REMOVE i._lock \
                     WITH i \
                     WHERE i.status = 'REVIEW_REQUIRED' AND i.version = $version \
                     SET i.status = 'REJECTED', i.version = i.version + 1, i.updated_at = datetime(), \
                         i.callback_required = false, i.reviewed_by = $actor, i.reviewed_at = datetime(), \
                         i.reject_reason = $reason \
                     CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                         event_id: $audit_id, aggregate_type: 'Incident', aggregate_id: i.incident_id, \
                         action: 'INCIDENT_REJECTED', actor_id: $actor, from_state: 'REVIEW_REQUIRED', \
                         to_state: 'REJECTED', previous_version: $version, new_version: i.version, \
                         reason: $reason, occurred_at: datetime(), details_json: '{}' }) \
                     CREATE (:OutboxEvent { \
                         event_id: $outbox_id, aggregate_id: i.incident_id, type: 'INCIDENT_REJECTED', \
                         payload_json: '{\"incident_id\":\"' + i.incident_id + '\",\"status\":\"REJECTED\"}', \
                         created_at: datetime(), published: false, attempts: 0 }) \
                     RETURN i.version AS version",
                )
                .param("id", command.incident_id.clone())
                .param("version", command.expected_version as i64)
                .param("actor", actor.actor_id.clone())
                .param("reason", command.reason.trim())
                .param("audit_id", new_id())
                .param("outbox_id", new_id()),
            )
            .await?
        }
    };

    let Some(row) = row else {
        return Err(review_refusal(txn, command).await);
    };
    let version = row.get::<i64>("version")?;
    Ok(match command.decision {
        ReviewVerdict::Approve { .. } => json!({
            "incident_id": command.incident_id, "status": "TRIAGE_APPROVED", "version": version,
            "task_id": row.get::<String>("task_id")?, "urgency": row.get::<String>("urgency")?,
        }),
        ReviewVerdict::Reject => json!({ "incident_id": command.incident_id, "status": "REJECTED", "version": version }),
    })
}

/// Why a review changed nothing: unknown incident, unknown junction, or the
/// incident is no longer the version or state the dispatcher saw.
async fn review_refusal(txn: &mut Txn, command: &ReviewCommand) -> OpsRepoError {
    let found = txn_first_row(
        txn,
        query(
            "OPTIONAL MATCH (i:OpsIncident {incident_id: $id}) \
             OPTIONAL MATCH (j:Junction {id: $junction, dataset: $dataset}) \
             RETURN i.status AS status, i.version AS version, j IS NOT NULL AS junction",
        )
        .param("id", command.incident_id.clone())
        .param("junction", match &command.decision {
            ReviewVerdict::Approve { junction_id, .. } => junction_id.clone(),
            ReviewVerdict::Reject => String::new(),
        })
        .param("dataset", command.dataset.clone()),
    )
    .await;
    let Ok(Some(row)) = found else {
        return OpsRepoError::Database("review check failed".into());
    };
    let status: Option<String> = row.get("status").unwrap_or(None);
    let Some(status) = status else {
        return OpsRepoError::NotFound(format!("incident {}", command.incident_id));
    };
    if matches!(command.decision, ReviewVerdict::Approve { .. }) && !row.get::<bool>("junction").unwrap_or(false) {
        return OpsRepoError::InvalidData("location junction not found in the dataset".into());
    }
    let version: Option<i64> = row.get("version").unwrap_or(None);
    OpsRepoError::NotAvailable(format!(
        "incident {} is {status} at version {}, not REVIEW_REQUIRED at version {}",
        command.incident_id,
        version.unwrap_or(-1),
        command.expected_version
    ))
}

async fn complete_in(txn: &mut Txn, actor: &ActorContext, command: &CompleteTaskCommand) -> Result<Value, OpsRepoError> {
    let row = txn_first_row(
        txn,
        query(
            "MATCH (t:Task {task_id: $task_id}) \
             SET t._lock = true \
             REMOVE t._lock \
             WITH t \
             WHERE t.status = 'ASSIGNED' AND t.version = $version \
             MATCH (i:OpsIncident {incident_id: t.incident_id}) \
             MATCH (r:Reservation {active_task_key: $vehicle_key})-[:OF_VEHICLE]->(v:Vehicle) \
             OPTIONAL MATCH (:Reservation {task_id: t.task_id, resource_kind: 'BEDS', status: 'COMMITTED'})-[:AT_SHELTER]->(s:Shelter) \
             OPTIONAL MATCH (i)-[:LOCATED_AT]->(ij:Junction) \
             WITH t, i, r, v, head(collect(s)) AS s, head(collect(ij)) AS ij \
             WITH t, i, r, v, s, ij, coalesce(s.junction_id, i.junction_id, v.junction_id) AS end_id \
             OPTIONAL MATCH (ej:Junction {id: end_id, dataset: ij.dataset}) \
             SET r.status = 'RELEASED', r.released_at = datetime(), r.release_reason = 'task completed' \
             REMOVE r.active_vehicle_key, r.active_task_key \
             SET v.status = 'AVAILABLE', v.version = v.version + 1, v.updated_at = datetime(), \
                 v.junction_id = end_id, \
                 v.position = CASE WHEN ej IS NULL THEN v.position ELSE point({latitude: ej.lat, longitude: ej.lon}) END, \
                 t.status = 'COMPLETED', t.version = t.version + 1, t.updated_at = datetime(), \
                 t.completed_at = datetime(), t.completed_by = $actor \
             WITH t, i, r, v, s, end_id \
             OPTIONAL MATCH (i)-[:HAS_TASK]->(other:Task) \
             WHERE other.task_id <> t.task_id AND other.status IN ['QUEUED', 'ASSIGNED', 'PENDING_TRIGGER'] \
             WITH t, i, r, v, s, end_id, count(other) AS still_open \
             SET i.version = i.version + 1, i.updated_at = datetime(), \
                 i.status = CASE WHEN still_open = 0 THEN 'COMPLETED' ELSE i.status END \
             CREATE (t)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $task_audit_id, aggregate_type: 'Task', aggregate_id: t.task_id, \
                 action: 'TASK_COMPLETED', actor_id: $actor, from_state: 'ASSIGNED', to_state: 'COMPLETED', \
                 previous_version: $version, new_version: t.version, reason: $note, \
                 occurred_at: datetime(), details_json: '{}' }) \
             CREATE (v)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $vehicle_audit_id, aggregate_type: 'Vehicle', aggregate_id: v.vehicle_id, \
                 action: 'VEHICLE_AVAILABLE', actor_id: $actor, from_state: 'ASSIGNED', to_state: 'AVAILABLE', \
                 previous_version: v.version - 1, new_version: v.version, \
                 reason: 'task ' + t.task_id + ' completed', occurred_at: datetime(), details_json: '{}' }) \
             RETURN t.version AS version, t.incident_id AS incident_id, i.status AS incident_status, \
                    v.vehicle_id AS vehicle_id, end_id AS position_junction, s.id AS dropoff_shelter_id",
        )
        .param("task_id", command.task_id.clone())
        .param("version", command.expected_version as i64)
        .param("vehicle_key", active_task_key(&command.task_id, ResourceKind::Vehicle))
        .param("actor", actor.actor_id.clone())
        .param("note", command.note.clone().unwrap_or_else(|| "task completed".into()))
        .param("task_audit_id", new_id())
        .param("vehicle_audit_id", new_id()),
    )
    .await?;
    let Some(row) = row else {
        return Err(complete_refusal(txn, command).await);
    };
    let outcome = json!({
        "task_id": command.task_id,
        "status": "COMPLETED",
        "version": row.get::<i64>("version")?,
        "incident_id": row.get::<String>("incident_id")?,
        "incident_status": row.get::<String>("incident_status")?,
        "vehicle_id": row.get::<String>("vehicle_id")?,
        "vehicle_position_junction": row.get::<Option<String>>("position_junction")?,
        "dropoff_shelter_id": row.get::<Option<String>>("dropoff_shelter_id")?,
    });
    txn_first_row(
        txn,
        query(
            "CREATE (:OutboxEvent { event_id: $task_event, aggregate_id: $task_id, type: 'TASK_COMPLETED', \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             CREATE (:OutboxEvent { event_id: $vehicle_event, aggregate_id: $vehicle_id, type: 'VEHICLE_AVAILABLE', \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             RETURN $task_id AS task_id",
        )
        .param("task_event", new_id())
        .param("vehicle_event", new_id())
        .param("task_id", command.task_id.clone())
        .param("vehicle_id", outcome["vehicle_id"].as_str().unwrap_or_default().to_string())
        .param("payload", outcome.to_string()),
    )
    .await?;
    Ok(outcome)
}

async fn complete_refusal(txn: &mut Txn, command: &CompleteTaskCommand) -> OpsRepoError {
    let found = txn_first_row(
        txn,
        query("OPTIONAL MATCH (t:Task {task_id: $id}) RETURN t.status AS status, t.version AS version")
            .param("id", command.task_id.clone()),
    )
    .await;
    match found {
        Ok(Some(row)) => match row.get::<Option<String>>("status").unwrap_or(None) {
            None => OpsRepoError::NotFound(format!("task {}", command.task_id)),
            Some(status) => OpsRepoError::NotAvailable(format!(
                "task {} is {status} at version {}, not ASSIGNED at version {}",
                command.task_id,
                row.get::<Option<i64>>("version").unwrap_or(None).unwrap_or(-1),
                command.expected_version
            )),
        },
        _ => OpsRepoError::Database("completion check failed".into()),
    }
}

// ------------------------------------------------------------------ merge (T4)

/// An incident as the merge sees it, after it is locked.
struct MergeSide {
    status: String,
    version: u64,
    tasks: Vec<(String, String)>,
}

impl MergeSide {
    fn active(&self) -> ActiveTask {
        if self.tasks.iter().any(|(_, s)| s == "ASSIGNED") {
            ActiveTask::Assigned
        } else if self.tasks.iter().any(|(_, s)| s == "QUEUED" || s == "PENDING_TRIGGER") {
            ActiveTask::Queued
        } else {
            ActiveTask::None
        }
    }

    fn task_ids(&self) -> Vec<String> {
        self.tasks.iter().map(|(id, _)| id.clone()).collect()
    }
}

async fn lock_incident(txn: &mut Txn, incident_id: &str) -> Result<Option<MergeSide>, OpsRepoError> {
    let row = txn_first_row(
        txn,
        query(
            "MATCH (i:OpsIncident {incident_id: $id}) \
             SET i._lock = true \
             REMOVE i._lock \
             WITH i \
             OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) WHERE t.status IN ['QUEUED', 'ASSIGNED', 'PENDING_TRIGGER'] \
             RETURN i.status AS status, i.version AS version, \
                    [x IN collect(t) | [x.task_id, x.status]] AS tasks",
        )
        .param("id", incident_id),
    )
    .await?;
    let Some(row) = row else { return Ok(None) };
    let tasks: Vec<Vec<String>> = row.get("tasks")?;
    Ok(Some(MergeSide {
        status: row.get("status")?,
        version: row.get::<i64>("version")? as u64,
        tasks: tasks.into_iter().filter_map(|t| Some((t.first()?.clone(), t.get(1)?.clone()))).collect(),
    }))
}

async fn merge_in(txn: &mut Txn, actor: &ActorContext, command: &MergeCommand) -> Result<Value, OpsRepoError> {
    // Lock both in a fixed order (rule A-8), so two merges never deadlock.
    let mut order = [command.survivor_id.as_str(), command.absorbed_id.as_str()];
    order.sort();
    let mut sides = Vec::new();
    for id in order {
        let side = lock_incident(txn, id).await?.ok_or_else(|| OpsRepoError::NotFound(format!("incident {id}")))?;
        sides.push((id, side));
    }
    let take = |id: &str| sides.iter().position(|(i, _)| *i == id).expect("locked");
    let survivor = sides.remove(take(&command.survivor_id)).1;
    let absorbed = sides.pop().expect("two incidents").1;

    for (side, version, name) in [(&survivor, command.survivor_version, "survivor"), (&absorbed, command.absorbed_version, "absorbed")] {
        if side.version != version {
            return Err(OpsRepoError::NotAvailable(format!("{name} incident changed (version {} not {version})", side.version)));
        }
        if matches!(side.status.as_str(), "MERGED" | "REJECTED" | "COMPLETED") {
            return Err(OpsRepoError::InvalidData(format!("the {name} incident is {} and cannot be merged", side.status)));
        }
    }
    if survivor.status == "REVIEW_REQUIRED" && absorbed.status != "REVIEW_REQUIRED" {
        return Err(OpsRepoError::InvalidData(
            "merge into the approved incident: the survivor is still under review".into(),
        ));
    }

    let plan = merge_plan(survivor.active(), absorbed.active());
    let reason = format!("merged ({}): {}", command.absorbed_id, command.reason.trim());
    let (cancel, moved) = match plan {
        MergePlan::KeepSurvivor => (absorbed.task_ids(), Vec::new()),
        MergePlan::MoveAbsorbed => (
            survivor.tasks.iter().filter(|(_, s)| s != "ASSIGNED").map(|(id, _)| id.clone()).collect(),
            absorbed.task_ids(),
        ),
    };
    let mut released_vehicles = Vec::new();
    for task_id in &cancel {
        released_vehicles.extend(cancel_task(txn, actor, task_id, &reason).await?);
    }
    for task_id in &moved {
        txn_first_row(
            txn,
            query(
                "MATCH (a:OpsIncident {incident_id: $absorbed})-[rel:HAS_TASK]->(t:Task {task_id: $task_id}), \
                       (s:OpsIncident {incident_id: $survivor}) \
                 DELETE rel \
                 CREATE (s)-[:HAS_TASK {seq: 1, moved_from: $absorbed}]->(t) \
                 SET t.incident_id = $survivor, t.version = t.version + 1, t.updated_at = datetime() \
                 WITH t \
                 OPTIONAL MATCH (r:Reservation {task_id: t.task_id}) \
                 SET r.incident_id = $survivor \
                 RETURN count(r) AS moved",
            )
            .param("absorbed", command.absorbed_id.clone())
            .param("survivor", command.survivor_id.clone())
            .param("task_id", task_id.clone()),
        )
        .await?;
    }

    let row = txn_first_row(
        txn,
        query(
            "MATCH (s:OpsIncident {incident_id: $survivor}), (a:OpsIncident {incident_id: $absorbed}) \
             WITH s, a, coalesce(a.priority_rank, 1) AS a_rank, coalesce(s.priority_rank, 1) AS s_rank \
             WITH s, a, a_rank > s_rank \
                  OR (a_rank = s_rank AND coalesce(s.urgency, 'UNKNOWN') = 'UNKNOWN' AND a.urgency = 'URGENT') AS raise \
             SET s.headcount = CASE WHEN coalesce(a.headcount, 0) > coalesce(s.headcount, 0) THEN a.headcount ELSE s.headcount END, \
                 s.urgency = CASE WHEN raise THEN a.urgency ELSE s.urgency END, \
                 s.rule_urgency = CASE WHEN raise THEN a.rule_urgency ELSE s.rule_urgency END, \
                 s.priority_rank = CASE WHEN raise THEN a.priority_rank ELSE s.priority_rank END, \
                 s.report_count = coalesce(s.report_count, 1) + coalesce(a.report_count, 1), \
                 s.version = s.version + 1, s.updated_at = datetime(), \
                 s.status = CASE WHEN EXISTS { MATCH (s)-[:HAS_TASK]->(:Task {status: 'ASSIGNED'}) } \
                     THEN 'ASSIGNED' ELSE s.status END, \
                 a.status = 'MERGED', a.merged_into = s.incident_id, a.callback_required = false, \
                 a.version = a.version + 1, a.updated_at = datetime() \
             SET s.callback_required = coalesce(s.urgency, 'UNKNOWN') = 'UNKNOWN' AND s.status <> 'REVIEW_REQUIRED' \
                 OR (s.status = 'REVIEW_REQUIRED' AND coalesce(s.callback_required, false)) \
             CREATE (a)-[:MERGED_INTO {merged_at: datetime(), actor_id: $actor, reason: $reason}]->(s) \
             WITH s, a, raise \
             OPTIONAL MATCH (s)-[:HAS_TASK]->(t:Task) WHERE t.status IN ['QUEUED', 'PENDING_TRIGGER'] \
             FOREACH (x IN CASE WHEN t IS NULL THEN [] ELSE [t] END | \
                 SET x.urgency = s.urgency, x.priority_rank = s.priority_rank) \
             WITH DISTINCT s, a, raise \
             OPTIONAL MATCH (r:Report)-[:REPORTS]->(a) \
             FOREACH (x IN CASE WHEN r IS NULL THEN [] ELSE [r] END | \
                 MERGE (x)-[link:REPORTS {kind: 'MERGED'}]->(s) \
                 ON CREATE SET link.rule = 'D-4', link.attached_at = datetime(), link.merged_from = a.incident_id) \
             WITH DISTINCT s, a, raise \
             OPTIONAL MATCH (s)-[d:POSSIBLE_DUPLICATE]-(a) \
             SET d.resolution = 'MERGED', d.resolved_at = datetime() \
             WITH DISTINCT s, a, raise \
             CREATE (s)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $survivor_audit, aggregate_type: 'Incident', aggregate_id: s.incident_id, \
                 action: 'INCIDENT_MERGED_IN', actor_id: $actor, previous_version: $survivor_version, \
                 new_version: s.version, reason: $reason, occurred_at: datetime(), details_json: $details }) \
             CREATE (a)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $absorbed_audit, aggregate_type: 'Incident', aggregate_id: a.incident_id, \
                 action: 'INCIDENT_MERGED', actor_id: $actor, from_state: $absorbed_status, to_state: 'MERGED', \
                 previous_version: $absorbed_version, new_version: a.version, reason: $reason, \
                 occurred_at: datetime(), details_json: $details }) \
             RETURN s.version AS survivor_version, a.version AS absorbed_version, s.headcount AS headcount, \
                    s.urgency AS urgency, raise",
        )
        .param("survivor", command.survivor_id.clone())
        .param("absorbed", command.absorbed_id.clone())
        .param("survivor_version", command.survivor_version as i64)
        .param("absorbed_version", command.absorbed_version as i64)
        .param("absorbed_status", absorbed.status.clone())
        .param("actor", actor.actor_id.clone())
        .param("reason", command.reason.trim())
        .param(
            "details",
            json!({ "plan": format!("{plan:?}"), "cancelled_tasks": cancel, "moved_tasks": moved,
                    "released_vehicles": released_vehicles })
            .to_string(),
        )
        .param("survivor_audit", new_id())
        .param("absorbed_audit", new_id()),
    )
    .await?
    .ok_or_else(|| OpsRepoError::Database("merge update returned no row".into()))?;

    let outcome = json!({
        "survivor_id": command.survivor_id,
        "absorbed_id": command.absorbed_id,
        "plan": match plan { MergePlan::KeepSurvivor => "KEEP_SURVIVOR", MergePlan::MoveAbsorbed => "MOVE_ABSORBED" },
        "survivor_version": row.get::<i64>("survivor_version")?,
        "absorbed_version": row.get::<i64>("absorbed_version")?,
        "headcount": row.get::<Option<i64>>("headcount")?,
        "urgency": row.get::<Option<String>>("urgency")?,
        "urgency_raised": row.get::<bool>("raise")?,
        "cancelled_tasks": cancel,
        "moved_tasks": moved,
        "released_vehicles": released_vehicles,
    });
    let mut events = vec![
        ("INCIDENT_MERGED", command.absorbed_id.clone()),
        ("INCIDENT_UPDATED", command.survivor_id.clone()),
    ];
    events.extend(cancel.iter().map(|t| ("TASK_CANCELLED", t.clone())));
    events.extend(released_vehicles.iter().map(|v| ("VEHICLE_AVAILABLE", v.clone())));
    write_events(txn, &events, &outcome).await?;
    Ok(outcome)
}

/// Cancel a task: any active reservation is released (vehicle available,
/// beds returned). Returns the vehicles freed.
async fn cancel_task(txn: &mut Txn, actor: &ActorContext, task_id: &str, reason: &str) -> Result<Vec<String>, OpsRepoError> {
    let row = txn_first_row(
        txn,
        query(
            "MATCH (t:Task {task_id: $task_id}) \
             WHERE t.status IN ['QUEUED', 'ASSIGNED', 'PENDING_TRIGGER'] \
             SET t._lock = true \
             REMOVE t._lock \
             WITH t, t.status AS from_state \
             SET t.status = 'CANCELLED', t.version = t.version + 1, t.updated_at = datetime(), \
                 t.cancelled_at = datetime(), t.cancel_reason = $reason \
             CREATE (t)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Task', aggregate_id: t.task_id, action: 'TASK_CANCELLED', \
                 actor_id: $actor, from_state: from_state, to_state: 'CANCELLED', new_version: t.version, \
                 reason: $reason, occurred_at: datetime(), details_json: '{}' }) \
             WITH t \
             OPTIONAL MATCH (r:Reservation {task_id: t.task_id}) \
             WHERE r.status IN ['HELD', 'COMMITTED'] AND r.active_task_key IS NOT NULL \
             OPTIONAL MATCH (r)-[:OF_VEHICLE]->(v:Vehicle) \
             OPTIONAL MATCH (r)-[:AT_SHELTER]->(s:Shelter) \
             FOREACH (_ IN CASE WHEN r IS NULL THEN [] ELSE [1] END | \
                 SET r.status = 'RELEASED', r.released_at = datetime(), r.release_reason = $reason \
                 REMOVE r.active_vehicle_key, r.active_task_key) \
             FOREACH (_ IN CASE WHEN v IS NULL THEN [] ELSE [1] END | \
                 SET v.status = 'AVAILABLE', v.version = v.version + 1, v.updated_at = datetime()) \
             FOREACH (_ IN CASE WHEN s IS NULL THEN [] ELSE [1] END | \
                 SET s.current_occupancy = CASE WHEN s.current_occupancy >= r.beds \
                         THEN s.current_occupancy - r.beds ELSE 0 END, \
                     s.version = coalesce(s.version, 0) + 1, s.last_updated = datetime()) \
             RETURN [x IN collect(DISTINCT v.vehicle_id) WHERE x IS NOT NULL] AS vehicles",
        )
        .param("task_id", task_id)
        .param("reason", reason)
        .param("actor", actor.actor_id.clone())
        .param("audit_id", new_id()),
    )
    .await?;
    Ok(match row {
        Some(row) => row.get("vehicles")?,
        None => Vec::new(),
    })
}

/// One outbox event per `(type, aggregate_id)`, all with the same payload.
async fn write_events(txn: &mut Txn, events: &[(&str, String)], payload: &Value) -> Result<(), OpsRepoError> {
    let rows: Vec<neo4rs::BoltType> = events
        .iter()
        .map(|(kind, aggregate)| {
            neo4rs::BoltType::from(std::collections::HashMap::from([
                ("event_id".to_string(), neo4rs::BoltType::from(new_id())),
                ("type".to_string(), neo4rs::BoltType::from(kind.to_string())),
                ("aggregate_id".to_string(), neo4rs::BoltType::from(aggregate.clone())),
            ]))
        })
        .collect();
    txn_first_row(
        txn,
        query(
            "UNWIND $events AS e \
             CREATE (:OutboxEvent { event_id: e.event_id, aggregate_id: e.aggregate_id, type: e.type, \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             RETURN count(*) AS written",
        )
        .param("events", rows)
        .param("payload", payload.to_string()),
    )
    .await?;
    Ok(())
}

// ------------------------------------------------------------------- split

async fn split_in(txn: &mut Txn, actor: &ActorContext, command: &SplitCommand) -> Result<Value, OpsRepoError> {
    let new_incident = new_id();
    let reason = format!("Split from {}: {}", command.incident_id, command.reason.trim());
    let row = txn_first_row(
        txn,
        query(
            "MATCH (i:OpsIncident {incident_id: $id}) \
             SET i._lock = true \
             REMOVE i._lock \
             WITH i \
             WHERE i.version = $version AND NOT i.status IN ['MERGED', 'REJECTED'] \
             MATCH (r:Report {report_id: $report_id})-[link:REPORTS]->(i) \
             WHERE link.kind IN ['FOLLOW_UP', 'MERGED'] \
             OPTIONAL MATCH (j:Junction {id: r.resolved_junction_id}) \
             WITH i, r, link, head(collect(j)) AS j \
             CREATE (n:OpsIncident { \
                 incident_id: $new_id, status: 'REVIEW_REQUIRED', version: 0, headcount: r.headcount, \
                 required_asset: r.required_asset, suggested_asset: r.suggested_asset, \
                 location: r.location, location_source: r.location_source, caller_location: r.caller_location, \
                 junction_id: r.resolved_junction_id, device_id: r.device_id, \
                 review_reasons: [$reason] + coalesce(r.review_reasons, []), report_count: 1, \
                 urgency: coalesce(r.urgency, 'UNKNOWN'), rule_urgency: coalesce(r.rule_urgency, 'UNKNOWN'), \
                 review_priority: coalesce(r.review_priority, 'HIGH'), review_rank: coalesce(r.review_rank, 2), \
                 urgency_flags: coalesce(r.urgency_flags, []), urgency_rules_version: r.urgency_rules_version, \
                 priority_rank: coalesce(r.priority_rank, 1), \
                 callback_required: coalesce(r.urgency, 'UNKNOWN') = 'UNKNOWN', \
                 split_from: i.incident_id, created_at: datetime(), updated_at: datetime() }) \
             CREATE (r)-[:REPORTS {kind: 'PRIMARY', attached_at: datetime(), rule: 'SPLIT'}]->(n) \
             FOREACH (_ IN CASE WHEN j IS NULL THEN [] ELSE [1] END | CREATE (n)-[:LOCATED_AT]->(j)) \
             WITH i, n, r, link.kind AS old_kind \
             MATCH (r)-[old:REPORTS]->(i) \
             DELETE old \
             WITH DISTINCT i, n, r, old_kind \
             OPTIONAL MATCH (other:Report)-[:REPORTS]->(i) \
             WITH i, n, r, old_kind, max(other.headcount) AS remaining_headcount, count(DISTINCT other) AS remaining \
             SET i.headcount = coalesce(remaining_headcount, i.headcount), i.report_count = remaining, \
                 i.version = i.version + 1, i.updated_at = datetime() \
             CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_old, aggregate_type: 'Incident', aggregate_id: i.incident_id, \
                 action: 'INCIDENT_SPLIT', actor_id: $actor, previous_version: $version, new_version: i.version, \
                 reason: $reason, occurred_at: datetime(), \
                 details_json: '{\"report_id\":\"' + r.report_id + '\",\"new_incident_id\":\"' + n.incident_id + \
                     '\",\"link\":\"' + old_kind + '\"}' }) \
             CREATE (n)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_new, aggregate_type: 'Incident', aggregate_id: n.incident_id, \
                 action: 'INCIDENT_CREATED', actor_id: $actor, to_state: 'REVIEW_REQUIRED', new_version: 0, \
                 reason: $reason, occurred_at: datetime(), details_json: '{}' }) \
             RETURN i.version AS version, i.headcount AS headcount, remaining",
        )
        .param("id", command.incident_id.clone())
        .param("version", command.expected_version as i64)
        .param("report_id", command.report_id.clone())
        .param("new_id", new_incident.clone())
        .param("reason", reason)
        .param("actor", actor.actor_id.clone())
        .param("audit_old", new_id())
        .param("audit_new", new_id()),
    )
    .await?;
    let Some(row) = row else {
        return Err(split_refusal(txn, command).await);
    };
    let outcome = json!({
        "incident_id": command.incident_id,
        "version": row.get::<i64>("version")?,
        "headcount": row.get::<Option<i64>>("headcount")?,
        "remaining_reports": row.get::<i64>("remaining")?,
        "report_id": command.report_id,
        "new_incident_id": new_incident,
        "new_status": "REVIEW_REQUIRED",
    });
    write_events(
        txn,
        &[("INCIDENT_SPLIT", command.incident_id.clone()), ("INCIDENT_REVIEW_REQUIRED", new_incident)],
        &outcome,
    )
    .await?;
    Ok(outcome)
}

async fn split_refusal(txn: &mut Txn, command: &SplitCommand) -> OpsRepoError {
    let found = txn_first_row(
        txn,
        query(
            "OPTIONAL MATCH (i:OpsIncident {incident_id: $id}) \
             OPTIONAL MATCH (:Report {report_id: $report_id})-[link:REPORTS]->(i) \
             RETURN i.status AS status, i.version AS version, link.kind AS kind",
        )
        .param("id", command.incident_id.clone())
        .param("report_id", command.report_id.clone()),
    )
    .await;
    let Ok(Some(row)) = found else {
        return OpsRepoError::Database("split check failed".into());
    };
    let status: Option<String> = row.get("status").unwrap_or(None);
    let Some(status) = status else {
        return OpsRepoError::NotFound(format!("incident {}", command.incident_id));
    };
    let version: Option<i64> = row.get("version").unwrap_or(None);
    if version != Some(command.expected_version as i64) {
        return OpsRepoError::NotAvailable(format!(
            "incident {} is at version {}, not {}",
            command.incident_id,
            version.unwrap_or(-1),
            command.expected_version
        ));
    }
    match row.get::<Option<String>>("kind").unwrap_or(None) {
        None => OpsRepoError::InvalidData("the report is not linked to this incident".into()),
        Some(kind) if kind == "PRIMARY" => {
            OpsRepoError::InvalidData("the incident's first report cannot be split off; split the others instead".into())
        }
        Some(_) => OpsRepoError::InvalidData(format!("a {status} incident cannot be split")),
    }
}

// ----------------------------------------------------------- vehicle status

async fn vehicle_status_in(txn: &mut Txn, actor: &ActorContext, command: &VehicleStatusCommand) -> Result<Value, OpsRepoError> {
    let to = if command.out_of_service { "OUT_OF_SERVICE" } else { "AVAILABLE" };
    let row = txn_first_row(
        txn,
        query(
            "MATCH (v:Vehicle {vehicle_id: $id}) \
             SET v._lock = true \
             REMOVE v._lock \
             WITH v, v.status AS from_state \
             WHERE v.version = $version \
               AND (($to = 'OUT_OF_SERVICE' AND from_state IN ['AVAILABLE', 'ASSIGNED', 'HELD']) \
                 OR ($to = 'AVAILABLE' AND from_state = 'OUT_OF_SERVICE')) \
             OPTIONAL MATCH (r:Reservation {active_vehicle_key: v.vehicle_id}) \
             OPTIONAL MATCH (t:Task {task_id: r.task_id}) \
             WITH v, from_state, r, t, t IS NOT NULL AND t.status = 'ASSIGNED' AS requeue \
             SET v.status = $to, v.version = v.version + 1, v.updated_at = datetime(), v.status_reason = $reason \
             FOREACH (_ IN CASE WHEN r IS NULL THEN [] ELSE [1] END | \
                 SET r.status = 'RELEASED', r.released_at = datetime(), \
                     r.release_reason = 'vehicle out of service: ' + $reason \
                 REMOVE r.active_vehicle_key, r.active_task_key) \
             FOREACH (_ IN CASE WHEN requeue THEN [1] ELSE [] END | \
                 SET t.status = 'QUEUED', t.version = t.version + 1, t.updated_at = datetime(), \
                     t.assigned_vehicle_id = null, t.eta_s = null, t.assigned_at = null, \
                     t.requeued_at = datetime(), t.requeue_reason = 'vehicle out of service') \
             CREATE (v)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Vehicle', aggregate_id: v.vehicle_id, \
                 action: 'VEHICLE_' + $to, actor_id: $actor, from_state: from_state, to_state: $to, \
                 previous_version: $version, new_version: v.version, reason: $reason, \
                 occurred_at: datetime(), details_json: '{}' }) \
             WITH v, from_state, t, requeue \
             OPTIONAL MATCH (b:Reservation {task_id: t.task_id, resource_kind: 'BEDS'})-[:AT_SHELTER]->(s:Shelter) \
             WHERE requeue AND b.active_task_key IS NOT NULL \
             FOREACH (_ IN CASE WHEN b IS NULL THEN [] ELSE [1] END | \
                 SET b.status = 'RELEASED', b.released_at = datetime(), b.release_reason = 'task re-queued' \
                 REMOVE b.active_task_key \
                 SET s.current_occupancy = CASE WHEN s.current_occupancy >= b.beds \
                         THEN s.current_occupancy - b.beds ELSE 0 END, \
                     s.version = coalesce(s.version, 0) + 1, s.last_updated = datetime()) \
             RETURN from_state, v.version AS version, CASE WHEN requeue THEN t.task_id ELSE null END AS requeued_task",
        )
        .param("id", command.vehicle_id.clone())
        .param("version", command.expected_version as i64)
        .param("to", to)
        .param("reason", command.reason.trim())
        .param("actor", actor.actor_id.clone())
        .param("audit_id", new_id()),
    )
    .await?;
    let Some(row) = row else {
        return Err(vehicle_refusal(txn, command, to).await);
    };
    let requeued: Option<String> = row.get("requeued_task")?;
    let outcome = json!({
        "vehicle_id": command.vehicle_id,
        "from": row.get::<String>("from_state")?,
        "status": to,
        "version": row.get::<i64>("version")?,
        "requeued_task": requeued,
    });
    let mut events = vec![("VEHICLE_STATUS_CHANGED", command.vehicle_id.clone())];
    if !command.out_of_service {
        events.push(("VEHICLE_AVAILABLE", command.vehicle_id.clone()));
    }
    if let Some(task) = &requeued {
        events.push(("TASK_QUEUED", task.clone()));
        events.push(("RESERVATION_RELEASED", task.clone()));
    }
    write_events(txn, &events, &outcome).await?;
    Ok(outcome)
}

async fn vehicle_refusal(txn: &mut Txn, command: &VehicleStatusCommand, to: &str) -> OpsRepoError {
    let found = txn_first_row(
        txn,
        query("OPTIONAL MATCH (v:Vehicle {vehicle_id: $id}) RETURN v.status AS status, v.version AS version")
            .param("id", command.vehicle_id.clone()),
    )
    .await;
    let Ok(Some(row)) = found else {
        return OpsRepoError::Database("vehicle check failed".into());
    };
    let Some(status) = row.get::<Option<String>>("status").unwrap_or(None) else {
        return OpsRepoError::NotFound(format!("vehicle {}", command.vehicle_id));
    };
    let version = row.get::<Option<i64>>("version").unwrap_or(None).unwrap_or(-1);
    if version != command.expected_version as i64 {
        return OpsRepoError::NotAvailable(format!(
            "vehicle {} is at version {version}, not {}",
            command.vehicle_id, command.expected_version
        ));
    }
    OpsRepoError::InvalidData(format!("a {status} vehicle cannot become {to}"))
}

fn require(actor: &ActorContext, any_of: &[Permission]) -> Result<(), OpsRepoError> {
    if any_of.iter().any(|p| actor.can(*p)) {
        Ok(())
    } else {
        Err(OpsRepoError::Forbidden(format!("{} may not run this command", actor.actor_id)))
    }
}

fn check_text(value: &str, name: &str, max: usize) -> Result<(), OpsRepoError> {
    let chars = value.trim().chars().count();
    if chars == 0 || chars > max {
        return Err(OpsRepoError::InvalidData(format!("{name} must be 1..={max} characters")));
    }
    Ok(())
}

/// Field checks for a review command (also used by the API before it runs).
pub fn validate_review(command: &ReviewCommand) -> Result<(), OpsRepoError> {
    check_text(&command.request_id, "request_id", 128)?;
    check_text(&command.incident_id, "incident_id", 128)?;
    check_text(&command.reason, "reason", MAX_REASON_CHARS)?;
    if let ReviewVerdict::Approve { headcount, junction_id, .. } = &command.decision {
        if !(1..=MAX_HEADCOUNT).contains(headcount) {
            return Err(OpsRepoError::InvalidData(format!("headcount must be 1..={MAX_HEADCOUNT}")));
        }
        check_text(junction_id, "junction_id", 128)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approve() -> ReviewCommand {
        ReviewCommand {
            request_id: "r1".into(),
            dataset: "pilot".into(),
            incident_id: "i1".into(),
            expected_version: 0,
            decision: ReviewVerdict::Approve {
                required_asset: AssetType::Ambulance,
                headcount: 2,
                junction_id: "node/1".into(),
                urgency: None,
            },
            reason: "called back, two people".into(),
        }
    }

    #[test]
    fn review_validation() {
        assert!(validate_review(&approve()).is_ok());
        let mut zero = approve();
        zero.decision = ReviewVerdict::Approve {
            required_asset: AssetType::Ambulance,
            headcount: 0,
            junction_id: "node/1".into(),
            urgency: None,
        };
        assert!(validate_review(&zero).is_err());
        assert!(validate_review(&ReviewCommand { reason: "  ".into(), ..approve() }).is_err());
        assert!(validate_review(&ReviewCommand { reason: "x".repeat(501), ..approve() }).is_err());
        let reject = ReviewCommand { decision: ReviewVerdict::Reject, ..approve() };
        assert!(validate_review(&reject).is_ok());
    }
}
