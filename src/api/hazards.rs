//! Hazard command API (M4, transaction T7), on the `api` role.
//!
//! - `POST /api/v1/hazards`: close a segment (`HAZARD_VERIFIER` or `EOC_ADMIN`)
//! - `POST /api/v1/hazards/clear`: reopen a segment (same roles)
//! - `GET  /api/v1/hazards`: active hazards and the overlay version (any
//!   operator role)
//!
//! Callers authenticate with an operator token ([`OperatorTokens`]). Segment
//! IDs contain `/`, so they travel in the body or query, not the path.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::json;

use super::auth::OperatorTokens;
use super::error_status;
use crate::db::{OpsRepoError, OpsRepository};
use crate::models::{ActorContext, HazardCommand, HazardOutcome, HazardRecord, HazardSource, HazardStatus, Permission};

const MAX_DURATION_HOURS: i64 = 168;

#[derive(Clone)]
pub struct HazardApiState {
    pub repo: OpsRepository,
    pub operators: Arc<OperatorTokens>,
    /// Dataset used when a request does not name one.
    pub default_dataset: String,
}

pub fn hazard_routes(state: HazardApiState) -> Router {
    Router::new()
        .route("/api/v1/hazards", post(apply).get(list))
        .route("/api/v1/hazards/clear", post(clear))
        .with_state(state)
}

#[derive(Debug, Default, Deserialize)]
pub struct ApplyRequest {
    pub request_id: Option<String>,
    pub dataset: Option<String>,
    pub segment_id: Option<String>,
    pub status: Option<String>,
    /// Either `duration_hours` (1..=168) or `valid_until`; neither means in
    /// force until cleared.
    pub duration_hours: Option<i64>,
    pub valid_until: Option<DateTime<Utc>>,
    pub reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ClearRequest {
    pub dataset: Option<String>,
    pub segment_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    pub dataset: Option<String>,
}

/// Validate an apply request into a command, or explain what is wrong.
pub fn validate_apply(request: ApplyRequest, default_dataset: &str, now: DateTime<Utc>) -> Result<HazardCommand, String> {
    let request_id = non_empty(request.request_id, "request_id")?;
    let segment_id = non_empty(request.segment_id, "segment_id")?;
    let reason = non_empty(request.reason, "reason")?;
    let status = request.status.ok_or("status is required (FLOODED or BLOCKED)")?;
    let status = HazardStatus::parse(&status).ok_or("status must be FLOODED or BLOCKED; use /api/v1/hazards/clear to reopen")?;
    let valid_until = match (request.duration_hours, request.valid_until) {
        (Some(_), Some(_)) => return Err("give duration_hours or valid_until, not both".into()),
        (Some(hours), None) if !(1..=MAX_DURATION_HOURS).contains(&hours) => {
            return Err(format!("duration_hours must be 1..={MAX_DURATION_HOURS}"))
        }
        (Some(hours), None) => Some(now + Duration::hours(hours)),
        (None, Some(until)) if until <= now => return Err("valid_until must be in the future".into()),
        (None, until) => until,
    };
    Ok(HazardCommand {
        request_id,
        dataset: request.dataset.unwrap_or_else(|| default_dataset.to_string()),
        segment_id,
        status,
        valid_until,
        reason,
        source: HazardSource::Operator,
    })
}

fn non_empty(value: Option<String>, name: &str) -> Result<String, String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{name} is required"))
}

/// Why a request was refused before reaching the command.
struct Refused(StatusCode, &'static str);

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

/// The authenticated operator, or a 401/403 refusal.
fn operator(state: &HazardApiState, headers: &HeaderMap, permission: Permission) -> Result<ActorContext, Refused> {
    let actor = state
        .operators
        .authenticate(headers)
        .ok_or(Refused(StatusCode::UNAUTHORIZED, "operator token required"))?;
    if !actor.can(permission) {
        return Err(Refused(StatusCode::FORBIDDEN, "role not allowed"));
    }
    Ok(actor)
}

async fn apply(State(state): State<HazardApiState>, headers: HeaderMap, body: Option<Json<ApplyRequest>>) -> Response {
    let actor = match operator(&state, &headers, Permission::VerifyHazard) {
        Ok(actor) => actor,
        Err(refused) => return refused.into_response(),
    };
    let Some(Json(request)) = body else {
        return bad_request("request body must be JSON");
    };
    let command = match validate_apply(request, &state.default_dataset, Utc::now()) {
        Ok(command) => command,
        Err(message) => return bad_request(&message),
    };
    match state.repo.apply_hazard(&actor, &command).await {
        Ok(HazardOutcome::Applied { hazard, overlay_version, replaced }) => (
            StatusCode::CREATED,
            Json(json!({ "hazard": hazard_body(&hazard), "overlay_version": overlay_version, "replaced": replaced })),
        )
            .into_response(),
        Ok(HazardOutcome::Replayed { hazard }) => {
            let mut response = (StatusCode::OK, Json(json!({ "hazard": hazard_body(&hazard) }))).into_response();
            response.headers_mut().insert("idempotent-replayed", HeaderValue::from_static("true"));
            response
        }
        Ok(other) => repo_error(&OpsRepoError::Database(format!("unexpected outcome {other:?}"))),
        Err(error) => repo_error(&error),
    }
}

async fn clear(State(state): State<HazardApiState>, headers: HeaderMap, body: Option<Json<ClearRequest>>) -> Response {
    let actor = match operator(&state, &headers, Permission::VerifyHazard) {
        Ok(actor) => actor,
        Err(refused) => return refused.into_response(),
    };
    let Some(Json(request)) = body else {
        return bad_request("request body must be JSON");
    };
    let (segment_id, reason) = match (non_empty(request.segment_id, "segment_id"), non_empty(request.reason, "reason")) {
        (Ok(segment), Ok(reason)) => (segment, reason),
        (Err(message), _) | (_, Err(message)) => return bad_request(&message),
    };
    let dataset = request.dataset.unwrap_or_else(|| state.default_dataset.clone());
    match state.repo.clear_hazard(&actor, &dataset, &segment_id, &reason).await {
        Ok(HazardOutcome::Cleared { hazard_id, overlay_version }) => (
            StatusCode::OK,
            Json(json!({ "cleared": true, "hazard_id": hazard_id, "overlay_version": overlay_version })),
        )
            .into_response(),
        Ok(HazardOutcome::NothingToClear) => {
            (StatusCode::OK, Json(json!({ "cleared": false, "segment_id": segment_id }))).into_response()
        }
        Ok(other) => repo_error(&OpsRepoError::Database(format!("unexpected outcome {other:?}"))),
        Err(error) => repo_error(&error),
    }
}

async fn list(State(state): State<HazardApiState>, headers: HeaderMap, Query(query): Query<ListQuery>) -> Response {
    if let Err(refused) = operator(&state, &headers, Permission::ReadOperations) {
        return refused.into_response();
    }
    let dataset = query.dataset.unwrap_or_else(|| state.default_dataset.clone());
    let (version, hazards) = match tokio::try_join!(state.repo.overlay_version(&dataset), state.repo.active_hazards(&dataset)) {
        Ok(result) => result,
        Err(error) => return repo_error(&error),
    };
    let now = Utc::now();
    let hazards: Vec<_> = hazards
        .iter()
        .map(|h| {
            let mut body = hazard_body(h);
            body["in_force"] = json!(h.in_force_at(now));
            body
        })
        .collect();
    Json(json!({ "dataset": dataset, "overlay_version": version, "hazards": hazards })).into_response()
}

fn hazard_body(hazard: &HazardRecord) -> serde_json::Value {
    json!({
        "hazard_id": hazard.hazard_id,
        "request_id": hazard.request_id,
        "dataset": hazard.dataset,
        "segment_id": hazard.segment_id,
        "status": hazard.status.as_str(),
        "valid_from": hazard.valid_from.to_rfc3339(),
        "valid_until": hazard.valid_until.map(|t| t.to_rfc3339()),
        "actor_id": hazard.actor_id,
        "reason": hazard.reason,
        "source": hazard.source.as_str(),
        "overlay_version": hazard.overlay_version,
    })
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

fn repo_error(error: &OpsRepoError) -> Response {
    // A bad command (e.g. reason too long) is the caller's mistake here.
    let status = match error {
        OpsRepoError::InvalidData(_) => StatusCode::BAD_REQUEST,
        other => error_status(other),
    };
    let mut response = (status, Json(json!({ "error": error.to_string() }))).into_response();
    if status == StatusCode::SERVICE_UNAVAILABLE {
        response.headers_mut().insert("retry-after", HeaderValue::from_static("2"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ApplyRequest {
        ApplyRequest {
            request_id: Some("r1".into()),
            segment_id: Some("way/1/seg/0/fwd".into()),
            status: Some("FLOODED".into()),
            reason: Some("field report".into()),
            ..Default::default()
        }
    }

    #[test]
    fn valid_requests_become_commands() {
        let now = Utc::now();
        let command = validate_apply(ApplyRequest { duration_hours: Some(6), ..request() }, "pilot", now).unwrap();
        assert_eq!(command.dataset, "pilot");
        assert_eq!(command.valid_until, Some(now + Duration::hours(6)));
        let open_ended = validate_apply(request(), "pilot", now).unwrap();
        assert_eq!(open_ended.valid_until, None, "no expiry means until cleared");
    }

    #[test]
    fn invalid_requests_are_explained() {
        let now = Utc::now();
        let cases = [
            ApplyRequest { status: Some("OPEN".into()), ..request() },
            ApplyRequest { duration_hours: Some(0), ..request() },
            ApplyRequest { duration_hours: Some(1), valid_until: Some(now + Duration::hours(1)), ..request() },
            ApplyRequest { valid_until: Some(now - Duration::hours(1)), ..request() },
            ApplyRequest { reason: Some("  ".into()), ..request() },
            ApplyRequest { request_id: None, ..request() },
        ];
        for case in cases {
            assert!(validate_apply(case, "pilot", now).is_err());
        }
    }
}
