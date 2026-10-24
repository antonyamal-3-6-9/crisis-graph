//! Intake API for the new pipeline (M2).
//!
//! `POST /api/v1/reports` saves the report and its outbox event in one
//! statement and answers `202 Accepted` without doing any LLM or graph work.
//! `request_id` is required: it makes retries safe (rule D-1).

use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

use crate::db::{OpsRepoError, OpsRepository};
use crate::models::{NewReport, Report};

const MAX_TEXT_CHARS: usize = 4000;
const MAX_ID_CHARS: usize = 128;
const RETRY_AFTER_SECS: &str = "2";

#[derive(Clone)]
pub struct ApiState {
    pub repo: OpsRepository,
}

pub fn intake_routes(state: ApiState) -> Router {
    Router::new()
        .route("/api/v1/reports", post(create_report))
        .route("/api/v1/reports/{report_id}", get(get_report))
        .with_state(state)
}

/// Request body. Fields are optional here so that missing values produce a
/// specific 400 message instead of a generic deserialisation error.
#[derive(Debug, Default, Deserialize)]
pub struct ReportRequest {
    pub device_id: Option<String>,
    pub request_id: Option<String>,
    pub raw_text: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy_m: Option<f64>,
    pub channel: Option<String>,
}

/// Validate a request into a `NewReport`, or explain what is wrong.
pub fn validate(request: ReportRequest) -> Result<NewReport, String> {
    let request_id = required_id(request.request_id, "request_id")?;
    let device_id = required_id(request.device_id, "device_id")?;

    let raw_text = request
        .raw_text
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or("raw_text is required")?;
    if raw_text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("raw_text must be at most {MAX_TEXT_CHARS} characters"));
    }

    match (request.latitude, request.longitude) {
        (Some(lat), Some(lon)) => {
            if !(-90.0..=90.0).contains(&lat) || !lat.is_finite() {
                return Err("latitude must be between -90 and 90".into());
            }
            if !(-180.0..=180.0).contains(&lon) || !lon.is_finite() {
                return Err("longitude must be between -180 and 180".into());
            }
        }
        (None, None) => {}
        _ => return Err("latitude and longitude must be given together".into()),
    }
    if let Some(accuracy) = request.accuracy_m {
        if !accuracy.is_finite() || accuracy < 0.0 {
            return Err("accuracy_m must be a non-negative number".into());
        }
    }

    let channel = request
        .channel
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty() && c.len() <= MAX_ID_CHARS)
        .unwrap_or_else(|| "emergency_number".to_string());

    Ok(NewReport {
        request_id,
        device_id,
        channel,
        raw_text,
        latitude: request.latitude,
        longitude: request.longitude,
        accuracy_m: request.accuracy_m,
    })
}

fn required_id(value: Option<String>, field: &str) -> Result<String, String> {
    let value = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{field} is required"))?;
    if value.chars().count() > MAX_ID_CHARS {
        return Err(format!("{field} must be at most {MAX_ID_CHARS} characters"));
    }
    Ok(value)
}

/// HTTP status for a repository error. Database trouble is `503` with
/// `Retry-After`: the client retries with the same `request_id`, which is
/// safe, so nothing is duplicated.
pub fn error_status(error: &OpsRepoError) -> StatusCode {
    match error {
        OpsRepoError::Database(_) | OpsRepoError::Transient(_) => StatusCode::SERVICE_UNAVAILABLE,
        OpsRepoError::NotFound(_) => StatusCode::NOT_FOUND,
        OpsRepoError::Conflict(_) | OpsRepoError::NotAvailable(_) | OpsRepoError::Fenced(_) => {
            StatusCode::CONFLICT
        }
        OpsRepoError::InvalidData(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn error_response(error: &OpsRepoError) -> Response {
    let status = error_status(error);
    let mut headers = HeaderMap::new();
    if status == StatusCode::SERVICE_UNAVAILABLE {
        headers.insert("retry-after", HeaderValue::from_static(RETRY_AFTER_SECS));
    }
    let body = Json(json!({ "error": status.canonical_reason().unwrap_or("error") }));
    (status, headers, body).into_response()
}

fn report_body(report: &Report) -> serde_json::Value {
    json!({
        "report_id": report.report_id,
        "request_id": report.request_id,
        "status": report.extraction_status.as_str(),
        "received_at": report.received_at.to_rfc3339(),
        "status_url": format!("/api/v1/reports/{}", report.report_id),
    })
}

async fn create_report(State(state): State<ApiState>, body: Option<Json<ReportRequest>>) -> Response {
    let Some(Json(request)) = body else {
        record("invalid");
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "body must be a JSON object" })))
            .into_response();
    };
    let report = match validate(request) {
        Ok(report) => report,
        Err(reason) => {
            record("invalid");
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": reason }))).into_response();
        }
    };

    match state.repo.create_report(&report).await {
        Ok((saved, created)) => {
            record(if created { "created" } else { "replayed" });
            let mut headers = HeaderMap::new();
            if !created {
                headers.insert("idempotent-replayed", HeaderValue::from_static("true"));
            }
            (StatusCode::ACCEPTED, headers, Json(report_body(&saved))).into_response()
        }
        Err(error) => {
            record("unavailable");
            warn!(%error, request_id = %report.request_id, "Failed to save report");
            error_response(&error)
        }
    }
}

async fn get_report(State(state): State<ApiState>, Path(report_id): Path<String>) -> Response {
    match state.repo.get_report(&report_id).await {
        Ok(Some(report)) => (StatusCode::OK, Json(report_body(&report))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "report not found" }))).into_response(),
        Err(error) => error_response(&error),
    }
}

fn record(result: &'static str) {
    metrics::counter!("crisisgraph_reports_received_total", "result" => result).increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> ReportRequest {
        ReportRequest {
            device_id: Some("device-1".into()),
            request_id: Some("req-1".into()),
            raw_text: Some("Water entering house near Bank Junction, 4 people".into()),
            latitude: Some(10.1082),
            longitude: Some(76.3565),
            accuracy_m: Some(15.0),
            channel: None,
        }
    }

    #[test]
    fn accepts_a_complete_request() {
        let report = validate(valid()).unwrap();
        assert_eq!(report.request_id, "req-1");
        assert_eq!(report.channel, "emergency_number");
    }

    #[test]
    fn request_id_is_required() {
        let error = validate(ReportRequest { request_id: None, ..valid() }).unwrap_err();
        assert_eq!(error, "request_id is required");
        let error = validate(ReportRequest { request_id: Some("  ".into()), ..valid() }).unwrap_err();
        assert_eq!(error, "request_id is required");
    }

    #[test]
    fn rejects_bad_location_and_text() {
        assert!(validate(ReportRequest { latitude: Some(200.0), ..valid() }).is_err());
        assert!(validate(ReportRequest { longitude: None, ..valid() }).is_err());
        assert!(validate(ReportRequest { accuracy_m: Some(-1.0), ..valid() }).is_err());
        assert!(validate(ReportRequest { raw_text: Some(" ".into()), ..valid() }).is_err());
        assert!(validate(ReportRequest { raw_text: Some("x".repeat(4001)), ..valid() }).is_err());
    }

    #[test]
    fn location_is_optional_but_paired() {
        let report = validate(ReportRequest { latitude: None, longitude: None, accuracy_m: None, ..valid() });
        assert!(report.is_ok());
    }

    #[test]
    fn database_errors_map_to_503() {
        assert_eq!(error_status(&OpsRepoError::Database("down".into())), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_status(&OpsRepoError::Transient("deadlock".into())), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_status(&OpsRepoError::NotFound("x".into())), StatusCode::NOT_FOUND);
    }

    #[test]
    fn unavailable_response_carries_retry_after() {
        let response = error_response(&OpsRepoError::Database("down".into()));
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers().get("retry-after").unwrap(), RETRY_AFTER_SECS);
    }
}
