use chrono::Utc;
use uuid::Uuid;

use crisis_graph::config::Config;
use crisis_graph::db::{Neo4jClient, RedisClient, StateManager};
use crisis_graph::ingestion::TriageExtractor;
use crisis_graph::models::{DispatchStatus, SosAlert};
use crisis_graph::pipeline::{CrisisOrchestrator, PipelineStages};

#[tokio::test]
async fn test_aluva_pipeline_gps_snapping_and_directed_dispatch() {
    let config = Config::from_env();
    let neo4j = match Neo4jClient::connect(&config).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Neo4j not reachable: {e}. Skipping live pipeline test.");
            return;
        }
    };
    if let Err(e) = neo4j.ping().await {
        eprintln!("Neo4j ping failed: {e}. Skipping live pipeline test.");
        return;
    }

    let redis = match RedisClient::connect(&config).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Redis not reachable: {e}. Skipping live pipeline test.");
            return;
        }
    };

    let state_mgr = StateManager::new(neo4j.clone());

    // 1. Seed Aluva pilot shelters
    state_mgr
        .seed_aluva_shelters()
        .await
        .expect("Seed Aluva shelters");
    let _ = state_mgr
        .clear_operational_hazards("aluva-periyar-pilot")
        .await;

    // 2. Initialize Pipeline
    let extractor = TriageExtractor::new(&config);
    let stages = PipelineStages::new(extractor, state_mgr.clone(), redis);
    let orchestrator = CrisisOrchestrator::new(stages);

    // 3. Create realistic SOS with GPS coordinates near Pump Junction
    let alert = SosAlert {
        alert_id: format!("SOS-ALUVA-{}", Uuid::new_v4().to_string()[..6].to_uppercase()),
        raw_text: "URGENT: Water entering ground floor at GPS coordinates lat: 10.1135, lon: 76.3540 near Pump Junction. 4 persons stranded including elderly cardiac patient. Need ambulance immediately!".to_string(),
        timestamp: Utc::now(),
        source_channel: Some("kerala_disaster_helpline".to_string()),
    };

    // 4. Run End-to-End Orchestrator Pipeline
    let final_state = orchestrator.process_alert(alert).await;

    // 5. Verify Assertions across all 4 stages
    assert!(
        !final_state.has_errors(),
        "Pipeline must not produce fatal errors: {:?}",
        final_state.errors
    );

    // Stage 1 Verification: GPS Snapped to nearest road junction
    let triage = final_state.triage.expect("Triage report generated");
    let victim_junction = triage
        .resolved_junction_id
        .expect("Victim junction resolved");
    assert_eq!(
        victim_junction, "node/7992454789",
        "Must snap to Pump Junction node/7992454789"
    );
    assert_eq!(triage.headcount, Some(4));

    // Stage 2 Verification: Allocated from real Aluva shelter
    let shelter = final_state.assigned_shelter.expect("Shelter assigned");
    assert!(
        shelter.id == "S_ALUVA_TOWNHALL"
            || shelter.id == "S_TALUK_HOSPITAL"
            || shelter.id == "S_UC_COLLEGE",
        "Assigned shelter must be an Aluva facility: {}",
        shelter.id
    );
    assert!(final_state.assigned_asset.is_some());

    // Stage 3 Verification: Directed Route computed
    let path = final_state.computed_path.expect("Computed path must exist");
    let segment_path = final_state.segment_path.expect("Segment path must exist");
    let dist_km = final_state
        .total_distance_km
        .expect("Total distance must exist");
    let time_s = final_state
        .total_travel_time_s
        .expect("Travel time must exist");

    assert!(!path.is_empty(), "Path must have waypoints");
    assert_eq!(path.first().unwrap(), &shelter.junction_id);
    assert_eq!(path.last().unwrap(), &victim_junction);
    assert!(
        !segment_path.is_empty(),
        "Must traverse real OSM road segments"
    );
    assert!(dist_km > 0.0);
    assert!(time_s > 0.0);

    // Stage 4 Verification: Tactical Brief and 100% verified safe
    let brief = final_state.brief.expect("Brief generated");
    assert_eq!(
        brief.status,
        DispatchStatus::RoutedVerified,
        "Route must be certified safe"
    );
    assert_eq!(brief.target_victim_junction, victim_junction);
    assert!(brief.raw_brief_text.contains("TACTICAL DISPATCH ORDER"));
    assert!(brief
        .raw_brief_text
        .contains("100% Segments Independently Verified Safe"));
}

#[tokio::test]
async fn test_aluva_pipeline_landmark_resolution_and_truck_dispatch() {
    let config = Config::from_env();
    let neo4j = match Neo4jClient::connect(&config).await {
        Ok(client) => client,
        Err(_) => return,
    };
    if neo4j.ping().await.is_err() {
        return;
    }
    let redis = match RedisClient::connect(&config).await {
        Ok(client) => client,
        Err(_) => return,
    };

    let state_mgr = StateManager::new(neo4j.clone());
    state_mgr
        .seed_aluva_shelters()
        .await
        .expect("Seed Aluva shelters");

    let extractor = TriageExtractor::new(&config);
    let stages = PipelineStages::new(extractor, state_mgr.clone(), redis);
    let orchestrator = CrisisOrchestrator::new(stages);

    // Alert specifying Pump Junction and evacuation truck
    let alert = SosAlert {
        alert_id: format!("SOS-ALUVA-{}", Uuid::new_v4().to_string()[..6].to_uppercase()),
        raw_text: "URGENT: Flash flood near Pump Junction, 8 passengers stranded. Send evacuation truck to transport them to relief camp!".to_string(),
        timestamp: Utc::now(),
        source_channel: Some("emergency_radio".to_string()),
    };

    let final_state = orchestrator.process_alert(alert).await;
    assert!(
        !final_state.has_errors(),
        "Errors: {:?}",
        final_state.errors
    );

    let triage = final_state.triage.expect("Triage");
    assert_eq!(triage.resolved_junction_id.unwrap(), "node/7992454789");
    assert_eq!(triage.headcount, Some(8));

    let brief = final_state.brief.expect("Brief");
    assert_eq!(brief.status, DispatchStatus::RoutedVerified);
    assert!(brief.total_distance_km > 0.0);
    assert!(!brief.segment_path.is_empty());
}

#[tokio::test]
async fn test_aluva_pipeline_service_alley_truck_restriction_fails_safely() {
    let config = Config::from_env();
    let neo4j = match Neo4jClient::connect(&config).await {
        Ok(client) => client,
        Err(_) => return,
    };
    if neo4j.ping().await.is_err() {
        return;
    }
    let redis = match RedisClient::connect(&config).await {
        Ok(client) => client,
        Err(_) => return,
    };

    let state_mgr = StateManager::new(neo4j.clone());
    state_mgr
        .seed_aluva_shelters()
        .await
        .expect("Seed Aluva shelters");

    let extractor = TriageExtractor::new(&config);
    let stages = PipelineStages::new(extractor, state_mgr.clone(), redis);
    let orchestrator = CrisisOrchestrator::new(stages);

    // Alert requesting heavy truck to Aluva Railway Station narrow service alley (hgv: false in OSM)
    let alert = SosAlert {
        alert_id: format!(
            "SOS-ALUVA-{}",
            Uuid::new_v4().to_string()[..6].to_uppercase()
        ),
        raw_text:
            "URGENT: Evacuate Aluva Railway Station, need heavy evacuation truck for 15 passengers!"
                .to_string(),
        timestamp: Utc::now(),
        source_channel: Some("kerala_police_radio".to_string()),
    };

    let final_state = orchestrator.process_alert(alert).await;

    // Safety verification: The solver refuses to send an oversized heavy truck down a non-HGV service alley
    let brief = final_state.brief.expect("Brief emitted");
    assert_eq!(
        brief.status,
        DispatchStatus::EscalateHumanDispatcher,
        "Oversized heavy truck route on non-HGV road must escalate to human dispatcher"
    );
}
