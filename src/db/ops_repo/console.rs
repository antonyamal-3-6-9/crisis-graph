//! Console read model (M7): what the operations console shows. Read-only;
//! every query returns plain JSON (times as RFC 3339 strings, positions as
//! `{lat, lon}`), so the API passes it through unchanged.

use neo4rs::query;
use serde_json::{json, Value};

use super::{OpsRepoError, OpsRepository};
use crate::models::display_status;

/// The incident fields every list and detail view needs.
const INCIDENT_FIELDS: &str = "{ incident_id: i.incident_id, status: i.status, version: i.version, \
     urgency: i.urgency, rule_urgency: i.rule_urgency, urgency_flags: coalesce(i.urgency_flags, []), \
     review_priority: i.review_priority, review_reasons: coalesce(i.review_reasons, []), \
     headcount: i.headcount, required_asset: i.required_asset, suggested_asset: i.suggested_asset, \
     junction_id: i.junction_id, location_source: i.location_source, \
     location: CASE WHEN i.location IS NULL THEN null \
         ELSE {lat: i.location.latitude, lon: i.location.longitude} END, \
     caller_location: CASE WHEN i.caller_location IS NULL THEN null \
         ELSE {lat: i.caller_location.latitude, lon: i.caller_location.longitude} END, \
     report_count: coalesce(i.report_count, 1), callback_required: coalesce(i.callback_required, false), \
     created_at: toString(i.created_at), updated_at: toString(i.updated_at) }";

const TASK_FIELDS: &str = "{ task_id: t.task_id, kind: t.kind, status: t.status, version: t.version, \
     required_asset: t.required_asset, urgency: t.urgency, priority_rank: t.priority_rank, \
     assigned_vehicle_id: t.assigned_vehicle_id, eta_s: t.eta_s, overlay_version: t.overlay_version, \
     verify_failures: coalesce(t.verify_failures, 0), escalations: coalesce(t.escalations, []), \
     queued_at: toString(t.queued_at), assigned_at: toString(t.assigned_at), \
     completed_at: toString(t.completed_at) }";

impl OpsRepository {
    /// Recent incidents, newest first, with their display status. `status`
    /// filters on the display status.
    pub async fn console_incidents(&self, status: Option<&str>, limit: usize) -> Result<Vec<Value>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (i:OpsIncident) \
             WITH i ORDER BY i.created_at DESC LIMIT $scan \
             OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) \
             WITH i, collect(t.status) AS task_statuses, \
                  [x IN collect(t) WHERE x.status = 'ASSIGNED' | x.assigned_vehicle_id] AS vehicles \
             OPTIONAL MATCH (i)-[d:POSSIBLE_DUPLICATE]-(:OpsIncident) \
             WITH i, task_statuses, vehicles, count(d) AS duplicates \
             RETURN {INCIDENT_FIELDS} AS incident, task_statuses, vehicles, duplicates \
             ORDER BY i.created_at DESC"
        ))
        .param("scan", (limit.max(1) * 4) as i64);
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut incidents = Vec::new();
        while let Some(row) = result.next().await? {
            let mut incident: Value = row.get("incident")?;
            let statuses: Vec<String> = row.get("task_statuses")?;
            let shown = display_status(incident["status"].as_str().unwrap_or_default(), &statuses);
            if status.is_some_and(|wanted| wanted != shown) {
                continue;
            }
            incident["display_status"] = json!(shown);
            incident["assigned_vehicles"] = row.get::<Value>("vehicles")?;
            incident["possible_duplicates"] = json!(row.get::<i64>("duplicates")?);
            incidents.push(incident);
            if incidents.len() >= limit {
                break;
            }
        }
        Ok(incidents)
    }

    /// One incident with its reports, tasks, reservations, possible
    /// duplicates and audit timeline.
    pub async fn console_incident(&self, incident_id: &str) -> Result<Option<Value>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (i:OpsIncident {{incident_id: $id}}) \
             CALL {{ WITH i \
                 OPTIONAL MATCH (r:Report)-[link:REPORTS]->(i) \
                 WITH r, link ORDER BY r.received_at \
                 RETURN collect(CASE WHEN r IS NULL THEN null ELSE {{ report_id: r.report_id, \
                     kind: link.kind, rule: link.rule, device_id: r.device_id, channel: r.channel, \
                     raw_text: r.raw_text, received_at: toString(r.received_at), \
                     extraction_status: r.extraction_status, extraction_json: r.extraction_json, \
                     urgency: r.urgency, urgency_flags: coalesce(r.urgency_flags, []), \
                     location_source: r.location_source, review_reasons: coalesce(r.review_reasons, []), \
                     latitude: r.latitude, longitude: r.longitude, accuracy_m: r.accuracy_m }} END) AS reports }} \
             CALL {{ WITH i \
                 OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) \
                 WITH t ORDER BY t.created_at \
                 RETURN collect(CASE WHEN t IS NULL THEN null ELSE {TASK_FIELDS} END) AS tasks, \
                        collect(t.status) AS task_statuses }} \
             CALL {{ WITH i \
                 OPTIONAL MATCH (res:Reservation {{incident_id: i.incident_id}}) \
                 WITH res ORDER BY res.created_at \
                 RETURN collect(CASE WHEN res IS NULL THEN null ELSE {{ reservation_id: res.reservation_id, \
                     task_id: res.task_id, kind: res.resource_kind, status: res.status, \
                     vehicle_id: res.vehicle_id, shelter_id: res.shelter_id, beds: res.beds, \
                     created_at: toString(res.created_at), released_at: toString(res.released_at), \
                     release_reason: res.release_reason }} END) AS reservations }} \
             CALL {{ WITH i \
                 OPTIONAL MATCH (i)-[d:POSSIBLE_DUPLICATE]-(o:OpsIncident) \
                 RETURN collect(CASE WHEN o IS NULL THEN null ELSE {{ incident_id: o.incident_id, \
                     status: o.status, distance_m: d.distance_m }} END) AS duplicates }} \
             CALL {{ WITH i \
                 OPTIONAL MATCH (i)-[:HAS_TASK]->(tt:Task) \
                 WITH i, collect(tt.task_id) AS task_ids \
                 OPTIONAL MATCH (a:AuditEvent) \
                 WHERE a.aggregate_id IN [i.incident_id] + task_ids \
                 WITH a ORDER BY a.occurred_at \
                 RETURN collect(CASE WHEN a IS NULL THEN null ELSE {{ action: a.action, actor_id: a.actor_id, \
                     aggregate_type: a.aggregate_type, aggregate_id: a.aggregate_id, from_state: a.from_state, \
                     to_state: a.to_state, reason: a.reason, occurred_at: toString(a.occurred_at) }} END) AS audit }} \
             RETURN {INCIDENT_FIELDS} AS incident, reports, tasks, task_statuses, reservations, duplicates, audit"
        ))
        .param("id", incident_id);
        let mut result = self.neo4j.graph.execute(q).await?;
        let Some(row) = result.next().await? else {
            return Ok(None);
        };
        let mut incident: Value = row.get("incident")?;
        let statuses: Vec<String> = row.get("task_statuses")?;
        incident["display_status"] = json!(display_status(incident["status"].as_str().unwrap_or_default(), &statuses));
        for key in ["reports", "tasks", "reservations", "duplicates", "audit"] {
            incident[key] = row.get::<Value>(key)?;
        }
        Ok(Some(incident))
    }

    /// The incident's current route (assigned or completed task), with the
    /// pickup and drop-off junctions: `(route_json, pickup, dropoff)`.
    pub async fn console_route(&self, incident_id: &str) -> Result<Option<(String, Option<String>, Option<String>)>, OpsRepoError> {
        let q = query(
            "MATCH (i:OpsIncident {incident_id: $id})-[:HAS_TASK]->(t:Task) \
             WHERE t.route_json IS NOT NULL \
             WITH i, t ORDER BY t.assigned_at DESC LIMIT 1 \
             OPTIONAL MATCH (b:Reservation {task_id: t.task_id, resource_kind: 'BEDS'})-[:AT_SHELTER]->(s:Shelter) \
             RETURN t.route_json AS route, i.junction_id AS pickup, head(collect(s.junction_id)) AS dropoff",
        )
        .param("id", incident_id);
        let mut result = self.neo4j.graph.execute(q).await?;
        match result.next().await? {
            Some(row) => Ok(Some((row.get("route")?, row.get("pickup")?, row.get("dropoff")?))),
            None => Ok(None),
        }
    }

    /// Every vehicle in the dataset with its position and current task.
    pub async fn console_vehicles(&self, dataset: &str) -> Result<Vec<Value>, OpsRepoError> {
        let q = query(
            "MATCH (v:Vehicle)-[:BASED_AT]->(home:Shelter {dataset: $dataset}) \
             OPTIONAL MATCH (j:Junction {id: v.junction_id, dataset: $dataset}) \
             OPTIONAL MATCH (r:Reservation {active_vehicle_key: v.vehicle_id}) \
             RETURN { vehicle_id: v.vehicle_id, asset_type: v.asset_type, status: v.status, \
                 version: v.version, capacity_persons: v.capacity_persons, home_shelter_id: home.id, \
                 junction_id: v.junction_id, \
                 position: CASE WHEN j IS NULL THEN null ELSE {lat: j.lat, lon: j.lon} END, \
                 task_id: r.task_id, incident_id: r.incident_id, updated_at: toString(v.updated_at) } AS vehicle \
             ORDER BY v.vehicle_id",
        )
        .param("dataset", dataset);
        self.json_rows(q, "vehicle").await
    }

    /// Shelters in the dataset with beds and position.
    pub async fn console_shelters(&self, dataset: &str) -> Result<Vec<Value>, OpsRepoError> {
        let q = query(
            "MATCH (s:Shelter {dataset: $dataset}) \
             OPTIONAL MATCH (j:Junction {id: s.junction_id, dataset: $dataset}) \
             OPTIONAL MATCH (v:Vehicle)-[:BASED_AT]->(s) \
             WITH s, j, count(v) AS fleet, sum(CASE WHEN v.status = 'AVAILABLE' THEN 1 ELSE 0 END) AS available \
             RETURN { shelter_id: s.id, name: s.name, junction_id: s.junction_id, \
                 capacity: coalesce(s.capacity, 0), occupancy: coalesce(s.current_occupancy, 0), \
                 position: CASE WHEN j IS NULL THEN null ELSE {lat: j.lat, lon: j.lon} END, \
                 fleet: fleet, available: available } AS shelter \
             ORDER BY s.id",
        )
        .param("dataset", dataset);
        self.json_rows(q, "shelter").await
    }

    /// Escalated tasks still waiting, most urgent first.
    pub async fn console_escalations(&self, limit: usize) -> Result<Vec<Value>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (i:OpsIncident)-[:HAS_TASK]->(t:Task {{status: 'QUEUED'}}) \
             WHERE size(coalesce(t.escalations, [])) > 0 \
             WITH i, t ORDER BY t.priority_rank DESC, t.escalated_at ASC LIMIT $limit \
             WITH i, t, {TASK_FIELDS} AS base \
             RETURN base {{ .*, incident_id: i.incident_id, escalated_at: toString(t.escalated_at) }} AS task"
        ))
        .param("limit", limit as i64);
        self.json_rows(q, "task").await
    }

    /// Recent alert events from the outbox (kept for the retention period).
    pub async fn console_alert_events(&self, limit: usize) -> Result<Vec<Value>, OpsRepoError> {
        let q = query(
            "MATCH (e:OutboxEvent) \
             WHERE e.type IN ['INCIDENT_CRITICAL', 'DROPOFF_UNRESOLVED', 'TASK_ESCALATED', 'INCIDENT_REVIEW_REQUIRED'] \
             WITH e ORDER BY e.created_at DESC LIMIT $limit \
             RETURN { event_id: e.event_id, type: e.type, aggregate_id: e.aggregate_id, \
                 payload_json: e.payload_json, created_at: toString(e.created_at) } AS event",
        )
        .param("limit", limit as i64);
        self.json_rows(q, "event").await
    }

    /// The report's incident and that incident's display status, if triage
    /// has linked it yet.
    pub async fn report_incident(&self, report_id: &str) -> Result<Option<(String, String)>, OpsRepoError> {
        let q = query(
            "MATCH (:Report {report_id: $id})-[:REPORTS]->(i:OpsIncident) \
             OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) \
             RETURN i.incident_id AS incident_id, i.status AS status, collect(t.status) AS task_statuses",
        )
        .param("id", report_id);
        let mut result = self.neo4j.graph.execute(q).await?;
        let Some(row) = result.next().await? else {
            return Ok(None);
        };
        let statuses: Vec<String> = row.get("task_statuses")?;
        let status: String = row.get("status")?;
        Ok(Some((row.get("incident_id")?, display_status(&status, &statuses).to_string())))
    }

    /// Directed segments starting or ending at `junction_id`, with their end
    /// coordinates, for choosing a segment on the map.
    pub async fn console_segments_at(&self, dataset: &str, junction_id: &str) -> Result<Vec<Value>, OpsRepoError> {
        let q = query(
            "MATCH (a:Junction {dataset: $dataset})-[r:CONNECTS_TO {dataset: $dataset}]->(b:Junction {dataset: $dataset}) \
             WHERE a.id = $junction OR b.id = $junction \
             OPTIONAL MATCH (h:Hazard {active_key: $dataset + ':' + r.segment_id}) \
             RETURN { segment_id: r.segment_id, road_name: r.road_name, road_class: r.road_class, \
                 from: {lat: a.lat, lon: a.lon}, to: {lat: b.lat, lon: b.lon}, length_m: r.length_m, \
                 hazard_status: h.status } AS segment \
             ORDER BY r.segment_id LIMIT 24",
        )
        .param("dataset", dataset)
        .param("junction", junction_id);
        self.json_rows(q, "segment").await
    }

    /// End coordinates of each segment, for drawing hazards on the map.
    pub async fn segment_ends(&self, dataset: &str, segment_ids: &[String]) -> Result<Vec<Value>, OpsRepoError> {
        let q = query(
            "UNWIND $ids AS id \
             MATCH (a:Junction)-[r:CONNECTS_TO {dataset: $dataset, segment_id: id}]->(b:Junction) \
             RETURN { segment_id: id, road_name: r.road_name, from: {lat: a.lat, lon: a.lon}, \
                 to: {lat: b.lat, lon: b.lon} } AS segment",
        )
        .param("dataset", dataset)
        .param("ids", segment_ids.to_vec());
        self.json_rows(q, "segment").await
    }

    async fn json_rows(&self, q: neo4rs::Query, column: &str) -> Result<Vec<Value>, OpsRepoError> {
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut rows = Vec::new();
        while let Some(row) = result.next().await? {
            rows.push(row.get::<Value>(column)?);
        }
        Ok(rows)
    }
}
