//! Exercise only the configured OpenAI-compatible triage extraction boundary.
//! This does not allocate resources, calculate a route, or mutate hazards.

use crisis_graph::config::Config;
use crisis_graph::ingestion::TriageExtractor;
use crisis_graph::models::SosAlert;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let report = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if report.trim().is_empty() {
        return Err("Usage: cargo run --bin triage_probe -- '<emergency report>'".into());
    }

    let config = Config::from_env();
    let extractor = TriageExtractor::new(&config);
    let alert = SosAlert::new(
        format!("PROBE-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
        report
    );

    let candidate = extractor.extract(&alert).await?;
    println!("{}", serde_json::to_string_pretty(&candidate)?);
    Ok(())
}
