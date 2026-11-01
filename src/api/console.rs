//! Console API (M7) on the `api` role: read-only views of the new pipeline,
//! the dispatcher commands (review, task complete) and submit-and-wait
//! dispatch.
//!
//! Reads are open, like intake (owner decision 2026-11-01: the pilot runs
//! locally). Every command needs an operator token whose role allows it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use redis::AsyncCommands;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;

use super::auth::{authorize, OperatorTokens};
use super::{error_status, validate, ReportRequest};
use crate::config::PipelineParams;
use crate::db::{
    CompleteTaskCommand, MergeCommand, OpsRepoError, OpsRepository, RedisClient, ReviewCommand, ReviewVerdict,
    SplitCommand, StateManager, VehicleStatusCommand,
};
use crate::matcher::{effective_tier, lease_name, Tier};
use crate::models::{AssetType, Permission, Urgency};
use crate::relay::RELAY_LEASE;
use crate::solver::RouteResult;

/// Radius for snapping a clicked point to a junction.
const SNAP_RADIUS_M: f64 = 300.0;
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;

#[derive(Clone)]
pub struct ConsoleApiState {
    pub repo: OpsRepository,
    pub state: StateManager,
    pub redis: RedisClient,
    pub operators: Arc<OperatorTokens>,
    pub params: PipelineParams,
}

impl ConsoleApiState {
    fn dataset(&self) -> &str {
        &self.params.graph_dataset
    }
}

pub fn console_routes(state: ConsoleApiState) -> Router {
    Router::new()
        .route("/api/v1/incidents", get(list_incidents))
        .route("/api/v1/incidents/{incident_id}", get(get_incident))
        .route("/api/v1/incidents/{incident_id}/route", get(get_route))
        .route("/api/v1/incidents/{incident_id}/review", post(review))
        .route("/api/v1/incidents/{incident_id}/merge", post(merge))
        .route("/api/v1/incidents/{incident_id}/split", post(split))
        .route("/api/v1/tasks/{task_id}/complete", post(complete))
        .route("/api/v1/vehicles/{vehicle_id}/status", post(vehicle_status))
        .route("/api/v1/vehicles", get(vehicles))
        .route("/api/v1/shelters", get(shelters))
        .route("/api/v1/queue", get(queue))
        .route("/api/v1/alerts", get(alerts))
        .route("/api/v1/hazards/active", get(active_hazards))
        .route("/api/v1/segments/near", get(segments_near))
        .route("/api/v1/system", get(system))
        .route("/api/v1/dispatch", post(dispatch))
        .with_state(state)
}

// ---------------------------------------------------------------- reads

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub limit: Option<usize>,
}

async fn list_incidents(State(state): State<ConsoleApiState>, Query(query): Query<ListQuery>) -> Response {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    match state.repo.console_incidents(query.status.as_deref(), limit).await {
        Ok(incidents) => Json(json!({ "incidents": incidents })).into_response(),
        Err(error) => repo_error(&error),
    }
}

async fn get_incident(State(state): State<ConsoleApiState>, Path(incident_id): Path<String>) -> Response {
    match state.repo.console_incident(&incident_id).await {
        Ok(Some(incident)) => Json(incident).into_response(),
        Ok(None) => not_found("incident not found"),
        Err(error) => repo_error(&error),
    }
}

async fn get_route(State(state): State<ConsoleApiState>, Path(incident_id): Path<String>) -> Response {
    match route_geojson(&state, &incident_id).await {
        Ok(Some(geojson)) => Json(geojson).into_response(),
        Ok(None) => not_found("no route recorded for this incident"),
        Err(error) => internal(error),
    }
}

/// The incident's route as GeoJSON: the line, plus pickup and drop-off points.
async fn route_geojson(state: &ConsoleApiState, incident_id: &str) -> Result<Option<Value>, Box<dyn std::error::Error + Send + Sync>> {
    let Some((route_json, pickup, dropoff)) = state.repo.console_route(incident_id).await? else {
        return Ok(None);
    };
    let route: RouteResult = serde_json::from_str(&route_json)?;
    let line = state.state.get_junction_coordinates(&route.path).await?;
    let mut features = vec![json!({
        "type": "Feature",
        "geometry": { "type": "LineString", "coordinates": line },
        "properties": { "kind": "route", "distance_km": route.total_distance_km,
                        "travel_time_s": route.total_travel_time_s, "segments": route.segment_path.len() },
    })];
    for (kind, junction) in [("pickup", pickup), ("dropoff", dropoff)] {
        if let Some(junction) = junction {
            if let Some(point) = state.state.get_junction_coordinates(std::slice::from_ref(&junction)).await?.first() {
                features.push(json!({
                    "type": "Feature",
                    "geometry": { "type": "Point", "coordinates": point },
                    "properties": { "kind": kind, "junction_id": junction },
                }));
            }
        }
    }
    Ok(Some(json!({ "type": "FeatureCollection", "features": features })))
}

async fn vehicles(State(state): State<ConsoleApiState>) -> Response {
    match state.repo.console_vehicles(state.dataset()).await {
        Ok(vehicles) => Json(json!({ "vehicles": vehicles })).into_response(),
        Err(error) => repo_error(&error),
    }
}

async fn shelters(State(state): State<ConsoleApiState>) -> Response {
    match state.repo.console_shelters(state.dataset()).await {
        Ok(shelters) => Json(json!({ "shelters": shelters })).into_response(),
        Err(error) => repo_error(&error),
    }
}

/// Queued tasks grouped by the tier the matcher uses, with waiting times.
async fn queue(State(state): State<ConsoleApiState>) -> Response {
    let tasks = match state.repo.queued_tasks(state.dataset(), None).await {
        Ok(tasks) => tasks,
        Err(error) => return repo_error(&error),
    };
    let now = Utc::now();
    let mut tiers = serde_json::Map::new();
    for tier in Tier::ALL {
        let mut items: Vec<Value> = tasks
            .iter()
            .filter(|t| effective_tier(t.priority_rank, t.queued_at, now, state.params.aging_stable_to_urgent_secs) == tier)
            .map(|t| {
                json!({
                    "task_id": t.task_id, "incident_id": t.incident_id, "required_asset": t.required_asset.to_string(),
                    "persons": t.persons(), "priority_rank": t.priority_rank, "junction_id": t.junction_id,
                    "queued_at": t.queued_at.to_rfc3339(), "waited_s": (now - t.queued_at).num_seconds().max(0),
                    "verify_failures": t.verify_failures, "escalations": t.escalations,
                })
            })
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item["waited_s"].as_i64().unwrap_or(0)));
        tiers.insert(tier.as_str().to_string(), Value::Array(items));
    }
    Json(json!({ "total": tasks.len(), "tiers": tiers })).into_response()
}

async fn alerts(State(state): State<ConsoleApiState>) -> Response {
    let dead_stream = format!("{}:dead", state.params.extract_stream);
    let (escalations, events, callbacks) = match tokio::try_join!(
        state.repo.console_escalations(50),
        state.repo.console_alert_events(50),
        state.repo.callback_queue(50),
    ) {
        Ok(result) => result,
        Err(error) => return repo_error(&error),
    };
    let dead_letters: Vec<Value> = match state.redis.latest(&dead_stream, 20).await {
        Ok(messages) => messages.iter().map(|m| json!({ "stream_id": m.id, "fields": m.fields })).collect(),
        Err(error) => {
            warn!(%error, "Could not read dead letters");
            Vec::new()
        }
    };
    let callbacks: Vec<Value> = callbacks
        .iter()
        .map(|c| {
            json!({ "incident_id": c.incident_id, "status": c.status,
                    "review_priority": c.review_priority.map(|p| p.as_str()), "created_at": c.created_at.to_rfc3339() })
        })
        .collect();
    Json(json!({ "escalations": escalations, "events": events, "callbacks": callbacks, "dead_letters": dead_letters }))
        .into_response()
}

/// Active hazard records with map geometry (open read for the console; the
/// token-checked `GET /api/v1/hazards` stays as it was).
async fn active_hazards(State(state): State<ConsoleApiState>) -> Response {
    let dataset = state.dataset().to_string();
    let (version, hazards) = match tokio::try_join!(state.repo.overlay_version(&dataset), state.repo.active_hazards(&dataset)) {
        Ok(result) => result,
        Err(error) => return repo_error(&error),
    };
    let ids: Vec<String> = hazards.iter().map(|h| h.segment_id.clone()).collect();
    let ends = match state.repo.segment_ends(&dataset, &ids).await {
        Ok(ends) => ends,
        Err(error) => return repo_error(&error),
    };
    let now = Utc::now();
    let features: Vec<Value> = hazards
        .iter()
        .filter_map(|h| {
            let end = ends.iter().find(|e| e["segment_id"] == h.segment_id.as_str())?;
            Some(json!({
                "type": "Feature",
                "geometry": { "type": "LineString", "coordinates": [
                    [end["from"]["lon"], end["from"]["lat"]], [end["to"]["lon"], end["to"]["lat"]] ] },
                "properties": {
                    "hazard_id": h.hazard_id, "segment_id": h.segment_id, "road_name": end["road_name"],
                    "status": h.status.as_str(), "actor_id": h.actor_id, "reason": h.reason,
                    "source": h.source.as_str(), "valid_from": h.valid_from.to_rfc3339(),
                    "valid_until": h.valid_until.map(|t| t.to_rfc3339()), "in_force": h.in_force_at(now),
                    "overlay_version": h.overlay_version,
                },
            }))
        })
        .collect();
    Json(json!({ "dataset": dataset, "overlay_version": version,
                 "geojson": { "type": "FeatureCollection", "features": features } }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct PointQuery {
    pub lat: f64,
    pub lon: f64,
}

/// Segments at the junction nearest to a clicked point.
async fn segments_near(State(state): State<ConsoleApiState>, Query(point): Query<PointQuery>) -> Response {
    let nearest = match state.state.find_nearest_junction(state.dataset(), point.lat, point.lon, SNAP_RADIUS_M).await {
        Ok(nearest) => nearest,
        Err(error) => return internal(error),
    };
    let Some((junction_id, distance_m)) = nearest else {
        return not_found("no road junction within 300 m of that point");
    };
    match state.repo.console_segments_at(state.dataset(), &junction_id).await {
        Ok(segments) => {
            Json(json!({ "junction_id": junction_id, "distance_m": distance_m, "segments": segments })).into_response()
        }
        Err(error) => repo_error(&error),
    }
}

/// Which leader-only roles are running (their leases exist), the overlay
/// version and the queue length.
async fn system(State(state): State<ConsoleApiState>) -> Response {
    let dataset = state.dataset().to_string();
    let mut conn = state.redis.manager.clone();
    let lease = |name: String| {
        let key = format!("lease:{name}");
        let mut conn = conn.clone();
        async move { conn.pttl::<_, i64>(key).await.unwrap_or(-2) }
    };
    let relay_ttl = lease(RELAY_LEASE.to_string()).await;
    let matcher_ttl = lease(lease_name(&dataset)).await;
    let queue_len: i64 = conn.zcard(format!("alloc:queue:{dataset}")).await.unwrap_or(0);
    let driver: bool = conn.exists(crate::sim::heartbeat_key(&dataset)).await.unwrap_or(false);
    let overlay_version = state.repo.overlay_version(&dataset).await.ok();
    Json(json!({
        "dataset": dataset,
        "relay_leader": relay_ttl > 0,
        "matcher_leader": matcher_ttl > 0,
        "overlay_version": overlay_version,
        "queue_length": queue_len,
        "simulated_driver": driver,
        "checked_at": Utc::now().to_rfc3339(),
    }))
    .into_response()
}

// -------------------------------------------------------------- commands

#[derive(Debug, Default, Deserialize)]
pub struct ReviewRequest {
    pub request_id: Option<String>,
    pub expected_version: Option<u64>,
    /// `APPROVE` or `REJECT`.
    pub decision: Option<String>,
    pub reason: Option<String>,
    pub required_asset: Option<String>,
    pub headcount: Option<u32>,
    /// Either a junction, or a point snapped to the nearest junction.
    pub junction_id: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Optional override: CRITICAL, URGENT, UNKNOWN or STABLE.
    pub urgency: Option<String>,
}

async fn review(
    State(state): State<ConsoleApiState>,
    Path(incident_id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<ReviewRequest>>,
) -> Response {
    let actor = match authorize(&state.operators, &headers, &[Permission::ReviewIncident]) {
        Ok(actor) => actor,
        Err(refused) => return refused.into_response(),
    };
    let Some(Json(request)) = body else {
        return unprocessable("request body must be JSON");
    };
    let command = match review_command(&state, incident_id, request).await {
        Ok(command) => command,
        Err(message) => return unprocessable(&message),
    };
    command_response(state.repo.review_incident(&actor, &command).await)
}

/// Turn a review request into a command, resolving the location.
async fn review_command(state: &ConsoleApiState, incident_id: String, request: ReviewRequest) -> Result<ReviewCommand, String> {
    let request_id = request.request_id.filter(|r| !r.trim().is_empty()).ok_or("request_id is required")?;
    let expected_version = request.expected_version.ok_or("expected_version is required")?;
    let reason = request.reason.unwrap_or_default();
    let verdict = match request.decision.as_deref().map(str::to_ascii_uppercase).as_deref() {
        Some("REJECT") => ReviewVerdict::Reject,
        Some("APPROVE") => {
            let required_asset = parse_asset(request.required_asset.as_deref().ok_or("required_asset is required")?)
                .ok_or("required_asset must be AMBULANCE, EVAC_TRUCK or RESCUE_BOAT")?;
            let headcount = request.headcount.ok_or("headcount is required")?;
            let urgency = match request.urgency.as_deref() {
                None | Some("") => None,
                Some(value) => Some(parse_urgency(value).ok_or("urgency must be CRITICAL, URGENT, UNKNOWN or STABLE")?),
            };
            let junction_id = match (request.junction_id.filter(|j| !j.trim().is_empty()), request.latitude, request.longitude) {
                (Some(junction), _, _) => junction,
                (None, Some(lat), Some(lon)) => state
                    .state
                    .find_nearest_junction(state.dataset(), lat, lon, SNAP_RADIUS_M)
                    .await
                    .map_err(|e| format!("location lookup failed: {e}"))?
                    .map(|(junction, _)| junction)
                    .ok_or("no road junction within 300 m of that location")?,
                _ => return Err("a location is required: junction_id, or latitude and longitude".into()),
            };
            ReviewVerdict::Approve { required_asset, headcount, junction_id, urgency }
        }
        _ => return Err("decision must be APPROVE or REJECT".into()),
    };
    let command = ReviewCommand {
        request_id,
        dataset: state.dataset().to_string(),
        incident_id,
        expected_version,
        decision: verdict,
        reason,
    };
    crate::db::validate_review(&command).map_err(|e| e.to_string())?;
    Ok(command)
}

#[derive(Debug, Default, Deserialize)]
pub struct CompleteRequest {
    pub request_id: Option<String>,
    pub expected_version: Option<u64>,
    pub note: Option<String>,
}

async fn complete(
    State(state): State<ConsoleApiState>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<CompleteRequest>>,
) -> Response {
    let actor = match authorize(&state.operators, &headers, &[Permission::CloseIncident, Permission::UpdateResponderStatus]) {
        Ok(actor) => actor,
        Err(refused) => return refused.into_response(),
    };
    let Some(Json(request)) = body else {
        return unprocessable("request body must be JSON");
    };
    let (Some(request_id), Some(expected_version)) = (request.request_id.filter(|r| !r.trim().is_empty()), request.expected_version)
    else {
        return unprocessable("request_id and expected_version are required");
    };
    let note = request.note.filter(|n| !n.trim().is_empty());
    command_response(
        state
            .repo
            .complete_task(&actor, &CompleteTaskCommand { request_id, task_id, expected_version, note })
            .await,
    )
}

#[derive(Debug, Default, Deserialize)]
pub struct MergeRequest {
    pub request_id: Option<String>,
    pub absorbed_id: Option<String>,
    pub survivor_version: Option<u64>,
    pub absorbed_version: Option<u64>,
    pub reason: Option<String>,
}

/// Merge `absorbed_id` into the incident in the path (T4).
async fn merge(
    State(state): State<ConsoleApiState>,
    Path(survivor_id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<MergeRequest>>,
) -> Response {
    let actor = match authorize(&state.operators, &headers, &[Permission::ReviewIncident]) {
        Ok(actor) => actor,
        Err(refused) => return refused.into_response(),
    };
    let Some(Json(request)) = body else {
        return unprocessable("request body must be JSON");
    };
    let (Some(request_id), Some(absorbed_id), Some(survivor_version), Some(absorbed_version)) =
        (request.request_id, request.absorbed_id, request.survivor_version, request.absorbed_version)
    else {
        return unprocessable("request_id, absorbed_id, survivor_version and absorbed_version are required");
    };
    let command = MergeCommand {
        request_id,
        survivor_id,
        survivor_version,
        absorbed_id,
        absorbed_version,
        reason: request.reason.unwrap_or_default(),
    };
    command_response(state.repo.merge_incidents(&actor, &command).await)
}

#[derive(Debug, Default, Deserialize)]
pub struct SplitRequest {
    pub request_id: Option<String>,
    pub report_id: Option<String>,
    pub expected_version: Option<u64>,
    pub reason: Option<String>,
}

/// Split a follow-up or merged report off the incident in the path.
async fn split(
    State(state): State<ConsoleApiState>,
    Path(incident_id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<SplitRequest>>,
) -> Response {
    let actor = match authorize(&state.operators, &headers, &[Permission::ReviewIncident]) {
        Ok(actor) => actor,
        Err(refused) => return refused.into_response(),
    };
    let Some(Json(request)) = body else {
        return unprocessable("request body must be JSON");
    };
    let (Some(request_id), Some(report_id), Some(expected_version)) = (request.request_id, request.report_id, request.expected_version)
    else {
        return unprocessable("request_id, report_id and expected_version are required");
    };
    let command = SplitCommand { request_id, incident_id, expected_version, report_id, reason: request.reason.unwrap_or_default() };
    command_response(state.repo.split_report(&actor, &command).await)
}

#[derive(Debug, Default, Deserialize)]
pub struct VehicleStatusRequest {
    pub request_id: Option<String>,
    /// `OUT_OF_SERVICE` or `AVAILABLE`.
    pub status: Option<String>,
    pub expected_version: Option<u64>,
    pub reason: Option<String>,
}

async fn vehicle_status(
    State(state): State<ConsoleApiState>,
    Path(vehicle_id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<VehicleStatusRequest>>,
) -> Response {
    let actor = match authorize(&state.operators, &headers, &[Permission::AssignDispatch, Permission::UpdateResponderStatus]) {
        Ok(actor) => actor,
        Err(refused) => return refused.into_response(),
    };
    let Some(Json(request)) = body else {
        return unprocessable("request body must be JSON");
    };
    let out_of_service = match request.status.as_deref().map(str::to_ascii_uppercase).as_deref() {
        Some("OUT_OF_SERVICE") => true,
        Some("AVAILABLE") => false,
        _ => return unprocessable("status must be OUT_OF_SERVICE or AVAILABLE"),
    };
    let (Some(request_id), Some(expected_version)) = (request.request_id, request.expected_version) else {
        return unprocessable("request_id and expected_version are required");
    };
    let command = VehicleStatusCommand {
        request_id,
        vehicle_id,
        expected_version,
        out_of_service,
        reason: request.reason.unwrap_or_default(),
    };
    command_response(state.repo.set_vehicle_status(&actor, &command).await)
}

fn command_response(result: Result<crate::db::CommandOutcome, OpsRepoError>) -> Response {
    match result {
        Ok(outcome) => {
            let mut response = (StatusCode::OK, Json(outcome.outcome)).into_response();
            if outcome.replayed {
                response.headers_mut().insert("idempotent-replayed", HeaderValue::from_static("true"));
            }
            response
        }
        Err(OpsRepoError::InvalidData(message)) => unprocessable(&message),
        Err(error) => repo_error(&error),
    }
}

// -------------------------------------------------------------- dispatch

/// Submit a report and wait until it is assigned, sent to review or
/// rejected (at most `DISPATCH_WAIT_SECS`). 200 with the incident and route,
/// or 202 with the status URL if it is still in progress.
async fn dispatch(State(state): State<ConsoleApiState>, body: Option<Json<ReportRequest>>) -> Response {
    let Some(Json(request)) = body else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "body must be a JSON object" }))).into_response();
    };
    let report = match validate(request) {
        Ok(report) => report,
        Err(reason) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": reason }))).into_response(),
    };
    let (report, _) = match state.repo.create_report(&report).await {
        Ok(created) => created,
        Err(error) => return repo_error(&error),
    };
    let deadline = Instant::now() + Duration::from_secs(state.params.dispatch_wait_secs);
    loop {
        match state.repo.report_incident(&report.report_id).await {
            Ok(Some((incident_id, status))) if matches!(status.as_str(), "ASSIGNED" | "REVIEW_REQUIRED" | "REJECTED" | "COMPLETED") => {
                let incident = state.repo.console_incident(&incident_id).await.ok().flatten();
                let route = route_geojson(&state, &incident_id).await.ok().flatten();
                return Json(json!({ "report_id": report.report_id, "status": status,
                                    "incident": incident, "route": route }))
                .into_response();
            }
            Ok(_) => {}
            Err(error) => return repo_error(&error),
        }
        if Instant::now() >= deadline {
            return (
                StatusCode::ACCEPTED,
                Json(json!({ "report_id": report.report_id, "status": "IN_PROGRESS",
                             "status_url": format!("/api/v1/reports/{}", report.report_id) })),
            )
                .into_response();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ---------------------------------------------------------------- helpers

fn parse_asset(value: &str) -> Option<AssetType> {
    match value.to_ascii_uppercase().as_str() {
        "AMBULANCE" => Some(AssetType::Ambulance),
        "EVAC_TRUCK" => Some(AssetType::EvacTruck),
        "RESCUE_BOAT" => Some(AssetType::RescueBoat),
        _ => None,
    }
}

fn parse_urgency(value: &str) -> Option<Urgency> {
    match value.to_ascii_uppercase().as_str() {
        "CRITICAL" => Some(Urgency::Critical),
        "URGENT" => Some(Urgency::Urgent),
        "UNKNOWN" => Some(Urgency::Unknown),
        "STABLE" => Some(Urgency::Stable),
        _ => None,
    }
}

fn not_found(message: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response()
}

fn unprocessable(message: &str) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": message }))).into_response()
}

fn internal(error: Box<dyn std::error::Error + Send + Sync>) -> Response {
    warn!(%error, "Console read failed");
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "temporarily unavailable" }))).into_response()
}

fn repo_error(error: &OpsRepoError) -> Response {
    let status = error_status(error);
    let mut response = (status, Json(json!({ "error": error.to_string() }))).into_response();
    if status == StatusCode::SERVICE_UNAVAILABLE {
        response.headers_mut().insert("retry-after", HeaderValue::from_static("2"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assets_and_urgency_case_insensitively() {
        assert_eq!(parse_asset("ambulance"), Some(AssetType::Ambulance));
        assert_eq!(parse_asset("HELICOPTER"), None, "no helicopters in the pilot fleet");
        assert_eq!(parse_urgency("critical"), Some(Urgency::Critical));
        assert_eq!(parse_urgency("high"), None);
    }
}
