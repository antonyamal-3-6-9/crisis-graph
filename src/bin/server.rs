use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        Json,
    },
    routing::{get, post},
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
use tower_http::services::ServeDir;
use tracing::{error, info, warn};
use uuid::Uuid;

use crisis_graph::config::Config;
use crisis_graph::db::{Neo4jClient, RedisClient, ShelterWithLocation, StateManager};
use crisis_graph::ingestion::TriageExtractor;
use crisis_graph::models::{CrisisState, DispatchStatus, RoadStatus, SosAlert, TriageReport};
use crisis_graph::pipeline::{CrisisOrchestrator, PipelineStages};

const STREAM_KEY: &str = "sos:stream:aluva";
const CONSUMER_GROUP: &str = "crisisgraph_workers";

#[derive(Clone)]
struct AppState {
    orchestrator: CrisisOrchestrator,
    state_manager: StateManager,
    redis: RedisClient,
    event_tx: Arc<broadcast::Sender<String>>,
}

#[derive(Debug, Deserialize)]
pub struct DispatchRequest {
    pub raw_text: String,
    pub source_channel: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DispatchResponse {
    pub alert_id: String,
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
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShelterSummary {
    pub id: String,
    pub name: String,
    pub junction_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ApplyHazardRequest {
    pub segment_id: String,
    pub status: String,
    pub duration_hours: Option<u32>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    info!("Starting CrisisGraph Emergency Dispatch Server...");

    let config = Config::from_env();
    let neo4j = Neo4jClient::connect(&config).await?;
    neo4j.ping().await?;

    let redis = RedisClient::connect(&config).await?;
    let state_manager = StateManager::new(neo4j.clone());

    // Ensure Aluva shelters are registered
    state_manager.seed_aluva_shelters().await?;

    // Setup Redis Streams consumer group
    if let Err(e) = redis
        .ensure_consumer_group(STREAM_KEY, CONSUMER_GROUP)
        .await
    {
        warn!("Consumer group setup note: {e}");
    }

    let extractor = TriageExtractor::new(&config);
    let stages = PipelineStages::new(extractor, state_manager.clone(), redis.clone());
    let orchestrator = CrisisOrchestrator::new(stages);

    // Event broadcast channel for real-time SSE streaming to web dashboard
    let (event_tx, _) = broadcast::channel::<String>(256);
    let event_tx_arc = Arc::new(event_tx);

    let app_state = AppState {
        orchestrator: orchestrator.clone(),
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

    let router = Router::new()
        // API Routes
        .route("/api/v1/health", get(health_check))
        .route("/api/v1/shelters", get(get_shelters))
        .route("/api/v1/shelters/reset", post(reset_shelters))
        .route("/api/v1/hazards", get(get_hazards))
        .route("/api/v1/hazards", post(apply_hazard))
        .route("/api/v1/hazards/clear", post(clear_hazards))
        .route("/api/v1/dispatch", post(handle_dispatch))
        // Event-Driven Stream & Real-time Endpoints
        .route("/api/v1/events", get(sse_events))
        .route("/api/v1/sos/stream", post(ingest_sos_stream))
        .route("/api/v1/sos/simulate_spike", post(simulate_spike))
        // Serve Web Assets from `web/` folder
        .fallback_service(ServeDir::new("web"))
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
    axum::serve(listener, router).await?;

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
    let alert = SosAlert {
        alert_id: format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
        raw_text: req.raw_text,
        timestamp: Utc::now(),
        source_channel: req
            .source_channel
            .or_else(|| Some("redis_streams_intake".to_string())),
    };

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
        let alert = SosAlert {
            alert_id: format!(
                "SOS-SPIKE-{}",
                Uuid::new_v4().to_string()[..6].to_uppercase()
            ),
            raw_text: alert_text.to_string(),
            timestamp: Utc::now(),
            source_channel: Some("disaster_spike_simulator".to_string()),
        };

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
    orchestrator: CrisisOrchestrator,
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
                            Err(_) => SosAlert {
                                alert_id: format!(
                                    "SOS-{}",
                                    Uuid::new_v4().to_string()[..8].to_uppercase()
                                ),
                                raw_text: payload_str,
                                timestamp: Utc::now(),
                                source_channel: Some("stream_fallback".to_string()),
                            },
                        };

                        // Process through the full 4-stage pipeline
                        let final_state = orchestrator.process_alert(alert).await;
                        let response = build_dispatch_response(&final_state, &state_mgr).await;

                        // Broadcast to connected web clients (SSE)
                        if let Ok(json_str) = serde_json::to_string(&response) {
                            let _ = event_tx.send(json_str);
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
        _ => RoadStatus::Flooded,
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
    let alert = SosAlert {
        alert_id: format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
        raw_text: req.raw_text,
        timestamp: Utc::now(),
        source_channel: req
            .source_channel
            .or_else(|| Some("web_dispatcher_api".to_string())),
    };

    let final_state = state.orchestrator.process_alert(alert).await;
    let response = build_dispatch_response(&final_state, &state.state_manager).await;

    // Also broadcast to SSE subscribers
    if let Ok(json_str) = serde_json::to_string(&response) {
        let _ = state.event_tx.send(json_str);
    }

    Ok(Json(response))
}

async fn build_dispatch_response(
    final_state: &CrisisState,
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

    // 1. Route Polyline Feature
    if route_coords.len() >= 2 {
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
    }
}
