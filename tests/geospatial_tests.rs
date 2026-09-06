use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[test]
fn test_aluva_processed_staging_artifacts() {
    let processed_dir = Path::new("data/processed/aluva-periyar-pilot");
    if !processed_dir.exists() {
        eprintln!("Processed pilot data not found. Skipping artifact verification.");
        return;
    }

    let junctions_path = processed_dir.join("junctions.geojson");
    let segments_path = processed_dir.join("road_segments.geojson");
    let report_path = processed_dir.join("import_report.json");

    assert!(junctions_path.exists(), "junctions.geojson must exist");
    assert!(segments_path.exists(), "road_segments.geojson must exist");
    assert!(report_path.exists(), "import_report.json must exist");

    // 1. Verify import_report.json
    let report_content = fs::read_to_string(&report_path).expect("Read report");
    let report: Value = serde_json::from_str(&report_content).expect("Parse report JSON");

    let val = &report["validation"];
    assert_eq!(val["passed"].as_bool(), Some(true), "Validation must pass");
    assert_eq!(val["dangling_endpoints_count"].as_u64(), Some(0));
    assert_eq!(val["duplicate_segment_ids_count"].as_u64(), Some(0));
    assert_eq!(val["zero_length_segments_count"].as_u64(), Some(0));

    let total_junctions = report["total_junctions_count"].as_u64().unwrap();
    let total_segments = report["total_directed_segments_count"].as_u64().unwrap();
    assert!(total_junctions > 20000, "Junctions count: {total_junctions}");
    assert!(total_segments > 40000, "Segments count: {total_segments}");

    // 2. Verify junctions.geojson
    let junctions_content = fs::read_to_string(&junctions_path).expect("Read junctions");
    let junctions_doc: Value = serde_json::from_str(&junctions_content).expect("Parse junctions GeoJSON");
    assert_eq!(junctions_doc["type"].as_str(), Some("FeatureCollection"));

    let junction_features = junctions_doc["features"].as_array().expect("Features array");
    assert_eq!(junction_features.len() as u64, total_junctions);

    let mut junction_ids: HashSet<String> = HashSet::new();
    for j in junction_features {
        assert_eq!(j["type"].as_str(), Some("Feature"));
        let id = j["id"].as_str().expect("Feature id");
        assert!(id.starts_with("node/"));
        junction_ids.insert(id.to_string());

        let geom = &j["geometry"];
        assert_eq!(geom["type"].as_str(), Some("Point"));
        let coords = geom["coordinates"].as_array().expect("Point coordinates");
        assert_eq!(coords.len(), 2);
    }

    // 3. Verify road_segments.geojson
    let segments_content = fs::read_to_string(&segments_path).expect("Read road segments");
    let segments_doc: Value = serde_json::from_str(&segments_content).expect("Parse segments GeoJSON");
    assert_eq!(segments_doc["type"].as_str(), Some("FeatureCollection"));

    let segment_features = segments_doc["features"].as_array().expect("Features array");
    assert_eq!(segment_features.len() as u64, total_segments);

    let mut found_airport_road = false;
    let mut found_kizhakkambalam_rev = false;

    for s in segment_features {
        assert_eq!(s["type"].as_str(), Some("Feature"));
        let props = &s["properties"];

        let from_j = props["from_junction_id"].as_str().expect("from_junction_id");
        let to_j = props["to_junction_id"].as_str().expect("to_junction_id");

        assert!(junction_ids.contains(from_j), "from_junction {from_j} must exist in junctions.geojson");
        assert!(junction_ids.contains(to_j), "to_junction {to_j} must exist in junctions.geojson");

        let length_m = props["length_m"].as_f64().expect("length_m");
        assert!(length_m > 0.0, "Segment length must be positive");

        let geom = &s["geometry"];
        assert_eq!(geom["type"].as_str(), Some("LineString"));
        let coords = geom["coordinates"].as_array().expect("Coordinates");
        assert!(coords.len() >= 2, "LineString must have at least 2 points");

        let way_id = props["osm_way_id"].as_i64().expect("osm_way_id");
        let dir = props["direction"].as_str().expect("direction");

        // Airport Road (way 53020815) is oneway: forward only
        if way_id == 53020815 {
            found_airport_road = true;
            assert_eq!(dir, "forward", "Airport Road must only have forward segments");
        }

        // Kizhakkambalam bus stand road (way 623556453) is oneway=-1: reverse only
        if way_id == 623556453 {
            found_kizhakkambalam_rev = true;
            assert_eq!(dir, "reverse", "Kizhakkambalam bus stand road must only have reverse segments");
        }
    }

    assert!(found_airport_road, "Must find Airport Road segment (way 53020815)");
    assert!(found_kizhakkambalam_rev, "Must find Kizhakkambalam bus stand road segment (way 623556453)");
}

#[test]
fn test_aluva_pilot_directed_routing_and_flood_detour() {
    use crisis_graph::geospatial::{AccessFlags, BaselineStatus};
    use crisis_graph::models::AssetType;
    use crisis_graph::solver::{DeterministicPathfinder, PathfinderError, RoutingEdge, RoutingQuery};

    let processed_dir = Path::new("data/processed/aluva-periyar-pilot");
    if !processed_dir.exists() {
        eprintln!("Processed pilot data not found. Skipping real routing test.");
        return;
    }

    let segments_path = processed_dir.join("road_segments.geojson");
    let segments_content = fs::read_to_string(&segments_path).expect("Read road segments");
    let segments_doc: Value = serde_json::from_str(&segments_content).expect("Parse segments GeoJSON");
    let segment_features = segments_doc["features"].as_array().expect("Features array");

    let mut edges = Vec::with_capacity(segment_features.len());
    for s in segment_features {
        let props = &s["properties"];
        let segment_id = props["segment_id"].as_str().unwrap().to_string();
        let source = props["from_junction_id"].as_str().unwrap().to_string();
        let target = props["to_junction_id"].as_str().unwrap().to_string();
        let length_m = props["length_m"].as_f64().unwrap();
        let travel_time_s = props["base_travel_time_s"].as_f64().unwrap();
        let road_class = props["road_class"].as_str().unwrap().to_string();
        let oneway = props["oneway"].as_bool().unwrap();
        let emergency = props["access"]["emergency"].as_bool().unwrap();
        let motorcar = props["access"]["motorcar"].as_bool().unwrap();
        let hgv = props["access"]["hgv"].as_bool().unwrap();
        let status_str = props["baseline_status"].as_str().unwrap();
        let baseline_status = match status_str {
            "OPEN" => BaselineStatus::Open,
            "RESTRICTED" => BaselineStatus::Restricted,
            _ => BaselineStatus::Unsuitable,
        };

        edges.push(RoutingEdge {
            segment_id,
            source,
            target,
            length_m,
            travel_time_s,
            road_class,
            access: AccessFlags {
                motorcar,
                emergency,
                hgv,
            },
            baseline_status,
            oneway,
        });
    }

    // Real junctions near Aluva:
    // way/1009141757/seg/0/fwd: node/343716201 -> node/2436360537
    // way/1009141757/seg/1/fwd: node/2436360537 -> node/343716107
    let origin = "node/343716201";
    let destination = "node/343716107";

    // 1. Clear condition: Ambulance route
    let query_clear = RoutingQuery::new(origin, destination)
        .with_asset_type(AssetType::Ambulance);
    let route_clear = DeterministicPathfinder::find_directed_route(&edges, &query_clear).unwrap();
    assert_eq!(route_clear.path[0], origin);
    assert_eq!(*route_clear.path.last().unwrap(), destination);
    assert!(route_clear.total_distance_km > 0.0);
    assert!(route_clear.total_travel_time_s > 0.0);
    assert_eq!(
        route_clear.segment_path,
        vec!["way/1009141757/seg/0/fwd", "way/1009141757/seg/1/fwd"]
    );

    // 2. Flood condition: block the primary segment
    let mut blocked = HashSet::new();
    blocked.insert("way/1009141757/seg/0/fwd".to_string());
    let query_blocked = RoutingQuery::new(origin, destination)
        .with_asset_type(AssetType::Ambulance)
        .with_blocked_segments(blocked);

    let route_blocked = DeterministicPathfinder::find_directed_route(&edges, &query_blocked);
    match route_blocked {
        Ok(detour) => {
            assert!(!detour.segment_path.contains(&"way/1009141757/seg/0/fwd".to_string()));
            assert!(detour.is_detour);
            assert!(detour.detour_reason.is_some());
        }
        Err(PathfinderError::NoPassableRouteFound { .. }) => {
            // If strictly dead-ended, fails closed safely
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}
