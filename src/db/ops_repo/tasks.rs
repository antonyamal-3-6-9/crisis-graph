use neo4rs::{query, Row};

use super::{new_id, parse_asset, parse_enum, OpsRepoError, OpsRepository, SYSTEM_ACTOR};
use crate::models::{AssetType, Task, TaskKind, TaskStatus};

const TASK_FIELDS: &str = "t.task_id AS task_id, t.incident_id AS incident_id, t.kind AS kind, \
     t.status AS status, t.required_asset AS required_asset, t.version AS version";

impl OpsRepository {
    /// Create a task if it does not exist (keyed by `task_id`), so a repeated
    /// creation is harmless. Returns the stored task.
    pub async fn create_task(
        &self,
        task_id: &str,
        incident_id: &str,
        kind: TaskKind,
        status: TaskStatus,
        required_asset: AssetType,
    ) -> Result<Task, OpsRepoError> {
        let q = query(&format!(
            "MERGE (t:Task {{task_id: $task_id}}) \
             ON CREATE SET t.incident_id = $incident_id, t.kind = $kind, t.status = $status, \
                 t.required_asset = $required_asset, t.version = 0, t.created_at = datetime(), \
                 t.queued_at = CASE WHEN $status = 'QUEUED' THEN datetime() ELSE null END \
             WITH t \
             OPTIONAL MATCH (i:Incident {{incident_id: $incident_id}}) \
             FOREACH (_ IN CASE WHEN i IS NULL THEN [] ELSE [1] END | \
                 MERGE (i)-[:HAS_TASK]->(t)) \
             RETURN {TASK_FIELDS}"
        ))
        .param("task_id", task_id)
        .param("incident_id", incident_id)
        .param("kind", kind.as_str())
        .param("status", status.as_str())
        .param("required_asset", required_asset.to_string());
        let row = self
            .write_first_row(q)
            .await?
            .ok_or_else(|| OpsRepoError::Database("task write returned no row".into()))?;
        task_from_row(&row)
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Option<Task>, OpsRepoError> {
        let q = query(&format!("MATCH (t:Task {{task_id: $task_id}}) RETURN {TASK_FIELDS}"))
            .param("task_id", task_id);
        let mut result = self.neo4j.graph.execute(q).await?;
        match result.next().await? {
            Some(row) => Ok(Some(task_from_row(&row)?)),
            None => Ok(None),
        }
    }

    /// Move a task between statuses, guarded by its current status and
    /// version (optimistic concurrency), with audit and outbox in the same
    /// statement. Fails with `NotAvailable` if the task changed meanwhile.
    pub async fn transition_task(
        &self,
        task_id: &str,
        expected_version: u64,
        from: TaskStatus,
        to: TaskStatus,
        reason: &str,
    ) -> Result<Task, OpsRepoError> {
        let q = query(&format!(
            "MATCH (t:Task {{task_id: $task_id}}) \
             WHERE t.status = $from AND t.version = $expected_version \
             SET t.status = $to, t.version = t.version + 1, t.updated_at = datetime(), \
                 t.queued_at = CASE WHEN $to = 'QUEUED' THEN datetime() ELSE t.queued_at END \
             CREATE (t)-[:HAS_AUDIT_EVENT]->(:AuditEvent {{ \
                 event_id: $audit_id, aggregate_type: 'Task', aggregate_id: $task_id, \
                 action: 'TASK_' + $to, actor_id: $actor_id, from_state: $from, to_state: $to, \
                 previous_version: $expected_version, new_version: t.version, reason: $reason, \
                 occurred_at: datetime(), details_json: '{{}}' }}) \
             CREATE (:OutboxEvent {{ \
                 event_id: $outbox_id, aggregate_id: $task_id, type: 'TASK_' + $to, \
                 payload_json: '{{\"task_id\":\"' + $task_id + '\",\"version\":' + toString(t.version) + '}}', \
                 created_at: datetime(), published: false, attempts: 0 }}) \
             RETURN {TASK_FIELDS}"
        ))
        .param("task_id", task_id)
        .param("expected_version", expected_version as i64)
        .param("from", from.as_str())
        .param("to", to.as_str())
        .param("reason", reason)
        .param("audit_id", new_id())
        .param("outbox_id", new_id())
        .param("actor_id", SYSTEM_ACTOR);
        match self.write_first_row(q).await? {
            Some(row) => task_from_row(&row),
            None => Err(OpsRepoError::NotAvailable(format!(
                "task {task_id} is not {} at version {expected_version}",
                from.as_str()
            ))),
        }
    }
}

fn task_from_row(row: &Row) -> Result<Task, OpsRepoError> {
    Ok(Task {
        task_id: row.get("task_id")?,
        incident_id: row.get("incident_id")?,
        kind: parse_enum(&row.get::<String>("kind")?, TaskKind::parse, "task kind")?,
        status: parse_enum(&row.get::<String>("status")?, TaskStatus::parse, "task status")?,
        required_asset: parse_asset(&row.get::<String>("required_asset")?)?,
        version: row.get::<i64>("version")? as u64,
    })
}
