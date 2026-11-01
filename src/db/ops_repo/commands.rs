//! Dispatcher commands (M7; pulled forward from M6 by owner decision):
//! review of a `REVIEW_REQUIRED` incident (approve or reject) and task
//! completion (T5).
//!
//! Each command is one transaction: role check (by the caller), version
//! check, the change, audit with the actor, outbox events, and a `:Command`
//! record keyed by `request_id`. A repeated `request_id` returns the stored
//! outcome and changes nothing.

use neo4rs::{query, Txn};
use serde_json::{json, Value};

use super::{new_id, txn_first_row, OpsRepoError, OpsRepository, MAX_WRITE_ATTEMPTS};
use crate::models::{active_task_key, ActorContext, AssetType, Permission, ResourceKind, Urgency};

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
