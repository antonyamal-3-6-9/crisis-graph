use axum::{
    extract::{ConnectInfo, Path, Query, Request, State},
    http::StatusCode,
    middleware::{from_fn, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        Json, Response,
    },
    routing::{get, patch, post},
    Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, info, warn};
use uuid::Uuid;

use crisis_graph::config::Config;
use crisis_graph::db::{
    IncidentRepository, Neo4jClient, RedisClient, ShelterWithLocation, StateManager,
};
use crisis_graph::ingestion::TriageExtractor;
use crisis_graph::models::{
    ActorContext, ActorRole, AssetType, CrisisState, DispatchStatus, Incident, IncidentStatus,
    ReviewDecisionKind, RoadStatus, RouteDecision, SosAlert, TriageReport, UncertaintyWaterfall,
};
use crisis_graph::pipeline::{PersistedCrisisOrchestrator, PipelineStages};
use crisis_graph::solver::{MultiModalPlan, TransportMode};

const STREAM_KEY: &str = "sos:stream:aluva";
const CONSUMER_GROUP: &str = "crisisgraph_workers";

#[derive(Clone)]
struct AppState {
    orchestrator: PersistedCrisisOrchestrator<IncidentRepository>,
    incident_repository: IncidentRepository,
    state_manager: StateManager,
    redis: RedisClient,
    event_tx: Arc<broadcast::Sender<String>>,
}

#[derive(Debug, Deserialize)]
pub struct DispatchRequest {
    pub raw_text: String,
    pub source_channel: Option<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub accuracy_m: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DispatchResponse {
    pub alert_id: String,
    pub incident_status: String,
    pub incident_version: u64,
    pub control_plane_persisted: bool,
    pub duplicate_suppressed: bool,
    pub review_required: bool,
    pub status: String,
    pub victim_junction: Option<String>,
    pub assigned_shelter: Option<ShelterSummary>,
    pub assigned_asset: Option<String>,
    pub headcount: Option<u32>,
    pub triage: Option<TriageReport>,
    pub distance_km: f64,
    pub travel_time_s: f64,
    pub is_detour: bool,
    pub detour_reason: Option<String>,
    pub segment_count: usize,
    pub tactical_brief: String,
    pub geojson: serde_json::Value,
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multimodal_plan: Option<MultiModalPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertainty_waterfall: Option<UncertaintyWaterfall>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShelterSummary {
    pub id: String,
    pub name: String,
    pub junction_id: String,
}

#[derive(Debug, Serialize)]
pub struct IncidentRouteResponse {
    pub incident_id: String,
    pub incident_status: IncidentStatus,
    pub route: RouteDecision,
    pub geojson: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct IncidentReviewRequest {
    pub decision: ReviewDecisionKind,
    pub expected_version: u64,
    pub reason: String,
    pub reviewer_id: Option<String>,
    pub headcount: Option<u32>,
    pub required_asset: Option<AssetType>,
    pub resolved_junction_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApplyHazardRequest {
    pub segment_id: String,
    pub status: String,
    pub duration_hours: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseIncidentRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ShelterFleetUpdateRequest {
    pub delta_ambulances: Option<i32>,
    pub delta_trucks: Option<i32>,
    pub delta_boats: Option<i32>,
    pub delta_capacity: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct WaitingQueueItem {
    pub position: usize,
    pub incident_id: String,
    pub status: IncidentStatus,
    pub requested_asset: Option<AssetType>,
    pub headcount: Option<u32>,
    pub location_raw: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    limit: Option<usize>,
}

async fn http_request_logger(req: Request, next: Next) -> Response {
    let client_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let method = req.method().clone();
    let uri = req.uri().to_string();
    let version = format!("{:?}", req.version());

    let is_sse = uri.starts_with("/api/v1/events");

    if !is_sse {
        info!(
            target: "http",
            "--> {client_ip} \"{method} {uri} {version}\""
        );
    }

    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed();
    let status = response.status();
    let status_code = status.as_u16();
    let reason = status.canonical_reason().unwrap_or("");

    let elapsed_str = if elapsed.as_secs() > 0 {
        format!("{:.2}s", elapsed.as_secs_f64())
    } else if elapsed.as_millis() > 0 {
        format!("{:.2}ms", elapsed.as_secs_f64() * 1000.0)
    } else {
        format!("{}µs", elapsed.as_micros())
    };

    if !is_sse {
        if elapsed.as_millis() >= 500 {
            warn!(
                target: "http",
                "<-- {client_ip} \"{method} {uri} {version}\" {status_code} {reason} [SLOW: {elapsed_str}]"
            );
        } else {
            info!(
                target: "http",
                "<-- {client_ip} \"{method} {uri} {version}\" {status_code} {reason} [{elapsed_str}]"
            );
        }
    }

    response
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "crisis_graph=info,server=info,http=info".into());

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .compact()
        .init();

    info!("Starting CrisisGraph Emergency Dispatch Server...");

    let config = Config::from_env();
    let neo4j = Neo4jClient::connect(&config).await?;
    neo4j.ping().await?;

    let redis = RedisClient::connect(&config).await?;
    let state_manager = StateManager::new(neo4j.clone());
    let incident_repository = IncidentRepository::new(neo4j.clone());

    // Register missing pilot facilities without erasing live allocations.
    state_manager.ensure_aluva_shelters().await?;
    state_manager.ensure_operational_schema().await?;
    incident_repository.setup_schema().await?;

    // Setup Redis Streams consumer group
    if let Err(e) = redis
        .ensure_consumer_group(STREAM_KEY, CONSUMER_GROUP)
        .await
    {
        warn!("Consumer group setup note: {e}");
    }

    let extractor = TriageExtractor::new(&config);
    let stages = PipelineStages::new(extractor, state_manager.clone(), redis.clone())
        .with_on_scene_radius_m(config.params.on_scene_radius_m);
    let orchestrator = PersistedCrisisOrchestrator::new(stages, incident_repository.clone());

    // Event broadcast channel for real-time SSE streaming to web dashboard
    let (event_tx, _) = broadcast::channel::<String>(256);
    let event_tx_arc = Arc::new(event_tx);

    let app_state = AppState {
        orchestrator: orchestrator.clone(),
        incident_repository: incident_repository.clone(),
        state_manager: state_manager.clone(),
        redis: redis.clone(),
        event_tx: event_tx_arc.clone(),
    };

    // Spawn background Redis Streams worker pool task
    spawn_stream_worker(
        redis.clone(),
        orchestrator.clone(),
        state_manager.clone(),
        event_tx_arc.clone(),
    );

    let ttl_seconds: u64 = std::env::var("MISSION_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150); // 2.5 minutes default threshold

    // Spawn background TTL Resource Sweeper & Queue Processor
    spawn_ttl_sweeper(
        redis.clone(),
        orchestrator.clone(),
        incident_repository.clone(),
        state_manager.clone(),
        event_tx_arc.clone(),
        ttl_seconds,
    );

    let router = Router::new()
        // API Routes
        .route("/api/v1/health", get(health_check))
        .route("/api/v1/shelters", get(get_shelters))
        .route("/api/v1/shelters/reset", post(reset_shelters))
        .route(
            "/api/v1/shelters/{shelter_id}/fleet",
            patch(handle_modify_shelter_fleet),
        )
        .route("/api/v1/hazards", get(get_hazards))
        .route("/api/v1/hazards", post(apply_hazard))
        .route("/api/v1/hazards/clear", post(clear_hazards))
        .route("/api/v1/dispatch", post(handle_dispatch))
        .route("/api/v1/incidents", get(list_incidents))
        .route("/api/v1/incidents/{incident_id}", get(get_incident))
        .route(
            "/api/v1/incidents/{incident_id}/route",
            get(get_incident_route),
        )
        .route(
            "/api/v1/incidents/{incident_id}/audit",
            get(get_incident_audit),
        )
        .route(
            "/api/v1/incidents/{incident_id}/review",
            post(handle_incident_review),
        )
        .route(
            "/api/v1/incidents/{incident_id}/release",
            post(handle_incident_release),
        )
        .route("/api/v1/queue/waiting", get(get_waiting_queue))
        .route(
            "/api/v1/resources/reservations",
            get(get_resource_reservations),
        )
        .route("/api/v1/admin/graph/summary", get(get_graph_summary))
        // Event-Driven Stream & Real-time Endpoints
        .route("/api/v1/events", get(sse_events))
        .route("/api/v1/sos/stream", post(ingest_sos_stream))
        .route("/api/v1/sos/simulate_spike", post(simulate_spike))
        // Read-only local demonstration console routes. These deliberately
        // share one dependency-free frontend application.
        .route_service("/", ServeFile::new("web/index.html"))
        .route_service("/dispatch", ServeFile::new("web/index.html"))
        .route_service("/incidents", ServeFile::new("web/index.html"))
        .route_service("/hazards", ServeFile::new("web/index.html"))
        .route_service("/resources", ServeFile::new("web/index.html"))
        // Serve Web Assets from `web/` folder
        .fallback_service(ServeDir::new("web"))
        .layer(from_fn(http_request_logger))
        .layer(CorsLayer::permissive())
        .with_state(app_state);

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .unwrap_or(3000);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("CrisisGraph Server listening on http://localhost:{}", port);
    info!("Web Dispatcher Console active at http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "service": "CrisisGraph Emergency Dispatch Engine",
        "timestamp": Utc::now().to_rfc3339(),
        "pilot": "Aluva–Periyar Pilot Area (10 km)",
        "streams": {
            "ingest_stream": STREAM_KEY,
            "consumer_group": CONSUMER_GROUP
        }
    }))
}

async fn sse_events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(data) => Some(Ok(Event::default().data(data))),
        Err(_) => None,
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn ingest_sos_stream(
    State(state): State<AppState>,
    Json(req): Json<DispatchRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut alert = SosAlert::new(
        format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
        req.raw_text,
    );
    alert.source_channel = req.source_channel.or_else(|| Some("redis_streams_intake".to_string()));
    alert.latitude = req.latitude;
    alert.longitude = req.longitude;
    alert.accuracy_m = req.accuracy_m;

    let payload = serde_json::to_string(&alert).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match state.redis.xadd(STREAM_KEY, &payload).await {
        Ok(stream_msg_id) => Ok(Json(json!({
            "status": "queued",
            "stream_message_id": stream_msg_id,
            "alert_id": alert.alert_id,
            "stream": STREAM_KEY
        }))),
        Err(e) => {
            error!("Failed to enqueue SOS into Redis stream: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn simulate_spike(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let sample_alerts = [
        "URGENT: Flash flood at (lat: 10.1135, lon: 76.3540) near Pump Junction. 4 persons trapped, ambulance needed!",
        "Aluva Railway Station has 3 injured passengers, need ambulance dispatch immediately.",
        "Water entering Aluva Manappuram temple grounds, 12 pilgrims stranded on temple steps. Urgent boat rescue needed!",
        "House partially submerged near Bank Junction (lat: 10.1082, lon: 76.3565). 2 elderly persons need rescue.",
        "Flash flood at UC College gate 2, 6 students stranded in water, need evacuation truck.",
    ];

    let mut queued = Vec::new();

    for alert_text in sample_alerts {
        let mut alert = SosAlert::new(
            format!(
                "SOS-SPIKE-{}",
                Uuid::new_v4().to_string()[..6].to_uppercase()
            ),
            alert_text.to_string(),
        );
        alert.source_channel = Some("disaster_spike_simulator".to_string());

        if let Ok(payload) = serde_json::to_string(&alert) {
            if let Ok(msg_id) = state.redis.xadd(STREAM_KEY, &payload).await {
                queued.push(json!({
                    "alert_id": alert.alert_id,
                    "stream_id": msg_id,
                    "text": alert_text
                }));
            }
        }
    }

    Ok(Json(json!({
        "status": "ok",
        "queued_count": queued.len(),
        "alerts": queued
    })))
}

fn spawn_stream_worker(
    redis: RedisClient,
    orchestrator: PersistedCrisisOrchestrator<IncidentRepository>,
    state_mgr: StateManager,
    event_tx: Arc<broadcast::Sender<String>>,
) {
    tokio::spawn(async move {
        info!(
            ">>> Redis Streams Worker Pool started on stream: {} [group: {}] <<<",
            STREAM_KEY, CONSUMER_GROUP
        );

        loop {
            // Read next batch of messages from consumer group
            match redis
                .read_group_messages(STREAM_KEY, CONSUMER_GROUP, "worker_tokio_1", 10, 2000)
                .await
            {
                Ok(messages) => {
                    for (msg_id, payload_str) in messages {
                        info!("[Stream Consumer] Received message id: {}", msg_id);

                        let alert: SosAlert = match serde_json::from_str(&payload_str) {
                            Ok(a) => a,
                            Err(_) => {
                                let mut a = SosAlert::new(
                                    format!(
                                        "SOS-{}",
                                        Uuid::new_v4().to_string()[..8].to_uppercase()
                                    ),
                                    payload_str,
                                );
                                a.source_channel = Some("stream_fallback".to_string());
                                a
                            },
                        };

                        // Process through the full 4-stage pipeline
                        let result = orchestrator.process_alert(alert).await;
                        let response = build_dispatch_response(
                            &result.state,
                            &result.incident,
                            result.persistence_healthy,
                            result.duplicate_suppressed,
                            &state_mgr,
                        )
                        .await;

                        // Broadcast to connected web clients (SSE)
                        if !result.duplicate_suppressed {
                            if let Ok(json_str) = serde_json::to_string(&response) {
                                let _ = event_tx.send(json_str);
                            }
                        }

                        // Acknowledge stream message
                        if let Err(e) = redis.xack(STREAM_KEY, CONSUMER_GROUP, &msg_id).await {
                            warn!("Failed to XACK stream message {msg_id}: {e}");
                        }
                    }
                }
                Err(e) => {
                    warn!("Stream reading error: {e}. Backing off 1s...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        }
    });
}

async fn process_waiting_queue(
    redis: &RedisClient,
    orchestrator: &PersistedCrisisOrchestrator<IncidentRepository>,
    incident_repo: &IncidentRepository,
    state_mgr: &StateManager,
    event_tx: &Arc<broadcast::Sender<String>>,
) {
    while let Ok(Some(incident_id)) = redis.pop_waiting_incident().await {
        info!("[FIFO Queue] Evaluating waiting incident: {}", incident_id);

        let incident_opt = match incident_repo.get(&incident_id).await {
            Ok(inc) => inc,
            Err(e) => {
                warn!("[FIFO Queue] Error loading incident {}: {}", incident_id, e);
                continue;
            }
        };

        let incident = match incident_opt {
            Some(inc) => inc,
            None => {
                warn!("[FIFO Queue] Incident {} not found in repository", incident_id);
                continue;
            }
        };

        if incident.status != IncidentStatus::ReviewRequired
            && incident.status != IncidentStatus::Allocating
            && incident.status != IncidentStatus::TriageApproved
        {
            info!(
                "[FIFO Queue] Incident {} is in status {:?}; skipping queue processing",
                incident_id, incident.status
            );
            continue;
        }

        let triage = incident
            .triage_revisions
            .last()
            .map(|r| r.triage.clone())
            .unwrap_or_else(|| TriageReport {
                victim_location_raw: None,
                resolved_junction_id: None,
                headcount: None,
                required_asset: None,
                hazards: vec![],
                confidence_score: 1.0,
                needs_human_review: false,
                uncertainty_reasons: vec![],
                inference: crisis_graph::models::TriageInferenceMetadata {
                    backend: "fifo_queue_auto_allocator".to_string(),
                    model: "queue_resumption".to_string(),
                    adapter_id: None,
                    adapter_sha256: None,
                    prompt_version: "auto".to_string(),
                    schema_version: "auto".to_string(),
                    latency_ms: 0,
                    tokens_per_second: None,
                    prompt_eval_ms: None,
                    completion_tokens: None,
                },
                victim_profile: None,
                urgency: None,
                location_source: None,
            });

        match orchestrator
            .process_review_approval(
                &incident_id,
                incident.version,
                "Auto-allocated from FIFO waiting queue following resource release",
                triage,
                "fifo_queue_worker",
            )
            .await
        {
            Ok(res) => {
                let response = build_dispatch_response(
                    &res.state,
                    &res.incident,
                    res.persistence_healthy,
                    res.duplicate_suppressed,
                    state_mgr,
                )
                .await;

                if let Ok(json_str) = serde_json::to_string(&response) {
                    let _ = event_tx.send(json_str);
                }

                if res.incident.status == IncidentStatus::RouteVerified {
                    info!(
                        "[FIFO Queue] Successfully allocated freed resource and certified route for incident {}",
                        incident_id
                    );
                } else {
                    info!(
                        "[FIFO Queue] Incident {} could not allocate currently available fleet; keeping in queue for next release",
                        incident_id
                    );
                    break;
                }
            }
            Err(e) => {
                warn!("[FIFO Queue] Error processing queued incident {}: {}", incident_id, e);
                break;
            }
        }
    }
}

fn spawn_ttl_sweeper(
    redis: RedisClient,
    orchestrator: PersistedCrisisOrchestrator<IncidentRepository>,
    incident_repo: IncidentRepository,
    state_mgr: StateManager,
    event_tx: Arc<broadcast::Sender<String>>,
    ttl_seconds: u64,
) {
    tokio::spawn(async move {
        info!(
            ">>> Resource TTL Sweeper active (Threshold: {}s / {:.1} min) <<<",
            ttl_seconds,
            ttl_seconds as f64 / 60.0
        );

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
        let sweeper_actor = ActorContext::new("crisisgraph_ttl_sweeper", [ActorRole::Dispatcher]);

        loop {
            interval.tick().await;

            match state_mgr.get_expired_reservations(ttl_seconds).await {
                Ok(expired) if !expired.is_empty() => {
                    info!(
                        "[TTL Sweeper] Found {} expired reservation(s) exceeding {}s threshold",
                        expired.len(),
                        ttl_seconds
                    );

                    let mut any_freed = false;
                    for exp in expired {
                        info!(
                            "[TTL Sweeper] Auto-releasing reservation for incident {} (Asset: {}, Shelter: {})",
                            exp.incident_id, exp.asset_type, exp.shelter_id
                        );

                        match state_mgr
                            .release_incident_reservation(
                                &exp.incident_id,
                                "Mission completed (TTL auto-release)",
                            )
                            .await
                        {
                            Ok(true) => {
                                any_freed = true;
                                if let Ok(Some(mut incident)) = incident_repo.get(&exp.incident_id).await {
                                    if !incident.status.is_terminal() {
                                        if let Ok(event) = incident.transition(
                                            &sweeper_actor,
                                            incident.version,
                                            IncidentStatus::Completed,
                                            "Mission completed: vehicle returned to depot after TTL expiration",
                                            Utc::now(),
                                        ) {
                                            if let Err(e) = incident_repo.save_transition(&incident, &event).await {
                                                warn!("Failed to persist TTL completion transition: {}", e);
                                            } else {
                                                info!(
                                                    incident_id = %exp.incident_id,
                                                    "Incident transitioned to COMPLETED by TTL sweeper"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(false) => {}
                            Err(e) => {
                                warn!("Failed to release expired reservation {}: {}", exp.incident_id, e);
                            }
                        }
                    }

                    if any_freed {
                        process_waiting_queue(
                            &redis,
                            &orchestrator,
                            &incident_repo,
                            &state_mgr,
                            &event_tx,
                        )
                        .await;
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("TTL Sweeper query error: {}", e);
                }
            }
        }
    });
}

async fn handle_incident_release(
    State(state): State<AppState>,
    Path(incident_id): Path<String>,
    Json(payload): Json<Option<ReleaseIncidentRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let reason = payload
        .and_then(|p| p.reason)
        .unwrap_or_else(|| "Operator manual vehicle release & mission completion".to_string());

    info!(
        incident_id = %incident_id,
        reason = %reason,
        "Processing operator manual resource release"
    );

    let released = state
        .state_manager
        .release_incident_reservation(&incident_id, &reason)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to release reservation: {}", e)))?;

    let dispatcher_actor = ActorContext::new("dispatcher_console", [ActorRole::Dispatcher]);
    if let Ok(Some(mut incident)) = state.incident_repository.get(&incident_id).await {
        if !incident.status.is_terminal() {
            if let Ok(event) = incident.transition(
                &dispatcher_actor,
                incident.version,
                IncidentStatus::Completed,
                &reason,
                Utc::now(),
            ) {
                let _ = state.incident_repository.save_transition(&incident, &event).await;
            }
        }
    }

    let _ = state.redis.remove_waiting_incident(&incident_id).await;

    // Wake up FIFO queue to re-assign freed resource to next waiting incident
    process_waiting_queue(
        &state.redis,
        &state.orchestrator,
        &state.incident_repository,
        &state.state_manager,
        &state.event_tx,
    )
    .await;

    Ok(Json(json!({
        "incident_id": incident_id,
        "released": released,
        "status": "COMPLETED",
        "reason": reason
    })))
}

async fn handle_modify_shelter_fleet(
    State(state): State<AppState>,
    Path(shelter_id): Path<String>,
    Json(payload): Json<ShelterFleetUpdateRequest>,
) -> Result<Json<ShelterWithLocation>, (StatusCode, String)> {
    let delta_amb = payload.delta_ambulances.unwrap_or(0);
    let delta_trk = payload.delta_trucks.unwrap_or(0);
    let delta_boat = payload.delta_boats.unwrap_or(0);
    let delta_cap = payload.delta_capacity.unwrap_or(0);

    info!(
        shelter_id = %shelter_id,
        delta_amb = delta_amb,
        delta_trk = delta_trk,
        delta_boat = delta_boat,
        delta_cap = delta_cap,
        "Modifying shelter fleet resources"
    );

    let updated = state
        .state_manager
        .modify_shelter_fleet(&shelter_id, delta_amb, delta_trk, delta_boat, delta_cap)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to modify shelter fleet: {}", e)))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Shelter '{}' not found", shelter_id)))?;

    if delta_amb > 0 || delta_trk > 0 || delta_boat > 0 || delta_cap > 0 {
        process_waiting_queue(
            &state.redis,
            &state.orchestrator,
            &state.incident_repository,
            &state.state_manager,
            &state.event_tx,
        )
        .await;
    }

    Ok(Json(updated))
}

async fn get_waiting_queue(
    State(state): State<AppState>,
) -> Result<Json<Vec<WaitingQueueItem>>, StatusCode> {
    let ids = state
        .redis
        .get_waiting_incidents()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut items = Vec::new();
    for (idx, id) in ids.into_iter().enumerate() {
        if let Ok(Some(inc)) = state.incident_repository.get(&id).await {
            let triage = inc.current_triage();
            items.push(WaitingQueueItem {
                position: idx + 1,
                incident_id: id,
                status: inc.status,
                requested_asset: triage.and_then(|t| t.required_asset.clone()),
                headcount: triage.and_then(|t| t.headcount),
                location_raw: triage.and_then(|t| t.victim_location_raw.clone()),
            });
        }
    }

    Ok(Json(items))
}

async fn get_shelters(
    State(state): State<AppState>,
) -> Result<Json<Vec<ShelterWithLocation>>, StatusCode> {
    state
        .state_manager
        .get_shelters_with_locations()
        .await
        .map(Json)
        .map_err(|e| {
            warn!("Failed to fetch shelters: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn list_incidents(
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> Result<Json<Vec<Incident>>, StatusCode> {
    state
        .incident_repository
        .list_recent(params.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| {
            warn!("Failed to list incidents: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_incident(
    State(state): State<AppState>,
    Path(incident_id): Path<String>,
) -> Result<Json<Incident>, StatusCode> {
    state
        .incident_repository
        .get(&incident_id)
        .await
        .map_err(|error| {
            warn!("Failed to load incident {incident_id}: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn get_incident_audit(
    State(state): State<AppState>,
    Path(incident_id): Path<String>,
) -> Result<Json<Vec<crisis_graph::models::AuditEvent>>, StatusCode> {
    state
        .incident_repository
        .audit_history(&incident_id)
        .await
        .map(Json)
        .map_err(|error| {
            warn!("Failed to load audit history for {incident_id}: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn handle_incident_review(
    State(state): State<AppState>,
    Path(incident_id): Path<String>,
    Json(req): Json<IncidentReviewRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let reviewer = req
        .reviewer_id
        .unwrap_or_else(|| "dispatcher_console".to_string());

    info!(
        incident_id = %incident_id,
        decision = ?req.decision,
        reviewer = %reviewer,
        reason = %req.reason,
        "Processing dispatcher incident review"
    );

    match req.decision {
        ReviewDecisionKind::Approve => {
            let current = state
                .incident_repository
                .get(&incident_id)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                .ok_or_else(|| (StatusCode::NOT_FOUND, "Incident not found".to_string()))?;

            let base_triage = current
                .triage_revisions
                .last()
                .map(|r| r.triage.clone())
                .unwrap_or_else(|| TriageReport {
                    victim_location_raw: None,
                    resolved_junction_id: None,
                    headcount: None,
                    required_asset: None,
                    hazards: vec![],
                    confidence_score: 1.0,
                    needs_human_review: false,
                    uncertainty_reasons: vec![],
                    inference: crisis_graph::models::TriageInferenceMetadata {
                        backend: "dispatcher_override".to_string(),
                        model: "human_review".to_string(),
                        adapter_id: None,
                        adapter_sha256: None,
                        prompt_version: "manual".to_string(),
                        schema_version: "manual".to_string(),
                        latency_ms: 0,
                        tokens_per_second: None,
                        prompt_eval_ms: None,
                        completion_tokens: None,
                    },
                    victim_profile: None,
                    urgency: None,
                    location_source: None,
                });

            let mut reviewed_triage = base_triage;
            if let Some(h) = req.headcount {
                reviewed_triage.headcount = Some(h);
            }
            if let Some(a) = req.required_asset {
                reviewed_triage.required_asset = Some(a);
            }
            if let Some(j) = req.resolved_junction_id {
                reviewed_triage.resolved_junction_id = Some(j);
            }
            reviewed_triage.needs_human_review = false;
            reviewed_triage.uncertainty_reasons.clear();
            reviewed_triage.confidence_score = 1.0;
            reviewed_triage.hazards.retain(|h| {
                h.road_segment.is_some() && h.status.is_some() && h.duration_hours.is_some()
            });

            let result = state
                .orchestrator
                .process_review_approval(
                    &incident_id,
                    req.expected_version,
                    &req.reason,
                    reviewed_triage,
                    &reviewer,
                )
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

            let response = build_dispatch_response(
                &result.state,
                &result.incident,
                result.persistence_healthy,
                result.duplicate_suppressed,
                &state.state_manager,
            )
            .await;

            if let Ok(json_str) = serde_json::to_string(&response) {
                let _ = state.event_tx.send(json_str);
            }

            Ok(Json(json!(response)))
        }
        ReviewDecisionKind::Reject => {
            let incident = state
                .orchestrator
                .process_review_rejection(
                    &incident_id,
                    req.expected_version,
                    &req.reason,
                    &reviewer,
                )
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

            let response = json!({
                "incident_id": incident.incident_id,
                "status": "REJECTED",
                "incident_status": incident.status,
                "version": incident.version,
                "decision": "REJECT",
                "reason": req.reason,
            });

            if let Ok(json_str) = serde_json::to_string(&response) {
                let _ = state.event_tx.send(json_str);
            }

            Ok(Json(response))
        }
        ReviewDecisionKind::RequestChanges => Err((
            StatusCode::BAD_REQUEST,
            "RequestChanges is not supported in the demonstration console".to_string(),
        )),
    }
}

async fn get_incident_route(
    State(state): State<AppState>,
    Path(incident_id): Path<String>,
) -> Result<Json<IncidentRouteResponse>, StatusCode> {
    let incident = state
        .incident_repository
        .get(&incident_id)
        .await
        .map_err(|error| {
            warn!("Failed to load incident {incident_id}: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    let route = incident
        .current_route()
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    let incident_status = incident.status;
    let coordinates = state
        .state_manager
        .get_junction_coordinates(&route.junction_path)
        .await
        .map_err(|error| {
            warn!("Failed to project route for incident {incident_id}: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if coordinates.len() != route.junction_path.len() {
        warn!(
            incident_id,
            expected = route.junction_path.len(),
            actual = coordinates.len(),
            "Persisted route references junctions missing from the baseline graph"
        );
        return Err(StatusCode::CONFLICT);
    }

    let geojson = persisted_route_geojson(&route, &coordinates);
    Ok(Json(IncidentRouteResponse {
        incident_id,
        incident_status,
        route,
        geojson,
    }))
}

fn persisted_route_geojson(route: &RouteDecision, coordinates: &[[f64; 2]]) -> serde_json::Value {
    let mut features = Vec::new();
    let coord_map: std::collections::HashMap<&str, [f64; 2]> = route
        .junction_path
        .iter()
        .map(|s| s.as_str())
        .zip(coordinates.iter().copied())
        .collect();

    let mut rendered_legs = false;

    // 1. Render Multi-Modal Discrete Legs if mitigation was required
    if let Some(ref plan) = route.multimodal_plan {
        if plan.requires_mitigation && !plan.legs.is_empty() {
            rendered_legs = true;
            for leg in &plan.legs {
                let leg_coords: Vec<[f64; 2]> = leg
                    .waypoints
                    .iter()
                    .filter_map(|w| coord_map.get(w.as_str()).copied())
                    .collect();

                if leg_coords.len() >= 2 {
                    let (stroke, dash_style, mode_label) = match leg.mode {
                        TransportMode::RoadVehicle => ("#10b981", "solid", "Road Vehicle Transit"),
                        TransportMode::Watercraft => ("#06b6d4", "dashed", "Rescue Boat Water Corridor"),
                        TransportMode::FootWading => ("#f59e0b", "dotted", "Foot / Stretcher Transfer"),
                        TransportMode::Aerial => ("#a855f7", "dashed", "Direct Aerial Evacuation"),
                    };

                    features.push(json!({
                        "type": "Feature",
                        "geometry": { "type": "LineString", "coordinates": leg_coords },
                        "properties": {
                            "type": "route_leg",
                            "leg_index": leg.leg_index,
                            "mode": format!("{:?}", leg.mode),
                            "mode_label": mode_label,
                            "distance_km": leg.distance_km,
                            "travel_time_s": leg.travel_time_s,
                            "is_hazard": leg.is_hazard_stretch,
                            "stroke": stroke,
                            "dash_style": dash_style
                        }
                    }));
                }
            }

            // Staging Point Marker
            if let Some(ref staging_id) = plan.staging_junction {
                if let Some(coord) = coord_map.get(staging_id.as_str()) {
                    features.push(json!({
                        "type": "Feature",
                        "geometry": {
                            "type": "Point",
                            "coordinates": coord
                        },
                        "properties": {
                            "type": "staging_hub",
                            "junction_id": staging_id,
                            "name": "Dry Staging Hub (Boat Launch & Ambulance Handover)"
                        }
                    }));
                }
            }
        }
    }

    // 2. Single Continuous Path (Fallback or direct road route)
    if !rendered_legs && coordinates.len() >= 2 {
        features.push(json!({
            "type": "Feature",
            "geometry": { "type": "LineString", "coordinates": coordinates },
            "properties": {
                "type": "route",
                "route_id": route.route_id,
                "route_version": route.route_version,
                "triage_revision": route.triage_revision,
                "status": "ROUTE_VERIFIED",
                "distance_km": route.total_distance_km,
                "travel_time_s": route.total_travel_time_s,
                "verified_segment_count": route.verified_segment_count,
                "stroke": "#10b981",
                "dash_style": "solid"
            }
        }));
    }

    // 3. Origin Base / Shelter Marker (Distinct Blue)
    if let Some(origin) = coordinates.first() {
        features.push(json!({
            "type": "Feature",
            "geometry": { "type": "Point", "coordinates": origin },
            "properties": {
                "type": "shelter",
                "id": route.origin_shelter_id,
                "name": route.origin_shelter_name,
                "asset": route.assigned_asset_id,
                "role": "origin",
                "label": format!("🚩 Base Depot: {}", route.origin_shelter_name)
            }
        }));
    }

    // 4. Victim Distress Location Marker (Distinct Red)
    if let Some(victim) = coordinates.last() {
        features.push(json!({
            "type": "Feature",
            "geometry": { "type": "Point", "coordinates": victim },
            "properties": {
                "type": "victim",
                "junction_id": route.victim_junction_id,
                "role": "destination",
                "label": format!("📍 Victim Location: {}", route.victim_junction_id)
            }
        }));
    }

    json!({ "type": "FeatureCollection", "features": features })
}

async fn get_resource_reservations(
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> Result<Json<Vec<crisis_graph::db::ResourceReservationInfo>>, StatusCode> {
    state
        .state_manager
        .get_resource_reservations(params.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| {
            warn!("Failed to list resource reservations: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_graph_summary(
    State(state): State<AppState>,
) -> Result<Json<crisis_graph::db::GraphSummary>, StatusCode> {
    state
        .state_manager
        .get_graph_summary("aluva-periyar-pilot")
        .await
        .map(Json)
        .map_err(|error| {
            warn!("Failed to get graph summary: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn reset_shelters(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    state
        .state_manager
        .seed_aluva_shelters()
        .await
        .map(|_| {
            Json(json!({
                "status": "ok",
                "message": "Aluva shelters reset to initial baseline capacity and assets."
            }))
        })
        .map_err(|e| {
            warn!("Failed to reset shelters: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_hazards(State(state): State<AppState>) -> Result<Json<serde_json::Value>, StatusCode> {
    match state
        .state_manager
        .get_active_hazards("aluva-periyar-pilot")
        .await
    {
        Ok(hazards) => {
            let features: Vec<serde_json::Value> = hazards
                .into_iter()
                .map(|h| {
                    json!({
                        "type": "Feature",
                        "geometry": {
                            "type": "LineString",
                            "coordinates": [
                                [h.from_lon, h.from_lat],
                                [h.to_lon, h.to_lat]
                            ]
                        },
                        "properties": {
                            "segment_id": h.segment_id,
                            "road_name": h.road_name,
                            "status": h.operational_status,
                            "type": "hazard"
                        }
                    })
                })
                .collect();

            Ok(Json(json!({
                "type": "FeatureCollection",
                "features": features
            })))
        }
        Err(e) => {
            warn!("Failed to get active hazards: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn apply_hazard(
    State(state): State<AppState>,
    Json(payload): Json<ApplyHazardRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let duration = payload.duration_hours.unwrap_or(4);
    let road_status = match payload.status.to_uppercase().as_str() {
        "OPEN" => RoadStatus::Open,
        "BLOCKED" => RoadStatus::Blocked,
        "FLOODED" => RoadStatus::Flooded,
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    match state
        .state_manager
        .apply_segment_hazard(&payload.segment_id, road_status, duration)
        .await
    {
        Ok(updated) => Ok(Json(json!({
            "status": "ok",
            "updated_segments": updated,
            "segment_id": payload.segment_id
        }))),
        Err(e) => {
            warn!("Failed to apply segment hazard: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn clear_hazards(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state
        .state_manager
        .clear_operational_hazards("aluva-periyar-pilot")
        .await
    {
        Ok(cleared) => Ok(Json(json!({
            "status": "ok",
            "cleared_segments": cleared
        }))),
        Err(e) => {
            warn!("Failed to clear hazards: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn handle_dispatch(
    State(state): State<AppState>,
    Json(req): Json<DispatchRequest>,
) -> Result<Json<DispatchResponse>, StatusCode> {
    let mut alert = SosAlert::new(
        format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
        req.raw_text,
    );
    alert.source_channel = req.source_channel.or_else(|| Some("web_dispatcher_api".to_string()));
    alert.latitude = req.latitude;
    alert.longitude = req.longitude;
    alert.accuracy_m = req.accuracy_m;

    let result = state.orchestrator.process_alert(alert.clone()).await;

    let response = build_dispatch_response(
        &result.state,
        &result.incident,
        result.persistence_healthy,
        result.duplicate_suppressed,
        &state.state_manager,
    )
    .await;

    // Also broadcast to SSE subscribers
    if let Ok(json_str) = serde_json::to_string(&response) {
        let _ = state.event_tx.send(json_str);
    }

    Ok(Json(response))
}

async fn build_dispatch_response(
    final_state: &CrisisState,
    incident: &Incident,
    control_plane_persisted: bool,
    duplicate_suppressed: bool,
    state_mgr: &StateManager,
) -> DispatchResponse {
    let route_nodes = final_state.computed_path.clone().unwrap_or_default();
    let route_coords = if !route_nodes.is_empty() {
        state_mgr
            .get_junction_coordinates(&route_nodes)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut features = Vec::new();

    let dispatch_status = final_state
        .brief
        .as_ref()
        .map(|b| b.status.clone())
        .unwrap_or(DispatchStatus::EscalateHumanDispatcher);

    let is_detour = final_state.detour_reason.is_some();

    // 1. Route Polyline Features (Multi-Modal Legs or Single Route)
    if let Some(ref plan) = final_state.multimodal_plan {
        if plan.requires_mitigation {
            for leg in &plan.legs {
                let leg_coords = state_mgr
                    .get_junction_coordinates(&leg.waypoints)
                    .await
                    .unwrap_or_default();

                if leg_coords.len() >= 2 {
                    let (stroke, dash_style, mode_label) = match leg.mode {
                        TransportMode::RoadVehicle => ("#10b981", "solid", "Road Vehicle Transit"),
                        TransportMode::Watercraft => ("#06b6d4", "dashed", "Rescue Boat Water Corridor"),
                        TransportMode::FootWading => ("#f59e0b", "dotted", "Foot / Stretcher Transfer"),
                        TransportMode::Aerial => ("#a855f7", "dashed", "Direct Aerial Evacuation"),
                    };

                    features.push(json!({
                        "type": "Feature",
                        "geometry": {
                            "type": "LineString",
                            "coordinates": leg_coords
                        },
                        "properties": {
                            "type": "route_leg",
                            "leg_index": leg.leg_index,
                            "mode": format!("{:?}", leg.mode),
                            "mode_label": mode_label,
                            "distance_km": leg.distance_km,
                            "travel_time_s": leg.travel_time_s,
                            "is_hazard": leg.is_hazard_stretch,
                            "stroke": stroke,
                            "dash_style": dash_style
                        }
                    }));
                }
            }

            // Staging Point Marker
            if let Some(ref staging_id) = plan.staging_junction {
                if let Ok(coords) = state_mgr.get_junction_coordinates(&[staging_id.clone()]).await {
                    if let Some(coord) = coords.first() {
                        features.push(json!({
                            "type": "Feature",
                            "geometry": {
                                "type": "Point",
                                "coordinates": coord
                            },
                            "properties": {
                                "type": "staging_hub",
                                "junction_id": staging_id,
                                "name": "Dry Staging Hub (Boat Launch & Ambulance Transfer)"
                            }
                        }));
                    }
                }
            }
        } else if route_coords.len() >= 2 {
            features.push(json!({
                "type": "Feature",
                "geometry": {
                    "type": "LineString",
                    "coordinates": route_coords
                },
                "properties": {
                    "type": "route",
                    "distance_km": final_state.total_distance_km.unwrap_or(0.0),
                    "travel_time_s": final_state.total_travel_time_s.unwrap_or(0.0),
                    "status": format!("{:?}", dispatch_status),
                    "stroke": "#10b981",
                    "is_detour": is_detour
                }
            }));
        }
    } else if route_coords.len() >= 2 {
        features.push(json!({
            "type": "Feature",
            "geometry": {
                "type": "LineString",
                "coordinates": route_coords
            },
            "properties": {
                "type": "route",
                "distance_km": final_state.total_distance_km.unwrap_or(0.0),
                "travel_time_s": final_state.total_travel_time_s.unwrap_or(0.0),
                "status": format!("{:?}", dispatch_status),
                "stroke": if dispatch_status == DispatchStatus::RoutedVerified { "#10b981" } else { "#f59e0b" },
                "is_detour": is_detour
            }
        }));
    }

    // 2. Shelter Origin Feature
    if let Some(ref shelter) = final_state.assigned_shelter {
        let shelter_coords = state_mgr
            .get_junction_coordinates(&[shelter.junction_id.clone()])
            .await
            .unwrap_or_default();
        if let Some(coord) = shelter_coords.first() {
            features.push(json!({
                "type": "Feature",
                "geometry": {
                    "type": "Point",
                    "coordinates": coord
                },
                "properties": {
                    "type": "shelter",
                    "id": shelter.id,
                    "name": shelter.name,
                    "asset": final_state.assigned_asset.clone()
                }
            }));
        }
    }

    // 3. Victim Destination Feature
    if let Some(ref triage) = final_state.triage {
        if let Some(ref jid) = triage.resolved_junction_id {
            let victim_coords = state_mgr
                .get_junction_coordinates(&[jid.clone()])
                .await
                .unwrap_or_default();
            if let Some(coord) = victim_coords.first() {
                features.push(json!({
                    "type": "Feature",
                    "geometry": {
                        "type": "Point",
                        "coordinates": coord
                    },
                    "properties": {
                        "type": "victim",
                        "junction_id": jid,
                        "headcount": triage.headcount,
                        "needed_asset": triage.required_asset.as_ref().map(ToString::to_string)
                    }
                }));
            }
        }
    }

    let geojson = json!({
        "type": "FeatureCollection",
        "features": features
    });

    let assigned_shelter = final_state
        .assigned_shelter
        .as_ref()
        .map(|s| ShelterSummary {
            id: s.id.clone(),
            name: s.name.clone(),
            junction_id: s.junction_id.clone(),
        });

    let headcount = final_state.triage.as_ref().and_then(|t| t.headcount);
    let victim_junction = final_state
        .triage
        .as_ref()
        .and_then(|t| t.resolved_junction_id.clone());

    let brief_text = final_state
        .brief
        .as_ref()
        .map(|b| b.raw_brief_text.clone())
        .unwrap_or_else(|| "No tactical brief generated.".to_string());

    DispatchResponse {
        alert_id: final_state.sos.alert_id.clone(),
        incident_status: incident.status.as_str().to_string(),
        incident_version: incident.version,
        control_plane_persisted,
        duplicate_suppressed,
        review_required: if duplicate_suppressed {
            incident.status == IncidentStatus::ReviewRequired
        } else {
            !control_plane_persisted
                || incident.status == IncidentStatus::ReviewRequired
                || (dispatch_status != DispatchStatus::RoutedVerified
                    && dispatch_status != DispatchStatus::MitigationRequired)
        },
        status: format!("{:?}", dispatch_status),
        victim_junction,
        assigned_shelter,
        assigned_asset: final_state.assigned_asset.clone(),
        headcount,
        triage: final_state.triage.clone(),
        distance_km: final_state.total_distance_km.unwrap_or(0.0),
        travel_time_s: final_state.total_travel_time_s.unwrap_or(0.0),
        is_detour,
        detour_reason: final_state.detour_reason.clone(),
        segment_count: final_state
            .segment_path
            .as_ref()
            .map(|p| p.len())
            .unwrap_or(0),
        tactical_brief: brief_text,
        geojson,
        errors: final_state.errors.clone(),
        multimodal_plan: final_state.multimodal_plan.clone(),
        uncertainty_waterfall: final_state.uncertainty_waterfall.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crisis_graph::models::RouteCostObjective;

    #[test]
    fn persisted_route_projection_contains_line_and_endpoint_evidence() {
        let route = RouteDecision {
            route_id: "route-1".to_string(),
            route_version: 1,
            triage_revision: 1,
            dataset: "aluva-periyar-pilot".to_string(),
            cost_objective: RouteCostObjective::FastestTime,
            origin_shelter_id: "S_TALUK_HOSPITAL".to_string(),
            origin_shelter_name: "Aluva Taluk Hospital".to_string(),
            origin_junction_id: "node/1".to_string(),
            victim_junction_id: "node/3".to_string(),
            assigned_asset_id: "AMBULANCE_UNIT_1".to_string(),
            junction_path: vec![
                "node/1".to_string(),
                "node/2".to_string(),
                "node/3".to_string(),
            ],
            segment_path: vec!["segment/1".to_string(), "segment/2".to_string()],
            total_distance_km: 1.4,
            total_travel_time_s: 180.0,
            detour_reason: None,
            verified_segment_count: 2,
            verified_by: "crisisgraph-pipeline".to_string(),
            verified_at: Utc::now(),
            multimodal_plan: None,
        };
        let coordinates = [[76.35, 10.10], [76.36, 10.11], [76.37, 10.12]];

        let geojson = persisted_route_geojson(&route, &coordinates);

        let features = geojson["features"].as_array().unwrap();
        assert_eq!(features.len(), 3);
        assert_eq!(features[0]["geometry"]["type"], "LineString");
        assert_eq!(features[0]["properties"]["route_id"], "route-1");
        assert_eq!(features[0]["properties"]["verified_segment_count"], 2);
        assert_eq!(features[1]["properties"]["type"], "shelter");
        assert_eq!(features[2]["properties"]["type"], "victim");
    }
}
