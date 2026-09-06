use crisis_graph::db::PassableEdge;
use crisis_graph::ingestion::SpatialResolver;
use crisis_graph::solver::DeterministicPathfinder;

#[test]
fn test_spatial_landmark_resolution() {
    let resolver = SpatialResolver::new();

    assert_eq!(resolver.resolve("Riverside Community School"), Some("J6".to_string()));
    assert_eq!(resolver.resolve("Town Hall"), Some("J2".to_string()));
    assert_eq!(resolver.resolve("Aluva River Bridge"), Some("J5".to_string()));
    assert_eq!(resolver.resolve("J4"), Some("J4".to_string()));
    assert_eq!(resolver.resolve("Unknown Alien Base"), None);
}

#[test]
fn test_deterministic_pathfinding_clear_path() {
    // Normal graph where J1 -> J2 -> J3 (3.2 + 2.1 = 5.3 km)
    let edges = vec![
        PassableEdge { source: "J1".to_string(), target: "J2".to_string(), weight: 3.2 },
        PassableEdge { source: "J2".to_string(), target: "J3".to_string(), weight: 2.1 },
        PassableEdge { source: "J1".to_string(), target: "J4".to_string(), weight: 4.5 },
        PassableEdge { source: "J4".to_string(), target: "J5".to_string(), weight: 3.8 },
    ];

    let result = DeterministicPathfinder::find_shortest_path(&edges, "J1", "J3").unwrap();
    assert_eq!(result.path, vec!["J1", "J2", "J3"]);
    assert!((result.total_distance_km - 5.3).abs() < 1e-4);
}

#[test]
fn test_deterministic_pathfinding_flooded_detour() {
    // When direct J3-J6 is blocked/flooded, pathfinder routes via J1 -> J4 -> J5 -> J6
    let edges = vec![
        PassableEdge { source: "J1".to_string(), target: "J2".to_string(), weight: 3.2 },
        PassableEdge { source: "J2".to_string(), target: "J3".to_string(), weight: 2.1 },
        PassableEdge { source: "J1".to_string(), target: "J4".to_string(), weight: 4.5 },
        PassableEdge { source: "J4".to_string(), target: "J5".to_string(), weight: 3.8 },
        PassableEdge { source: "J5".to_string(), target: "J6".to_string(), weight: 2.7 },
        // J3-J6 is omitted because it is flooded
    ];

    let result = DeterministicPathfinder::find_shortest_path(&edges, "J1", "J6").unwrap();
    assert_eq!(result.path, vec!["J1", "J4", "J5", "J6"]);
    assert!((result.total_distance_km - 11.0).abs() < 1e-4);
    assert!(result.is_detour);
}

#[test]
fn test_unreachable_destination_fails_safely() {
    let edges = vec![
        PassableEdge { source: "J1".to_string(), target: "J2".to_string(), weight: 3.2 },
    ];

    let result = DeterministicPathfinder::find_shortest_path(&edges, "J1", "J8");
    assert!(result.is_err());
}
