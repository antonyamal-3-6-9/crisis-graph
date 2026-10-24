//! Triage-side writes: the extraction checkpoint, transaction T1 (create a
//! new incident or attach a follow-up) and possible-duplicate flagging.

use neo4rs::{query, Query, Row, Txn};
use serde_json::json;

use super::reports::{report_from_row, REPORT_FIELDS};
use super::{new_id, OpsRepoError, OpsRepository, SYSTEM_ACTOR, MAX_WRITE_ATTEMPTS};
use crate::models::{ExtractionRecord, TriageCommit, TriageState, OPEN_INCIDENT_STATUSES};

fn open_statuses() -> Vec<String> {
    OPEN_INCIDENT_STATUSES.iter().map(|s| s.to_string()).collect()
}

impl OpsRepository {
    /// Start processing a report: count the attempt and return the report
    /// with the incident it is already linked to, if any.
    pub async fn begin_triage(&self, report_id: &str) -> Result<Option<TriageState>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (r:Report {{report_id: $report_id}}) \
             SET r.triage_attempts = coalesce(r.triage_attempts, 0) + 1 \
             WITH r \
             OPTIONAL MATCH (r)-[:REPORTS]->(i:OpsIncident) \
             RETURN {REPORT_FIELDS}, r.triage_attempts AS attempts, i.incident_id AS linked"
        ))
        .param("report_id", report_id);
        match self.write_first_row(q).await? {
            Some(row) => Ok(Some(TriageState {
                report: report_from_row(&row)?,
                attempts: row.get::<i64>("attempts")? as u32,
                linked_incident_id: row.get("linked")?,
            })),
            None => Ok(None),
        }
    }

    /// Save the extraction on the report (the checkpoint before
    /// deduplication). Only a `PENDING` report is updated, so a redelivered
    /// message never overwrites a saved extraction. The report's location is
    /// the caller's coordinates if given, else the resolved junction.
    pub async fn save_extraction(&self, report_id: &str, record: &ExtractionRecord) -> Result<bool, OpsRepoError> {
        let q = query(
            "MATCH (r:Report {report_id: $report_id}) \
             WHERE r.extraction_status = 'PENDING' \
             OPTIONAL MATCH (j:Junction {id: $junction_id}) \
             WITH r, head(collect(j)) AS j \
             SET r.extraction_status = $status, r.extraction_json = $json, \
                 r.headcount = $headcount, r.required_asset = $required_asset, \
                 r.resolved_junction_id = $junction_id, r.dispatch_ready = $ready, \
                 r.review_reasons = $reasons, r.extracted_at = datetime(), \
                 r.location = CASE \
                     WHEN r.latitude IS NOT NULL AND r.longitude IS NOT NULL \
                         THEN point({latitude: r.latitude, longitude: r.longitude}) \
                     WHEN j IS NOT NULL THEN point({latitude: j.lat, longitude: j.lon}) \
                     ELSE null END \
             RETURN r.report_id AS report_id",
        )
        .param("report_id", report_id)
        .param("status", if record.succeeded { "EXTRACTED" } else { "FAILED" })
        .param("json", record.extraction_json.clone())
        .param("headcount", record.headcount.map(|h| h as i64))
        .param("required_asset", record.required_asset.as_ref().map(|a| a.to_string()))
        .param("junction_id", record.resolved_junction_id.clone())
        .param("ready", record.dispatch_ready)
        .param("reasons", record.review_reasons.clone());
        Ok(self.write_first_row(q).await?.is_some())
    }

    /// Transaction T1. Locks the report, then:
    /// - already linked → `AlreadyLinked` (redelivered or duplicate message);
    /// - rule D-2: an open incident has a report from the same device within
    ///   `same_device_window_secs` → attach as a follow-up (largest headcount
    ///   kept);
    /// - otherwise create an `:OpsIncident` (TRIAGE_APPROVED with a QUEUED
    ///   ROAD_PICKUP task if dispatch-ready, else REVIEW_REQUIRED).
    ///
    /// Audit and outbox events are written in the same transaction.
    pub async fn commit_triage(
        &self,
        report_id: &str,
        same_device_window_secs: u64,
    ) -> Result<TriageCommit, OpsRepoError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut txn = self.neo4j.graph.start_txn().await?;
            let outcome = commit_triage_in(&mut txn, report_id, same_device_window_secs).await;
            let outcome = match outcome {
                Ok(commit) => txn.commit().await.map(|_| commit).map_err(OpsRepoError::from),
                Err(error) => {
                    let _ = txn.rollback().await;
                    Err(error)
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

    /// Rule D-3, run after T1: link this incident to other open incidents at
    /// the same junction or within `radius_m`, created within `window_secs`,
    /// as POSSIBLE_DUPLICATE (never merged automatically). Idempotent; returns
    /// the IDs newly flagged by this call.
    pub async fn flag_possible_duplicates(
        &self,
        incident_id: &str,
        radius_m: f64,
        window_secs: u64,
    ) -> Result<Vec<String>, OpsRepoError> {
        // Each pair is stored once, from the lower to the higher incident ID.
        // Locking that canonical node first makes concurrent flaggers of the
        // same pair queue, so MERGE sees the other's link instead of adding
        // a second one.
        let q = query(
            "MATCH (i:OpsIncident {incident_id: $incident_id}) \
             MATCH (o:OpsIncident) \
             WHERE o.incident_id <> i.incident_id AND o.status IN $open \
               AND abs(duration.inSeconds(o.created_at, i.created_at).seconds) <= $window \
               AND ((i.junction_id IS NOT NULL AND o.junction_id = i.junction_id) \
                    OR (i.location IS NOT NULL AND o.location IS NOT NULL \
                        AND point.distance(i.location, o.location) <= $radius)) \
             WITH i, o, CASE WHEN i.incident_id < o.incident_id THEN [i, o] ELSE [o, i] END AS pair \
             WITH i, o, pair[0] AS a, pair[1] AS b \
             SET a._lock = true \
             REMOVE a._lock \
             MERGE (a)-[d:POSSIBLE_DUPLICATE]->(b) \
             ON CREATE SET d.flagged_at = datetime(), d.rule = 'D-3', d.resolution = 'OPEN', \
                 d.distance_m = CASE WHEN a.location IS NULL OR b.location IS NULL THEN null \
                     ELSE point.distance(a.location, b.location) END, \
                 d._new = true \
             WITH i, o, d, coalesce(d._new, false) AS created \
             REMOVE d._new \
             FOREACH (_ IN CASE WHEN created THEN [1] ELSE [] END | \
                 CREATE (:OutboxEvent { \
                     event_id: randomUUID(), aggregate_id: i.incident_id, type: 'POSSIBLE_DUPLICATE_FLAGGED', \
                     payload_json: '{\"incident_id\":\"' + i.incident_id + '\",\"other_incident_id\":\"' + o.incident_id + '\"}', \
                     created_at: datetime(), published: false, attempts: 0 })) \
             RETURN [x IN collect(CASE WHEN created THEN o.incident_id END) WHERE x IS NOT NULL] AS flagged",
        )
        .param("incident_id", incident_id)
        .param("open", open_statuses())
        .param("window", window_secs as i64)
        .param("radius", radius_m);
        match self.write_first_row(q).await? {
            Some(row) => Ok(row.get::<Vec<String>>("flagged")?),
            None => Ok(Vec::new()),
        }
    }
}

async fn commit_triage_in(
    txn: &mut Txn,
    report_id: &str,
    window_secs: u64,
) -> Result<TriageCommit, OpsRepoError> {
    // 1. Lock the report so concurrent processing of the same report queues.
    let row = txn_first_row(
        txn,
        query(
            "MATCH (r:Report {report_id: $report_id}) \
             SET r._lock = true \
             REMOVE r._lock \
             WITH r \
             OPTIONAL MATCH (r)-[:REPORTS]->(i:OpsIncident) \
             RETURN r.extraction_status AS status, r.device_id AS device_id, \
                    toString(r.received_at) AS received_at, \
                    coalesce(r.dispatch_ready, false) AND r.required_asset IS NOT NULL AS ready, \
                    i.incident_id AS linked",
        )
        .param("report_id", report_id),
    )
    .await?
    .ok_or_else(|| OpsRepoError::NotFound(format!("report {report_id}")))?;

    if row.get::<String>("status")? == "PENDING" {
        return Err(OpsRepoError::InvalidData(format!(
            "report {report_id} has no saved extraction"
        )));
    }
    if let Some(linked) = row.get::<Option<String>>("linked")? {
        return Ok(TriageCommit::AlreadyLinked { incident_id: linked });
    }
    let device_id: String = row.get("device_id")?;
    let received_at: String = row.get("received_at")?;
    let ready: bool = row.get("ready")?;

    // 2. Rule D-2: an open incident with a report from the same device.
    let followup = txn_first_row(
        txn,
        query(
            "MATCH (prev:Report {device_id: $device_id})-[:REPORTS]->(i:OpsIncident) \
             WHERE prev.report_id <> $report_id AND i.status IN $open \
               AND abs(duration.inSeconds(prev.received_at, datetime($received_at)).seconds) <= $window \
             RETURN i.incident_id AS incident_id \
             ORDER BY prev.received_at DESC LIMIT 1",
        )
        .param("device_id", device_id)
        .param("report_id", report_id)
        .param("open", open_statuses())
        .param("received_at", received_at)
        .param("window", window_secs as i64),
    )
    .await?;

    if let Some(row) = followup {
        let incident_id: String = row.get("incident_id")?;
        let attached = txn_first_row(
            txn,
            query(
                "MATCH (r:Report {report_id: $report_id}), (i:OpsIncident {incident_id: $incident_id}) \
                 SET i._lock = true \
                 REMOVE i._lock \
                 CREATE (r)-[:REPORTS {kind: 'FOLLOW_UP', attached_at: datetime(), rule: 'D-2'}]->(i) \
                 WITH r, i, coalesce(i.headcount, 0) AS before \
                 SET i.headcount = CASE WHEN coalesce(r.headcount, 0) > before THEN r.headcount ELSE i.headcount END, \
                     i.version = i.version + 1, i.updated_at = datetime(), \
                     i.report_count = coalesce(i.report_count, 1) + 1, r.dedup_rule = 'D-2' \
                 WITH r, i, coalesce(r.headcount, 0) > before AS increased \
                 CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                     event_id: $audit_id, aggregate_type: 'Incident', aggregate_id: i.incident_id, \
                     action: 'REPORT_ATTACHED', actor_id: $actor_id, previous_version: i.version - 1, \
                     new_version: i.version, reason: 'same-device follow-up (D-2): report ' + r.report_id, \
                     occurred_at: datetime(), details_json: '{}' }) \
                 CREATE (:OutboxEvent { \
                     event_id: $outbox_id, aggregate_id: i.incident_id, type: 'INCIDENT_UPDATED', \
                     payload_json: '{\"incident_id\":\"' + i.incident_id + '\",\"report_id\":\"' + r.report_id + \
                         '\",\"headcount_increased\":' + toString(increased) + '}', \
                     created_at: datetime(), published: false, attempts: 0 }) \
                 RETURN increased",
            )
            .param("report_id", report_id)
            .param("incident_id", incident_id.clone())
            .param("audit_id", new_id())
            .param("outbox_id", new_id())
            .param("actor_id", SYSTEM_ACTOR),
        )
        .await?
        .ok_or_else(|| OpsRepoError::NotFound(format!("incident {incident_id}")))?;
        return Ok(TriageCommit::Attached {
            incident_id,
            headcount_increased: attached.get("increased")?,
        });
    }

    // 3. New incident.
    let incident_id = new_id();
    let task_id = ready.then(new_id);
    let (status, event_type) = if ready {
        ("TRIAGE_APPROVED", "INCIDENT_READY")
    } else {
        ("REVIEW_REQUIRED", "INCIDENT_REVIEW_REQUIRED")
    };
    txn_first_row(
        txn,
        query(
            "MATCH (r:Report {report_id: $report_id}) \
             OPTIONAL MATCH (j:Junction {id: r.resolved_junction_id}) \
             WITH r, head(collect(j)) AS j \
             CREATE (i:OpsIncident { \
                 incident_id: $incident_id, status: $status, version: 0, headcount: r.headcount, \
                 required_asset: r.required_asset, location: r.location, \
                 junction_id: r.resolved_junction_id, device_id: r.device_id, \
                 review_reasons: coalesce(r.review_reasons, []), report_count: 1, \
                 created_at: datetime(), updated_at: datetime() }) \
             CREATE (r)-[:REPORTS {kind: 'PRIMARY', attached_at: datetime(), rule: 'NEW'}]->(i) \
             SET r.dedup_rule = 'NEW' \
             FOREACH (_ IN CASE WHEN j IS NULL THEN [] ELSE [1] END | CREATE (i)-[:LOCATED_AT]->(j)) \
             FOREACH (_ IN CASE WHEN $task_id IS NULL THEN [] ELSE [1] END | \
                 CREATE (i)-[:HAS_TASK {seq: 1}]->(:Task { \
                     task_id: $task_id, incident_id: $incident_id, kind: 'ROAD_PICKUP', status: 'QUEUED', \
                     required_asset: r.required_asset, version: 0, created_at: datetime(), \
                     queued_at: datetime() })) \
             CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Incident', aggregate_id: $incident_id, \
                 action: 'INCIDENT_CREATED', actor_id: $actor_id, to_state: $status, new_version: 0, \
                 reason: 'triaged report ' + r.report_id, occurred_at: datetime(), details_json: $payload }) \
             CREATE (:OutboxEvent { \
                 event_id: $outbox_id, aggregate_id: $incident_id, type: $event_type, \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             RETURN i.incident_id AS incident_id",
        )
        .param("report_id", report_id)
        .param("incident_id", incident_id.clone())
        .param("status", status)
        .param("event_type", event_type)
        .param("task_id", task_id.clone())
        .param("audit_id", new_id())
        .param("outbox_id", new_id())
        .param("actor_id", SYSTEM_ACTOR)
        .param(
            "payload",
            json!({ "incident_id": incident_id, "report_id": report_id, "task_id": task_id, "status": status })
                .to_string(),
        ),
    )
    .await?;
    Ok(TriageCommit::Created { incident_id, task_id })
}

/// Execute inside a transaction and return the first row, draining the rest
/// so the connection is ready for the next statement.
async fn txn_first_row(txn: &mut Txn, q: Query) -> Result<Option<Row>, OpsRepoError> {
    let mut stream = txn.execute(q).await?;
    let first = stream.next(txn.handle()).await?;
    while stream.next(txn.handle()).await?.is_some() {}
    Ok(first)
}
