use crisis_graph::config::Config;
use crisis_graph::db::{Neo4jClient, RedisClient, StateManager};
use neo4rs::query;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    info!("Starting CrisisGraph database cleanup & reset...");

    let config = Config::from_env();
    let neo4j = Neo4jClient::connect(&config).await?;
    let state_mgr = StateManager::new(neo4j.clone());
    let redis = RedisClient::connect(&config).await?;

    // 1. Delete all Incidents and Audit Events
    info!("Deleting Incident and IncidentAuditEvent nodes...");
    neo4j.graph.run(query("MATCH (i:Incident) DETACH DELETE i")).await?;
    neo4j.graph.run(query("MATCH (e:IncidentAuditEvent) DETACH DELETE e")).await?;

    // 2. Delete all Resource Reservations
    info!("Deleting ResourceReservation nodes...");
    neo4j.graph.run(query("MATCH (r:ResourceReservation) DETACH DELETE r")).await?;

    // 3. Delete hazard records (and their audit), any old-style hazard
    // properties on road segments, and legacy HazardEvent nodes. The overlay
    // version only ever increases, so it is bumped to make running graph
    // snapshots reload.
    info!("Deleting hazard records and clearing old road-segment hazard properties...");
    neo4j.graph.run(query(
        "MATCH (h:Hazard) OPTIONAL MATCH (h)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE h, a"
    )).await?;
    neo4j.graph.run(query(
        "MATCH ()-[r:CONNECTS_TO]->() \
         WHERE r.operational_status IS NOT NULL \
         REMOVE r.operational_status, r.valid_until"
    )).await?;
    neo4j.graph.run(query(
        "MATCH (o:OverlayState) SET o.overlay_version = o.overlay_version + 1, o.updated_at = datetime()"
    )).await?;
    neo4j.graph.run(query("MATCH (h:HazardEvent) DETACH DELETE h")).await?;

    // 4. Reset shelters to clean baseline occupancy & full fleet
    info!("Resetting Aluva shelters to baseline capacities and fleet levels...");
    state_mgr.seed_aluva_shelters().await?;

    // 4b. Reset the operational model (new pipeline): drop transient records,
    // return every vehicle to AVAILABLE. Vehicle records themselves are kept.
    info!("Resetting operational records (reports, tasks, reservations, outbox) and vehicles...");
    for label in ["Reservation", "Task", "Report", "OpsIncident", "OutboxEvent", "ProcessedEvent", "AuditEvent"] {
        neo4j
            .graph
            .run(query(&format!("MATCH (n:{label}) DETACH DELETE n")))
            .await?;
    }
    neo4j
        .graph
        .run(query(
            "MATCH (v:Vehicle) SET v.status = 'AVAILABLE', v.version = v.version + 1, \
             v.updated_at = datetime()",
        ))
        .await?;

    // 5. Clean Redis stream and any lingering locks
    info!("Cleaning Redis emergency streams and locks...");
    let mut conn = redis.manager.clone();
    let _: Result<(), _> = redis::cmd("DEL")
        .arg("sos:stream:aluva")
        .query_async(&mut conn)
        .await;

    // Scan and delete locks
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg("lock:*")
        .query_async(&mut conn)
        .await
        .unwrap_or_default();

    for key in keys {
        let _: Result<(), _> = redis::cmd("DEL")
            .arg(&key)
            .query_async(&mut conn)
            .await;
    }

    info!("Database cleanup completed successfully! All incidents, hazards, and reservations purged; shelters restored to 100% capacity.");
    Ok(())
}
