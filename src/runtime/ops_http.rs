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
}

/// `/healthz` (liveness), `/readyz` (dependencies reachable) and `/metrics`
/// (Prometheus text). Requests get an `x-request-id` that is recorded on the
/// tracing span and echoed in the response.
pub fn ops_router(state: OpsState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .with_state(state)
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
    let ready = neo4j.is_ok() && redis.is_ok();

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
            }
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
