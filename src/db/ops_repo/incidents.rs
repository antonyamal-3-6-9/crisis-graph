//! Triage-side writes: the extraction checkpoint, transaction T1 (create a
//! new incident or attach a follow-up) and possible-duplicate flagging.

use neo4rs::{query, Query, Row, Txn};
use serde_json::json;

use super::reports::{report_from_row, REPORT_FIELDS};
use super::{new_id, parse_enum, parse_time, OpsRepoError, OpsRepository, SYSTEM_ACTOR, MAX_WRITE_ATTEMPTS};
use crate::models::{
    CallbackItem, ExtractionRecord, ReviewPriority, TriageCommit, TriageState, OPEN_INCIDENT_STATUSES,
};

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
    /// message never overwrites a saved extraction.
    ///
    /// `location` is the victims' position (finding F-1): the caller's
    /// coordinates only when the extraction placed the caller on scene
    /// (`DEVICE_GPS`), otherwise the chosen junction, otherwise null. The
    /// caller's own position is kept separately as `caller_location`.
    pub async fn save_extraction(&self, report_id: &str, record: &ExtractionRecord) -> Result<bool, OpsRepoError> {
        let urgency = &record.urgency;
        let q = query(
            "MATCH (r:Report {report_id: $report_id}) \
             WHERE r.extraction_status = 'PENDING' \
             OPTIONAL MATCH (j:Junction {id: $junction_id}) \
             WITH r, head(collect(j)) AS j, \
                  CASE WHEN r.latitude IS NOT NULL AND r.longitude IS NOT NULL \
                      THEN point({latitude: r.latitude, longitude: r.longitude}) END AS caller \
             SET r.extraction_status = $status, r.extraction_json = $json, \
                 r.headcount = $headcount, r.required_asset = $required_asset, \
                 r.suggested_asset = $suggested_asset, \
                 r.resolved_junction_id = $junction_id, r.dispatch_ready = $ready, \
                 r.review_reasons = $reasons, r.extracted_at = datetime(), \
                 r.urgency = $urgency, r.rule_urgency = $rule_urgency, \
                 r.review_priority = $review_priority, r.review_rank = $review_rank, \
                 r.urgency_flags = $urgency_flags, r.urgency_rules_version = $rules_version, \
                 r.priority_rank = $priority_rank, r.immediate_attention = $immediate, \
                 r.location_source = $location_source, r.caller_location = caller, \
                 r.location = CASE \
                     WHEN $location_source = 'DEVICE_GPS' AND caller IS NOT NULL THEN caller \
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
        .param("reasons", record.review_reasons.clone())
        .param("suggested_asset", record.suggested_asset.as_ref().map(|a| a.to_string()))
        .param("urgency", urgency.urgency.as_str())
        .param("rule_urgency", urgency.rule_urgency.as_str())
        .param("review_priority", urgency.review_priority.map(|p| p.as_str()))
        .param("review_rank", urgency.review_priority.map(|p| p.rank() as i64))
        .param("urgency_flags", urgency.flags.iter().map(|f| f.as_str().to_string()).collect::<Vec<_>>())
        .param("rules_version", urgency.rules_version.clone())
        .param("priority_rank", urgency.urgency.rank() as i64)
        .param("immediate", urgency.needs_immediate_attention())
        .param("location_source", record.location_source.map(|s| s.as_str()));
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

    /// The dispatcher callback list: open incidents with UNKNOWN urgency,
    /// highest review priority first, then oldest first.
    pub async fn callback_queue(&self, limit: usize) -> Result<Vec<CallbackItem>, OpsRepoError> {
        let q = query(
            "MATCH (i:OpsIncident) \
             WHERE i.callback_required = true AND i.status IN $open \
             RETURN i.incident_id AS incident_id, i.status AS status, \
                    i.review_priority AS review_priority, toString(i.created_at) AS created_at \
             ORDER BY coalesce(i.review_rank, 2) DESC, i.created_at ASC \
             LIMIT $limit",
        )
        .param("open", open_statuses())
        .param("limit", limit as i64);
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut items = Vec::new();
        while let Some(row) = result.next().await? {
            items.push(CallbackItem {
                incident_id: row.get("incident_id")?,
                status: row.get("status")?,
                review_priority: row
                    .get::<Option<String>>("review_priority")?
                    .map(|p| parse_enum(&p, ReviewPriority::parse, "review priority"))
                    .transpose()?,
                created_at: parse_time(&row.get::<String>("created_at")?)?,
            });
        }
        Ok(items)
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
                    coalesce(r.urgency, 'UNKNOWN') AS urgency, coalesce(r.urgency_flags, []) AS flags, \
                    r.review_priority AS review_priority, \
                    coalesce(r.immediate_attention, false) AS immediate, \
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
    let urgency: String = row.get("urgency")?;
    let flags: Vec<String> = row.get("flags")?;
    let review_priority: Option<String> = row.get("review_priority")?;
    let immediate: bool = row.get("immediate")?;

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
                 WITH r, i, coalesce(i.headcount, 0) AS before, \
                      coalesce(r.priority_rank, 1) AS new_rank, coalesce(i.priority_rank, 1) AS old_rank \
                 WITH r, i, before, \
                      new_rank > old_rank OR (new_rank = old_rank \
                          AND coalesce(i.urgency, 'UNKNOWN') = 'UNKNOWN' AND r.urgency = 'URGENT') AS raise \
                 WITH r, i, before, raise, \
                      NOT raise AND coalesce(i.urgency, 'UNKNOWN') = 'UNKNOWN' AND r.urgency = 'UNKNOWN' \
                          AND coalesce(r.review_rank, 0) > coalesce(i.review_rank, 0) AS bump_review \
                 SET i.headcount = CASE WHEN coalesce(r.headcount, 0) > before THEN r.headcount ELSE i.headcount END, \
                     i.urgency = CASE WHEN raise THEN r.urgency ELSE i.urgency END, \
                     i.rule_urgency = CASE WHEN raise THEN r.rule_urgency ELSE i.rule_urgency END, \
                     i.urgency_flags = CASE WHEN raise THEN r.urgency_flags ELSE i.urgency_flags END, \
                     i.urgency_rules_version = CASE WHEN raise THEN r.urgency_rules_version \
                         ELSE i.urgency_rules_version END, \
                     i.priority_rank = CASE WHEN raise THEN r.priority_rank ELSE i.priority_rank END, \
                     i.review_priority = CASE WHEN raise OR bump_review THEN r.review_priority \
                         ELSE i.review_priority END, \
                     i.review_rank = CASE WHEN raise OR bump_review THEN r.review_rank ELSE i.review_rank END, \
                     i.callback_required = CASE WHEN raise THEN r.urgency = 'UNKNOWN' ELSE i.callback_required END, \
                     i.version = i.version + 1, i.updated_at = datetime(), \
                     i.report_count = coalesce(i.report_count, 1) + 1, r.dedup_rule = 'D-2' \
                 WITH r, i, coalesce(r.headcount, 0) > before AS increased, raise, \
                      raise AND coalesce(r.immediate_attention, false) AS alert \
                 OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) WHERE t.status IN ['QUEUED', 'PENDING_TRIGGER'] \
                 FOREACH (x IN CASE WHEN raise AND t IS NOT NULL THEN [t] ELSE [] END | \
                     SET x.urgency = i.urgency, x.priority_rank = i.priority_rank, x.version = x.version + 1) \
                 WITH r, i, increased, raise, alert, count(t) AS tasks \
                 CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                     event_id: $audit_id, aggregate_type: 'Incident', aggregate_id: i.incident_id, \
                     action: 'REPORT_ATTACHED', actor_id: $actor_id, previous_version: i.version - 1, \
                     new_version: i.version, reason: 'same-device follow-up (D-2): report ' + r.report_id, \
                     occurred_at: datetime(), \
                     details_json: '{\"urgency_raised\":' + toString(raise) + \
                         ',\"urgency\":\"' + coalesce(i.urgency, 'UNKNOWN') + '\"}' }) \
                 CREATE (:OutboxEvent { \
                     event_id: $outbox_id, aggregate_id: i.incident_id, type: 'INCIDENT_UPDATED', \
                     payload_json: '{\"incident_id\":\"' + i.incident_id + '\",\"report_id\":\"' + r.report_id + \
                         '\",\"headcount_increased\":' + toString(increased) + \
                         ',\"urgency_raised\":' + toString(raise) + \
                         ',\"urgency\":\"' + coalesce(i.urgency, 'UNKNOWN') + '\"}', \
                     created_at: datetime(), published: false, attempts: 0 }) \
                 FOREACH (_ IN CASE WHEN alert THEN [1] ELSE [] END | \
                     CREATE (:OutboxEvent { \
                         event_id: $alert_id, aggregate_id: i.incident_id, type: 'INCIDENT_CRITICAL', \
                         payload_json: '{\"incident_id\":\"' + i.incident_id + '\",\"report_id\":\"' + r.report_id + \
                             '\",\"urgency\":\"' + i.urgency + '\"}', \
                         created_at: datetime(), published: false, attempts: 0 })) \
                 RETURN increased, raise",
            )
            .param("report_id", report_id)
            .param("incident_id", incident_id.clone())
            .param("audit_id", new_id())
            .param("outbox_id", new_id())
            .param("alert_id", new_id())
            .param("actor_id", SYSTEM_ACTOR),
        )
        .await?
        .ok_or_else(|| OpsRepoError::NotFound(format!("incident {incident_id}")))?;
        return Ok(TriageCommit::Attached {
            incident_id,
            headcount_increased: attached.get("increased")?,
            urgency_raised: attached.get("raise")?,
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
                 required_asset: r.required_asset, suggested_asset: r.suggested_asset, \
                 location: r.location, location_source: r.location_source, \
                 caller_location: r.caller_location, \
                 junction_id: r.resolved_junction_id, device_id: r.device_id, \
                 review_reasons: coalesce(r.review_reasons, []), report_count: 1, \
                 urgency: coalesce(r.urgency, 'UNKNOWN'), rule_urgency: coalesce(r.rule_urgency, 'UNKNOWN'), \
                 review_priority: CASE WHEN r.urgency IS NULL THEN 'HIGH' ELSE r.review_priority END, \
                 review_rank: CASE WHEN r.urgency IS NULL THEN 2 ELSE r.review_rank END, \
                 urgency_flags: coalesce(r.urgency_flags, []), \
                 urgency_rules_version: r.urgency_rules_version, \
                 priority_rank: coalesce(r.priority_rank, 1), \
                 callback_required: coalesce(r.urgency, 'UNKNOWN') = 'UNKNOWN', \
                 created_at: datetime(), updated_at: datetime() }) \
             CREATE (r)-[:REPORTS {kind: 'PRIMARY', attached_at: datetime(), rule: 'NEW'}]->(i) \
             SET r.dedup_rule = 'NEW' \
             FOREACH (_ IN CASE WHEN j IS NULL THEN [] ELSE [1] END | CREATE (i)-[:LOCATED_AT]->(j)) \
             FOREACH (_ IN CASE WHEN $task_id IS NULL THEN [] ELSE [1] END | \
                 CREATE (i)-[:HAS_TASK {seq: 1}]->(:Task { \
                     task_id: $task_id, incident_id: $incident_id, kind: 'ROAD_PICKUP', status: 'QUEUED', \
                     required_asset: r.required_asset, urgency: i.urgency, priority_rank: i.priority_rank, \
                     version: 0, created_at: datetime(), queued_at: datetime() })) \
             CREATE (i)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Incident', aggregate_id: $incident_id, \
                 action: 'INCIDENT_CREATED', actor_id: $actor_id, to_state: $status, new_version: 0, \
                 reason: 'triaged report ' + r.report_id, occurred_at: datetime(), details_json: $payload }) \
             CREATE (:OutboxEvent { \
                 event_id: $outbox_id, aggregate_id: $incident_id, type: $event_type, \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             FOREACH (_ IN CASE WHEN $immediate THEN [1] ELSE [] END | \
                 CREATE (:OutboxEvent { \
                     event_id: $alert_id, aggregate_id: $incident_id, type: 'INCIDENT_CRITICAL', \
                     payload_json: $payload, created_at: datetime(), published: false, attempts: 0 })) \
             RETURN i.incident_id AS incident_id",
        )
        .param("report_id", report_id)
        .param("incident_id", incident_id.clone())
        .param("status", status)
        .param("event_type", event_type)
        .param("task_id", task_id.clone())
        .param("audit_id", new_id())
        .param("outbox_id", new_id())
        .param("alert_id", new_id())
        .param("immediate", immediate)
        .param("actor_id", SYSTEM_ACTOR)
        .param(
            "payload",
            json!({
                "incident_id": incident_id, "report_id": report_id, "task_id": task_id, "status": status,
                "urgency": urgency, "urgency_flags": flags, "review_priority": review_priority,
            })
            .to_string(),
        ),
    )
    .await?;
    Ok(TriageCommit::Created { incident_id, task_id, alerted: immediate })
}

/// Execute inside a transaction and return the first row, draining the rest
/// so the connection is ready for the next statement.
async fn txn_first_row(txn: &mut Txn, q: Query) -> Result<Option<Row>, OpsRepoError> {
    let mut stream = txn.execute(q).await?;
    let first = stream.next(txn.handle()).await?;
    while stream.next(txn.handle()).await?.is_some() {}
    Ok(first)
}
