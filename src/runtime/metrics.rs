use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

use super::Role;

/// Install the process-wide Prometheus recorder and register the baseline
/// role metrics. Call once per process, before any metric is recorded.
pub fn install(role: Role) -> Result<PrometheusHandle, Box<dyn std::error::Error + Send + Sync>> {
    let handle = PrometheusBuilder::new().install_recorder()?;
    metrics::gauge!("crisisgraph_role_up", "role" => role.as_str()).set(1.0);
    Ok(handle)
}

/// Record one heartbeat for the role.
pub fn heartbeat(role: Role) {
    metrics::counter!("crisisgraph_heartbeat_total", "role" => role.as_str()).increment(1);
}

/// Mark the role as going down (scraped during graceful shutdown).
pub fn role_down(role: Role) {
    metrics::gauge!("crisisgraph_role_up", "role" => role.as_str()).set(0.0);
}
