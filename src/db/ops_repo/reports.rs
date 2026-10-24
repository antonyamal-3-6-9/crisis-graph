use neo4rs::{query, Row};
use serde_json::json;

use super::{new_id, parse_enum, parse_time, OpsRepoError, OpsRepository};
use crate::models::{NewReport, Report, ReportExtractionStatus};

pub(super) const REPORT_FIELDS: &str = "r.report_id AS report_id, r.request_id AS request_id, \
     r.device_id AS device_id, r.channel AS channel, r.raw_text AS raw_text, \
     r.latitude AS latitude, r.longitude AS longitude, r.accuracy_m AS accuracy_m, \
     toString(r.received_at) AS received_at, r.extraction_status AS extraction_status";

impl OpsRepository {
    /// Save a new report and its `REPORT_RECEIVED` outbox event in one
    /// statement. Keyed by `request_id` (rule D-1): a retry with the same
    /// `request_id` returns the original report with `created = false` and
    /// writes nothing.
    pub async fn create_report(&self, report: &NewReport) -> Result<(Report, bool), OpsRepoError> {
        let report_id = new_id();
        let q = query(&format!(
            "MERGE (r:Report {{request_id: $request_id}}) \
             ON CREATE SET r.report_id = $report_id, r.device_id = $device_id, r.channel = $channel, \
                 r.raw_text = $raw_text, r.latitude = $latitude, r.longitude = $longitude, \
                 r.accuracy_m = $accuracy_m, r.received_at = datetime(), \
                 r.extraction_status = 'PENDING', r._created = true \
             WITH r, coalesce(r._created, false) AS created \
             REMOVE r._created \
             FOREACH (_ IN CASE WHEN created THEN [1] ELSE [] END | \
                 CREATE (:OutboxEvent {{ \
                     event_id: $outbox_id, aggregate_id: r.report_id, type: 'REPORT_RECEIVED', \
                     payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }})) \
             RETURN created, {REPORT_FIELDS}"
        ))
        .param("request_id", report.request_id.clone())
        .param("report_id", report_id.clone())
        .param("device_id", report.device_id.clone())
        .param("channel", report.channel.clone())
        .param("raw_text", report.raw_text.clone())
        .param("latitude", report.latitude)
        .param("longitude", report.longitude)
        .param("accuracy_m", report.accuracy_m)
        .param("outbox_id", new_id())
        .param("payload", json!({ "report_id": report_id }).to_string());

        let row = self
            .write_first_row(q)
            .await?
            .ok_or_else(|| OpsRepoError::Database("report write returned no row".into()))?;
        let created = row.get::<bool>("created")?;
        Ok((report_from_row(&row)?, created))
    }

    pub async fn get_report(&self, report_id: &str) -> Result<Option<Report>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (r:Report {{report_id: $report_id}}) RETURN {REPORT_FIELDS}"
        ))
        .param("report_id", report_id);
        let mut result = self.neo4j.graph.execute(q).await?;
        match result.next().await? {
            Some(row) => Ok(Some(report_from_row(&row)?)),
            None => Ok(None),
        }
    }
}

pub(super) fn report_from_row(row: &Row) -> Result<Report, OpsRepoError> {
    Ok(Report {
        report_id: row.get("report_id")?,
        request_id: row.get("request_id")?,
        device_id: row.get("device_id")?,
        channel: row.get("channel")?,
        raw_text: row.get("raw_text")?,
        latitude: row.get("latitude")?,
        longitude: row.get("longitude")?,
        accuracy_m: row.get("accuracy_m")?,
        received_at: parse_time(&row.get::<String>("received_at")?)?,
        extraction_status: parse_enum(
            &row.get::<String>("extraction_status")?,
            ReportExtractionStatus::parse,
            "extraction status",
        )?,
    })
}
