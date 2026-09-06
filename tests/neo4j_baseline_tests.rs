use neo4rs::query;

use crisis_graph::config::Config;
use crisis_graph::db::{Neo4jClient, StateManager};
use crisis_graph::models::{AssetType, RoadStatus};
use crisis_graph::solver::{DeterministicPathfinder, PathfinderError, RoutingQuery};

#[tokio::test]
async fn test_neo4j_pilot_graph_and_operational_overlay() {
    let config = Config::from_env();
    let neo4j = match Neo4jClient::connect(&config).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Neo4j not reachable at {}: {e}. Skipping live test.", config.neo4j_uri);
            return;
        }
    };

    if let Err(e) = neo4j.ping().await {
        eprintln!("Neo4j ping failed: {e}. Skipping live test.");
        return;
    }

    let dataset = "aluva-periyar-pilot";

    // 1. Verify expected counts in Neo4j
    let count_j_q = query("MATCH (j:Junction {dataset: $dataset}) RETURN count(j) AS c")
        .param("dataset", dataset);
    let mut row_j = neo4j.graph.execute(count_j_q).await.expect("query junctions");
    let junction_count: i64 = row_j.next().await.unwrap().unwrap().get("c").unwrap();

    let count_r_q = query("MATCH ()-[r:CONNECTS_TO {dataset: $dataset}]->() RETURN count(r) AS c")
        .param("dataset", dataset);
    let mut row_r = neo4j.graph.execute(count_r_q).await.expect("query segments");
    let segment_count: i64 = row_r.next().await.unwrap().unwrap().get("c").unwrap();

    assert_eq!(junction_count, 21256, "Must have 21,256 junctions in Neo4j");
    assert_eq!(segment_count, 48626, "Must have 48,626 total road segments in Neo4j");

    // 2. Fetch directed graph snapshot from Neo4j via StateManager
    let state_mgr = StateManager::new(neo4j.clone());

    // Ensure clean operational state initially
    let _ = state_mgr.clear_operational_hazards(dataset).await;

    let baseline_edges = state_mgr.get_passable_directed_subgraph(dataset).await
        .expect("Fetch passable directed edges from Neo4j");
    // In Aluva pilot: 47,461 OPEN + 579 RESTRICTED = 48,040 passable edges (586 UNSUITABLE excluded)
    assert_eq!(baseline_edges.len(), 48040, "48,040 edges should be passable initially");

    // 3. Run multi-modal pathfinding on live Neo4j edges
    // Aluva test nodes:
    let origin = "node/343716201";
    let destination = "node/343716107";

    let clear_query = RoutingQuery::new(origin, destination)
        .with_asset_type(AssetType::Ambulance);

    let route_clear = DeterministicPathfinder::find_directed_route(&baseline_edges, &clear_query)
        .expect("Clear ambulance route should succeed");
    assert_eq!(route_clear.path[0], origin);
    assert_eq!(*route_clear.path.last().unwrap(), destination);
    assert_eq!(
        route_clear.segment_path,
        vec!["way/1009141757/seg/0/fwd", "way/1009141757/seg/1/fwd"]
    );

    // 4. Apply dynamic hazard to Neo4j (Periyar flood event)
    let flood_segment = "way/1009141757/seg/0/fwd";
    let updated_count = state_mgr.apply_segment_hazard(flood_segment, RoadStatus::Flooded, 6)
        .await
        .expect("Apply hazard in Neo4j");
    assert_eq!(updated_count, 1, "Should update exactly 1 segment in Neo4j");

    // 5. Query Neo4j again: flooded edge should be filtered out by DB query
    let post_flood_edges = state_mgr.get_passable_directed_subgraph(dataset).await
        .expect("Fetch updated passable edges");
    assert_eq!(
        post_flood_edges.len(),
        48039,
        "Passable edges count should be 48,039 (1 flooded edge excluded)"
    );
    assert!(
        !post_flood_edges.iter().any(|e| e.segment_id == flood_segment),
        "Flooded segment must not appear in passable edges"
    );

    // 6. Pathfinding with flooded edge missing in Neo4j snapshot
    let route_after_flood = DeterministicPathfinder::find_directed_route(&post_flood_edges, &clear_query);
    match route_after_flood {
        Ok(detour) => {
            assert!(!detour.segment_path.contains(&flood_segment.to_string()));
            assert!(detour.is_detour);
        }
        Err(PathfinderError::NoPassableRouteFound { .. }) => {
            // Fails closed safely if no alternative route exists
        }
        Err(e) => panic!("Unexpected error during flood routing: {:?}", e),
    }

    // 7. Cleanup: clear operational hazards
    let cleared = state_mgr.clear_operational_hazards(dataset).await
        .expect("Clear operational hazards");
    assert_eq!(cleared, 1, "Should have cleared 1 hazard");

    // Verify pristine state restored
    let restored_edges = state_mgr.get_passable_directed_subgraph(dataset).await
        .expect("Fetch restored edges");
    assert_eq!(restored_edges.len(), 48040, "All 48,040 passable edges restored");
}
