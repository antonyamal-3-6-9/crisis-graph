use chrono::Utc;
use std::env;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use uuid::Uuid;

use crisis_graph::config::Config;
use crisis_graph::db::{seed_database, Neo4jClient, RedisClient, StateManager};
use crisis_graph::ingestion::TriageExtractor;
use crisis_graph::models::SosAlert;
use crisis_graph::pipeline::{CrisisOrchestrator, PipelineStages};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Initialize Tracing Subscriber
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set tracing default subscriber");

    info!("============================================================");
    info!("   CrisisGraph Air-Gapped Engine Initializing (Rust)        ");
    info!("============================================================");

    // 2. Load Configuration
    let config = Config::from_env();
    info!("Target Neo4j URI : {}", config.neo4j_uri);
    info!("Target Redis URI : {}:{}", config.redis_host, config.redis_port);
    info!("Target vLLM URL  : {}", config.vllm_base_url);

    // 3. Connect to Database and Cache
    let neo4j = Neo4jClient::connect(&config).await?;
    info!("Connected to Neo4j successfully.");

    let redis = RedisClient::connect(&config).await?;
    info!("Connected to Redis/Valkey successfully.");

    // 4. Seed Database if specified via command line or by default
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--seed" || arg == "-s") || true {
        info!("Running database seed script...");
        seed_database(&neo4j).await?;
    }

    // 5. Initialize Components
    let state_manager = StateManager::new(neo4j.clone());
    let extractor = TriageExtractor::new(&config);
    let stages = PipelineStages::new(extractor, state_manager, redis);
    let orchestrator = CrisisOrchestrator::new(stages);

    // 6. Simulate an Emergency Disaster SOS Stream
    let sample_alerts = vec![
        SosAlert {
            alert_id: format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
            raw_text: "URGENT: Flash flood at Riverside Community School, 14 people trapped on roof. Need rescue boat immediately! Road J3-J6 is completely underwater and flooded.".to_string(),
            timestamp: Utc::now(),
            source_channel: Some("citizen_radio_mesh".to_string()),
        },
        SosAlert {
            alert_id: format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
            raw_text: "Landslide near Aluva River Bridge, 4 injured civilians need immediate ambulance evacuation to Highland camp.".to_string(),
            timestamp: Utc::now(),
            source_channel: Some("vhf_repeater".to_string()),
        },
    ];

    for alert in sample_alerts {
        info!("Processing Incoming Alert: {}", alert.alert_id);
        let final_state = orchestrator.process_alert(alert).await;

        if let Some(brief) = final_state.brief {
            println!("\n{}\n", brief.raw_brief_text);
        } else {
            eprintln!("Failed to generate tactical brief: {:?}", final_state.errors);
        }
    }

    info!("CrisisGraph pipeline execution finished.");
    Ok(())
}
