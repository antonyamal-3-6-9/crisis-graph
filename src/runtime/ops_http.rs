use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::{Request, StatusCode},
    routing::get,
    Json, Router,
};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{json, Value};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use super::Role;
use crate::db::{Neo4jClient, RedisClient};

const DEPENDENCY_TIMEOUT: Duration = Duration::from_secs(2);

/// Shared state for the operational endpoints every role exposes.
#[derive(Clone)]
pub struct OpsState {
    pub role: Role,
    pub neo4j: Neo4jClient,
    pub redis: RedisClient,
    pub metrics: PrometheusHandle,
    /// Role-specific conditions `/readyz` also requires (e.g. the graph
    /// snapshot loaded), each set by the role when it becomes true.
    pub gates: Vec<ReadyGate>,
}

/// A named readiness condition owned by a role.
#[derive(Clone)]
pub struct ReadyGate {
    pub name: &'static str,
    pub ready: Arc<AtomicBool>,
}

impl ReadyGate {
    pub fn new(name: &'static str) -> Self {
        Self { name, ready: Arc::new(AtomicBool::new(false)) }
    }

    pub fn set_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
}

/// `/healthz` (liveness), `/readyz` (dependencies reachable) and `/metrics`
/// (Prometheus text), with request tracing applied.
pub fn ops_router(state: OpsState) -> Router {
    instrument(ops_routes(state))
}

/// The operational routes without middleware, for merging with role routes.
pub fn ops_routes(state: OpsState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .with_state(state)
}

/// Give every request an `x-request-id`, record it on the tracing span and
/// echo it in the response.
pub fn instrument(router: Router) -> Router {
    router
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
            let request_id = request
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("-");
            tracing::info_span!(
                "http",
                method = %request.method(),
                uri = %request.uri(),
                request_id = %request_id,
            )
        }))
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}

async fn healthz(State(state): State<OpsState>) -> Json<Value> {
    Json(json!({ "status": "ok", "role": state.role.as_str() }))
}

async fn readyz(State(state): State<OpsState>) -> (StatusCode, Json<Value>) {
    let neo4j = check(state.neo4j.ping()).await;
    let redis = check(state.redis.ping()).await;
    let gates_ready = state.gates.iter().all(ReadyGate::is_ready);
    let ready = neo4j.is_ok() && redis.is_ok() && gates_ready;
    let gates: serde_json::Map<String, Value> = state
        .gates
        .iter()
        .map(|gate| (gate.name.to_string(), json!({ "ok": gate.is_ready() })))
        .collect();

    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "status": if ready { "ready" } else { "not_ready" },
            "role": state.role.as_str(),
            "dependencies": {
                "neo4j": dependency_status(&neo4j),
                "redis": dependency_status(&redis),
            },
            "gates": gates,
        })),
    )
}

async fn metrics(State(state): State<OpsState>) -> String {
    state.metrics.render()
}

async fn check<F>(probe: F) -> Result<(), String>
where
    F: std::future::Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
{
    match tokio::time::timeout(DEPENDENCY_TIMEOUT, probe).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("timed out after {}s", DEPENDENCY_TIMEOUT.as_secs())),
    }
}

fn dependency_status(result: &Result<(), String>) -> Value {
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}
