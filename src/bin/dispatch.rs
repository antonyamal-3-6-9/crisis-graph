use chrono::Utc;
use std::io::{self, Write};
use uuid::Uuid;

use crisis_graph::config::Config;
use crisis_graph::db::{Neo4jClient, RedisClient, StateManager};
use crisis_graph::ingestion::TriageExtractor;
use crisis_graph::models::SosAlert;
use crisis_graph::pipeline::{CrisisOrchestrator, PipelineStages};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("============================================================");
    println!("     CRISISGRAPH - INTERACTIVE EMERGENCY DISPATCH CLI      ");
    println!("             Aluva–Periyar Pilot Test Console              ");
    println!("============================================================");

    let config = Config::from_env();
    let neo4j = Neo4jClient::connect(&config).await?;
    neo4j.ping().await?;

    let redis = RedisClient::connect(&config).await?;
    let state_manager = StateManager::new(neo4j.clone());

    // Ensure Aluva shelters are registered
    state_manager.seed_aluva_shelters().await?;

    let extractor = TriageExtractor::new(&config);
    let stages = PipelineStages::new(extractor, state_manager, redis);
    let orchestrator = CrisisOrchestrator::new(stages);

    println!("\nConnected to Neo4j (bolt://localhost:7687) & Redis (localhost:6379).");
    println!("Enter an emergency SOS message, or type 'sample', or 'exit' to quit.\n");

    loop {
        print!("crisis-dispatch> ");
        io::stdout().flush()?;

        let mut input = String::new();
        let bytes_read = io::stdin().read_line(&mut input)?;
        if bytes_read == 0 {
            // EOF reached (Ctrl+D or piped input)
            println!("\nEOF received. Exiting dispatcher console.");
            break;
        }
        let trimmed = input.trim();

        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
            println!("Exiting dispatcher console.");
            break;
        }

        let alert_text = if trimmed.eq_ignore_ascii_case("sample") {
            "URGENT: Flash flood at (lat: 10.1135, lon: 76.3540) near Pump Junction. 4 persons trapped, elderly patient needs ambulance immediately!"
        } else {
            trimmed
        };

        let alert = SosAlert {
            alert_id: format!("SOS-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
            raw_text: alert_text.to_string(),
            timestamp: Utc::now(),
            source_channel: Some("interactive_operator_cli".to_string()),
        };

        println!("\n[Processing alert: {}]...", alert.alert_id);
        let final_state = orchestrator.process_alert(alert).await;

        if let Some(brief) = final_state.brief {
            println!("\n{}", brief.raw_brief_text);

            // Print extra inspection details for technical verification
            if let Some(ref triage) = final_state.triage {
                println!("\n--- DIAGNOSTIC INSPECTION ---");
                println!("• Victim Junction  : {:?}", triage.resolved_junction_id);
                println!("• Needed Asset     : {:?}", triage.required_asset);
                println!("• Headcount        : {:?}", triage.headcount);
                println!(
                    "• Inference        : {} / {} / {:?}",
                    triage.inference.backend, triage.inference.model, triage.inference.adapter_id
                );
                if let Some(ref shelter) = final_state.assigned_shelter {
                    println!("• Assigned Shelter : {} ({})", shelter.name, shelter.id);
                }
                if let Some(ref segments) = final_state.segment_path {
                    println!("• Road Segments    : {} total traversed", segments.len());
                    if !segments.is_empty() {
                        println!(
                            "• First 3 Segments : {:?}",
                            &segments[..segments.len().min(3)]
                        );
                    }
                }
                println!("-----------------------------\n");
            }
        } else {
            eprintln!("\nPipeline Error: {:?}", final_state.errors);
        }
    }

    Ok(())
}
