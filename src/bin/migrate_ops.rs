//! Creates the operational data model next to the legacy one (M1).
//!
//! Idempotent: creates constraints and indexes, the hazard `OverlayState`,
//! and vehicle records for the baseline fleet, and moves hazards stored on
//! road segments into hazard records (M4). Existing vehicles, legacy
//! reservations and legacy shelter counts are not modified.
//!
//! Example:
//! cargo run --bin migrate_ops

use neo4rs::query;
use tracing::info;

use crisis_graph::config::Config;
use crisis_graph::db::{
    ensure_ops_schema, FleetCapacities, Neo4jClient, OpsRepository, StateManager,
};

const DATASET: &str = "aluva-periyar-pilot";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env();
    let neo4j = Neo4jClient::connect(&config).await?;
    neo4j.ping().await?;

    print_counts(&neo4j, "before").await?;

    ensure_ops_schema(&neo4j).await?;
    info!("Operational constraints and indexes ensured");

    // Shelters must exist before vehicles can be based at them.
    StateManager::new(neo4j.clone()).ensure_aluva_shelters().await?;

    neo4j
        .graph
        .run(
            query(
                "MERGE (o:OverlayState {dataset: $dataset}) \
                 ON CREATE SET o.overlay_version = 0, o.updated_at = datetime()",
            )
            .param("dataset", DATASET),
        )
        .await?;

    let migrated = OpsRepository::new(neo4j.clone()).migrate_segment_hazards().await?;
    info!(migrated, "Road-segment hazards moved into hazard records");

    let created = OpsRepository::new(neo4j.clone())
        .seed_baseline_fleet(FleetCapacities::default())
        .await?;
    info!(created, "Baseline fleet vehicles created (existing vehicles unchanged)");

    print_counts(&neo4j, "after").await?;
    Ok(())
}

async fn print_counts(
    neo4j: &Neo4jClient,
    label: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut result = neo4j
        .graph
        .execute(query(
            "MATCH (v:Vehicle) WITH v.home_shelter_id AS shelter, v.asset_type AS asset, count(*) AS n \
             RETURN shelter, asset, n ORDER BY shelter, asset",
        ))
        .await?;
    println!("Vehicles ({label}):");
    let mut total = 0;
    while let Some(row) = result.next().await? {
        let shelter: String = row.get("shelter")?;
        let asset: String = row.get("asset")?;
        let n: i64 = row.get("n")?;
        total += n;
        println!("  {shelter:<22} {asset:<12} {n}");
    }
    println!("  total: {total}");
    Ok(())
}
