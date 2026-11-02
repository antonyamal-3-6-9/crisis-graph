use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

use super::Role;

/// Buckets for every `*_seconds` histogram (latencies from milliseconds to
/// half an hour), so Prometheus can compute quantiles over any window.
const SECONDS_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1200.0, 1800.0,
];

/// Install the process-wide Prometheus recorder and register the baseline
/// role metrics. Call once per process, before any metric is recorded.
pub fn install(role: Role) -> Result<PrometheusHandle, Box<dyn std::error::Error + Send + Sync>> {
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Suffix("_seconds".into()), SECONDS_BUCKETS)?
        .install_recorder()?;
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
