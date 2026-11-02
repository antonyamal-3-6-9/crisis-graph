//! Logging and distributed tracing (M8, pipeline doc §4.11).
//!
//! Every role installs the same subscriber: compact structured logs, plus an
//! OpenTelemetry layer so spans have W3C trace context. When
//! `OTEL_EXPORTER_OTLP_ENDPOINT` is set (e.g. Jaeger's OTLP/HTTP port
//! `http://localhost:4318`), spans are also exported there, with
//! `service.name = crisisgraph-<role>`.
//!
//! One SOS is one trace: intake starts it and stores its `traceparent` on
//! the report and the outbox event; the relay copies it into the stream
//! message; triage, the matcher and the simulated driver continue it with
//! [`set_parent`]. Incidents and tasks keep the report's `traceparent`.

use std::collections::HashMap;

use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::{TraceContextExt, TracerProvider as _};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

const DEFAULT_FILTER: &str = "crisis_graph=info,crisisgraph=info,tower_http=info";

/// Keeps the tracer provider alive; flushes spans on drop.
pub struct Telemetry {
    provider: SdkTracerProvider,
    pub exporting_to: Option<String>,
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        let _ = self.provider.shutdown();
    }
}

/// Install logging and tracing for `service` (e.g. `crisisgraph-api`).
pub fn init(service: &str) -> Result<Telemetry, Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok().filter(|e| !e.trim().is_empty());
    let provider = tracer_provider(service, endpoint.as_deref())?;
    let tracer = provider.tracer("crisisgraph");
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| DEFAULT_FILTER.into()))
        .with(tracing_subscriber::fmt::layer().with_target(true).compact())
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()?;
    Ok(Telemetry { provider, exporting_to: endpoint })
}

/// A tracer provider; with an endpoint, spans are batched and exported over
/// OTLP/HTTP, otherwise they only carry trace context.
pub fn tracer_provider(service: &str, endpoint: Option<&str>) -> Result<SdkTracerProvider, Box<dyn std::error::Error + Send + Sync>> {
    let resource = Resource::builder().with_service_name(service.to_string()).build();
    let mut builder = SdkTracerProvider::builder().with_resource(resource);
    if let Some(endpoint) = endpoint {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
            .build()?;
        builder = builder.with_batch_exporter(exporter);
    }
    Ok(builder.build())
}

/// The W3C `traceparent` of `span`, if it has trace context.
pub fn traceparent(span: &Span) -> Option<String> {
    let mut carrier = HashMap::new();
    TraceContextPropagator::new().inject_context(&span.context(), &mut carrier);
    carrier.remove("traceparent")
}

/// Continue the trace in `traceparent` (if valid) as the parent of `span`.
pub fn set_parent(span: &Span, traceparent: Option<&str>) {
    let Some(value) = traceparent.filter(|v| !v.is_empty()) else { return };
    let carrier = HashMap::from([("traceparent".to_string(), value.to_string())]);
    let context = TraceContextPropagator::new().extract(&carrier);
    if context.span().span_context().is_valid() {
        span.set_parent(context);
    }
}

/// The trace ID of `span` as hex, for log lines.
pub fn trace_id(span: &Span) -> Option<String> {
    let context = span.context();
    let span_context = context.span().span_context().clone();
    span_context.is_valid().then(|| span_context.trace_id().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::info_span;

    #[test]
    fn traceparent_round_trips_into_a_child_span() {
        let provider = tracer_provider("test", None).unwrap();
        let subscriber = tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(provider.tracer("t")));
        tracing::subscriber::with_default(subscriber, || {
            let root = info_span!("intake");
            let header = traceparent(&root).expect("root has context");
            assert!(header.starts_with("00-") && header.len() == 55, "{header}");

            let child = info_span!("triage");
            set_parent(&child, Some(&header));
            assert_eq!(trace_id(&child), trace_id(&root), "same trace");
            assert_ne!(traceparent(&child), Some(header.clone()), "its own span ID");

            let orphan = info_span!("orphan");
            set_parent(&orphan, Some("garbage"));
            assert_ne!(trace_id(&orphan), trace_id(&root));
        });
    }
}
