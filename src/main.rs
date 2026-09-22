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
    info!(
        "Target Redis URI : {}:{}",
        config.redis_host, config.redis_port
    );
    info!("Inference backend: {}", config.inference_backend);
    info!("Inference URL    : {}", config.inference_base_url);
    info!("Inference model  : {}", config.inference_model);
    info!("Triage adapter   : {:?}", config.triage_adapter_id);

    // 3. Connect to Database and Cache
    let neo4j = Neo4jClient::connect(&config).await?;
    info!("Connected to Neo4j successfully.");

    let redis = RedisClient::connect(&config).await?;
    info!("Connected to Redis/Valkey successfully.");

    // 4. Seed Database if specified via command line or ensure Aluva shelters exist
    let args: Vec<String> = env::args().collect();
    let state_manager = StateManager::new(neo4j.clone());

    if args.iter().any(|arg| arg == "--seed-demo") {
        info!("Seeding legacy prototype demo graph (J1-J8)...");
        seed_database(&neo4j).await?;
    } else {
        info!("Ensuring Aluva pilot shelters are registered in Neo4j...");
        state_manager.seed_aluva_shelters().await?;
    }

    // 5. Initialize Components
    let extractor = TriageExtractor::new(&config);
    let stages = PipelineStages::new(extractor, state_manager, redis);
    let orchestrator = CrisisOrchestrator::new(stages);

    // 6. Simulate an Emergency Disaster SOS Stream (Aluva–Periyar Pilot Area)
    let sample_alerts = vec![
        SosAlert {
            alert_id: format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
            raw_text: "URGENT: Periyar river rising rapidly at (lat: 10.1135, lon: 76.3540) near Pump Junction. 4 persons trapped on 1st floor, elderly patient needs ambulance immediately!".to_string(),
            timestamp: Utc::now(),
            source_channel: Some("kerala_disaster_helpline".to_string()),
        },
        SosAlert {
            alert_id: format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
            raw_text: "Flash flood near Aluva Railway Station, 8 passengers stranded. Send evacuation truck to transport them to relief camp!".to_string(),
            timestamp: Utc::now(),
            source_channel: Some("railway_police_radio".to_string()),
        },
        SosAlert {
            alert_id: format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
            raw_text: "Water entering Aluva Manappuram temple grounds, 12 pilgrims stranded on temple steps. Urgent boat rescue needed!".to_string(),
            timestamp: Utc::now(),
            source_channel: Some("fire_rescue_vhf".to_string()),
        },
    ];

    for alert in sample_alerts {
        info!("Processing Incoming Alert: {}", alert.alert_id);
        let final_state = orchestrator.process_alert(alert).await;

        if let Some(brief) = final_state.brief {
            println!("\n{}\n", brief.raw_brief_text);
        } else {
            eprintln!(
                "Failed to generate tactical brief: {:?}",
                final_state.errors
            );
        }
    }

    info!("CrisisGraph pipeline execution finished.");
    Ok(())
}
