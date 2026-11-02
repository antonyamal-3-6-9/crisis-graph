//! Group plans (T8, owner decision 2026-11-02): a road pickup that cannot be
//! served as requested (no route for the requested vehicle type, or more
//! people than one vehicle of that type carries) is served automatically by
//! another road vehicle type and/or several vehicles, with a dispatcher
//! alert. The matcher decides with `models::group_plan` on graph facts only;
//! these transactions record the plan.
//!
//! Each task of a planned incident carries its own `persons`, used for seats
//! and drop-off beds. If the incident's headcount later rises (a follow-up
//! call or a merge), [`OpsRepository::coverage_gaps`] finds the people not
//! yet covered and [`OpsRepository::top_up_group`] adds vehicles for them.

use neo4rs::query;
use serde_json::json;

use super::{new_id, OpsRepoError, OpsRepository, SYSTEM_ACTOR};
use crate::models::{AssetType, GroupPlan, QueuedTask};

/// A planned incident with more people than its pickups cover.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverageGap {
    pub incident_id: String,
    pub incident_version: u64,
    pub asset: AssetType,
    pub headcount: u32,
    pub covered: u32,
}

impl CoverageGap {
    pub fn missing(&self) -> u32 {
        self.headcount.saturating_sub(self.covered)
    }
}

impl OpsRepository {
    /// Apply `plan` to a QUEUED road pickup (guarded by its version, so it
    /// happens once): the task takes the plan's vehicle type and the first
    /// share of people; a QUEUED `ROAD_PICKUP` is added for each other share.
    /// Raises `ASSET_SWITCHED` (type changed) or `GROUP_SPLIT` (same type,
    /// more vehicles). Returns the new task IDs, or `None` if the task changed.
    pub async fn apply_group_plan(&self, task: &QueuedTask, plan: &GroupPlan) -> Result<Option<Vec<String>>, OpsRepoError> {
        let extra: Vec<(String, u32)> = plan.shares.iter().skip(1).map(|share| (new_id(), *share)).collect();
        let switched = plan.switched_from(&task.required_asset);
        let alert = if switched.is_some() { "ASSET_SWITCHED" } else { "GROUP_SPLIT" };
        let payload = json!({
            "incident_id": task.incident_id,
            "task_id": task.task_id,
            "reason": plan.reason.as_str(),
            "requested_asset": task.required_asset.to_string(),
            "asset": plan.asset.to_string(),
            "persons": task.persons(),
            "shares": plan.shares,
            "added_task_ids": extra.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        })
        .to_string();
        let extra_rows: Vec<neo4rs::BoltType> = extra
            .iter()
            .map(|(id, persons)| {
                let mut map = std::collections::HashMap::new();
                map.insert("id".to_string(), neo4rs::BoltType::from(id.clone()));
                map.insert("persons".to_string(), neo4rs::BoltType::from(*persons as i64));
                map.insert("queued_event".to_string(), neo4rs::BoltType::from(new_id()));
                neo4rs::BoltType::from(map)
            })
            .collect();
        let q = query(
            "MATCH (t:Task {task_id: $task_id}) \
             SET t._lock = true \
             REMOVE t._lock \
             WITH t \
             WHERE t.status = 'QUEUED' AND t.version = $version AND t.kind = 'ROAD_PICKUP' \
             MATCH (i:OpsIncident {incident_id: t.incident_id}) \
             OPTIONAL MATCH (i)-[h:HAS_TASK]->() \
             WITH t, i, coalesce(max(h.seq), 1) AS seq \
             SET t.asset_switched_from = CASE WHEN t.required_asset <> $asset THEN t.required_asset \
                     ELSE t.asset_switched_from END, \
                 t.required_asset = $asset, t.persons = $first, t.group_reason = $reason, \
                 t.version = t.version + 1, t.updated_at = datetime(), \
                 i.group_plan_asset = $asset, i.group_plan_json = $payload, \
                 i.version = i.version + 1, i.updated_at = datetime() \
             FOREACH (k IN range(0, size($extra) - 1) | \
                 CREATE (i)-[:HAS_TASK {seq: seq + 1 + k}]->(:Task { \
                     task_id: $extra[k].id, incident_id: i.incident_id, kind: 'ROAD_PICKUP', status: 'QUEUED', \
                     required_asset: $asset, persons: $extra[k].persons, group_reason: $reason, \
                     asset_switched_from: CASE WHEN $requested <> $asset THEN $requested ELSE null END, \
                     urgency: t.urgency, priority_rank: t.priority_rank, version: 0, \
                     created_at: datetime(), queued_at: t.queued_at, traceparent: t.traceparent }) \
                 CREATE (:OutboxEvent { event_id: $extra[k].queued_event, aggregate_id: $extra[k].id, \
                     type: 'TASK_QUEUED', payload_json: $payload, created_at: datetime(), published: false, attempts: 0 })) \
             CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Incident', aggregate_id: i.incident_id, action: 'GROUP_PLANNED', \
                 actor_id: $actor, new_version: i.version, \
                 reason: $reason + ': ' + toString(size($extra) + 1) + ' x ' + $asset, \
                 occurred_at: datetime(), details_json: $payload }) \
             CREATE (:OutboxEvent { event_id: $alert_id, aggregate_id: i.incident_id, type: $alert, \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             CREATE (:OutboxEvent { event_id: $changed_id, aggregate_id: t.task_id, type: 'TASK_QUEUED', \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             RETURN t.task_id AS id",
        )
        .param("task_id", task.task_id.clone())
        .param("version", task.version as i64)
        .param("asset", plan.asset.to_string())
        .param("requested", task.required_asset.to_string())
        .param("first", plan.shares.first().copied().unwrap_or(1) as i64)
        .param("reason", plan.reason.as_str())
        .param("extra", extra_rows)
        .param("payload", payload)
        .param("alert", alert)
        .param("actor", SYSTEM_ACTOR)
        .param("audit_id", new_id())
        .param("alert_id", new_id())
        .param("changed_id", new_id());
        Ok(self.write_first_row(q).await?.map(|_| extra.into_iter().map(|(id, _)| id).collect()))
    }

    /// Planned incidents in `dataset` whose headcount exceeds the people
    /// their road pickups cover (cancelled pickups do not count).
    pub async fn coverage_gaps(&self, dataset: &str) -> Result<Vec<CoverageGap>, OpsRepoError> {
        let q = query(
            "MATCH (i:OpsIncident)-[:LOCATED_AT]->(:Junction {dataset: $dataset}) \
             WHERE i.group_plan_asset IS NOT NULL AND i.mission IS NULL \
               AND NOT i.status IN ['MERGED', 'REJECTED', 'COMPLETED', 'REVIEW_REQUIRED'] \
             OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task {kind: 'ROAD_PICKUP'}) WHERE t.status <> 'CANCELLED' \
             WITH i, sum(coalesce(t.persons, 0)) AS covered \
             WHERE coalesce(i.headcount, 1) > covered \
             RETURN i.incident_id AS incident_id, i.version AS version, i.group_plan_asset AS asset, \
                    coalesce(i.headcount, 1) AS headcount, covered",
        )
        .param("dataset", dataset);
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut gaps = Vec::new();
        while let Some(row) = result.next().await? {
            gaps.push(CoverageGap {
                incident_id: row.get("incident_id")?,
                incident_version: row.get::<i64>("version")?.max(0) as u64,
                asset: super::parse_asset(&row.get::<String>("asset")?)?,
                headcount: row.get::<i64>("headcount")?.max(0) as u32,
                covered: row.get::<i64>("covered")?.max(0) as u32,
            });
        }
        Ok(gaps)
    }

    /// Add QUEUED road pickups for the people a planned incident's pickups
    /// do not cover yet (guarded by the incident version). Raises
    /// `GROUP_SPLIT`. Returns the new task IDs, or `None` if it changed.
    pub async fn top_up_group(&self, gap: &CoverageGap, shares: &[u32]) -> Result<Option<Vec<String>>, OpsRepoError> {
        let added: Vec<(String, u32)> = shares.iter().map(|share| (new_id(), *share)).collect();
        let payload = json!({
            "incident_id": gap.incident_id,
            "reason": "HEADCOUNT_ROSE",
            "asset": gap.asset.to_string(),
            "headcount": gap.headcount,
            "covered_before": gap.covered,
            "shares": shares,
            "added_task_ids": added.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        })
        .to_string();
        let rows: Vec<neo4rs::BoltType> = added
            .iter()
            .map(|(id, persons)| {
                let mut map = std::collections::HashMap::new();
                map.insert("id".to_string(), neo4rs::BoltType::from(id.clone()));
                map.insert("persons".to_string(), neo4rs::BoltType::from(*persons as i64));
                map.insert("queued_event".to_string(), neo4rs::BoltType::from(new_id()));
                neo4rs::BoltType::from(map)
            })
            .collect();
        let q = query(
            "MATCH (i:OpsIncident {incident_id: $incident_id}) \
             SET i._lock = true \
             REMOVE i._lock \
             WITH i \
             WHERE i.version = $version \
             OPTIONAL MATCH (i)-[h:HAS_TASK]->() \
             WITH i, coalesce(max(h.seq), 1) AS seq \
             SET i.version = i.version + 1, i.updated_at = datetime() \
             FOREACH (k IN range(0, size($added) - 1) | \
                 CREATE (i)-[:HAS_TASK {seq: seq + 1 + k}]->(:Task { \
                     task_id: $added[k].id, incident_id: i.incident_id, kind: 'ROAD_PICKUP', status: 'QUEUED', \
                     required_asset: $asset, persons: $added[k].persons, group_reason: 'HEADCOUNT_ROSE', \
                     urgency: i.urgency, priority_rank: coalesce(i.priority_rank, 1), version: 0, \
                     created_at: datetime(), queued_at: datetime(), traceparent: i.traceparent }) \
                 CREATE (:OutboxEvent { event_id: $added[k].queued_event, aggregate_id: $added[k].id, \
                     type: 'TASK_QUEUED', payload_json: $payload, created_at: datetime(), published: false, attempts: 0 })) \
             CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Incident', aggregate_id: i.incident_id, action: 'GROUP_TOPPED_UP', \
                 actor_id: $actor, new_version: i.version, reason: 'headcount rose: ' + toString(size($added)) + ' x ' + $asset, \
                 occurred_at: datetime(), details_json: $payload }) \
             CREATE (:OutboxEvent { event_id: $alert_id, aggregate_id: i.incident_id, type: 'GROUP_SPLIT', \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             RETURN i.incident_id AS id",
        )
        .param("incident_id", gap.incident_id.clone())
        .param("version", gap.incident_version as i64)
        .param("asset", gap.asset.to_string())
        .param("added", rows)
        .param("payload", payload)
        .param("actor", SYSTEM_ACTOR)
        .param("audit_id", new_id())
        .param("alert_id", new_id());
        Ok(self.write_first_row(q).await?.map(|_| added.into_iter().map(|(id, _)| id).collect()))
    }
}
