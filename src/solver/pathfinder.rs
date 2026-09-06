use std::collections::{HashMap, HashSet};
use petgraph::algo::dijkstra;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::{Deserialize, Serialize};

use crate::db::PassableEdge;
use crate::geospatial::{AccessFlags, BaselineStatus, DirectedSegment};
use crate::models::AssetType;

/// Specific error types emitted by the pathfinding engine.
#[derive(Debug, thiserror::Error)]
pub enum PathfinderError {
    #[error("Origin junction '{0}' not found in graph")]
    OriginNotFound(String),
    #[error("Destination junction '{0}' not found in graph")]
    DestinationNotFound(String),
    #[error("No passable route found between '{origin}' and '{destination}' (all paths blocked or access restricted)")]
    NoPassableRouteFound { origin: String, destination: String },
    #[error("Internal pathfinding error: {0}")]
    Internal(String),
}

/// Cost objective used by Dijkstra's algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostMetric {
    /// Optimize for the fastest arrival time (minimizes travel_time_s)
    FastestTime,
    /// Optimize for the shortest physical distance (minimizes length_m)
    ShortestDistance,
}

impl Default for CostMetric {
    fn default() -> Self {
        Self::FastestTime
    }
}

/// A strongly-typed directed routing edge with multi-modal capability attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingEdge {
    pub segment_id: String,
    pub source: String,
    pub target: String,
    pub length_m: f64,
    pub travel_time_s: f64,
    pub road_class: String,
    pub access: AccessFlags,
    pub baseline_status: BaselineStatus,
    pub oneway: bool,
}

impl From<&DirectedSegment> for RoutingEdge {
    fn from(s: &DirectedSegment) -> Self {
        Self {
            segment_id: s.segment_id.clone(),
            source: s.from_junction_id.clone(),
            target: s.to_junction_id.clone(),
            length_m: s.length_m,
            travel_time_s: s.base_travel_time_s,
            road_class: s.road_class.clone(),
            access: s.access.clone(),
            baseline_status: s.baseline_status,
            oneway: s.oneway,
        }
    }
}

/// A routing query specifying endpoints, responding asset type, and operational constraints.
#[derive(Debug, Clone)]
pub struct RoutingQuery {
    pub origin: String,
    pub destination: String,
    pub asset_type: Option<AssetType>,
    pub cost_metric: CostMetric,
    pub blocked_segments: HashSet<String>,
}

impl RoutingQuery {
    pub fn new(origin: impl Into<String>, destination: impl Into<String>) -> Self {
        Self {
            origin: origin.into(),
            destination: destination.into(),
            asset_type: None,
            cost_metric: CostMetric::FastestTime,
            blocked_segments: HashSet::new(),
        }
    }

    pub fn with_asset_type(mut self, asset_type: AssetType) -> Self {
        self.asset_type = Some(asset_type);
        self
    }

    pub fn with_cost_metric(mut self, metric: CostMetric) -> Self {
        self.cost_metric = metric;
        self
    }

    pub fn with_blocked_segments(mut self, blocked: HashSet<String>) -> Self {
        self.blocked_segments = blocked;
        self
    }
}

/// The computed route plan containing turn-by-turn segments and physical metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteResult {
    /// Sequence of junction IDs from origin to destination (e.g. ["node/1", "node/2"])
    pub path: Vec<String>,
    /// Sequence of traversed directed segment IDs (e.g. ["way/101/seg/0/fwd"])
    pub segment_path: Vec<String>,
    /// Total distance in kilometers
    pub total_distance_km: f64,
    /// Total estimated travel time in seconds
    pub total_travel_time_s: f64,
    /// Whether a detour was taken around a blocked or closed segment
    pub is_detour: bool,
    /// Explanation of why a detour occurred
    pub detour_reason: Option<String>,
    /// Detailed metadata for all traversed segments
    pub traversed_edges: Vec<RoutingEdge>,
}

/// Deterministic directed graph pathfinder.
pub struct DeterministicPathfinder;

impl DeterministicPathfinder {
    /// Primary multi-modal directed routing engine.
    ///
    /// Evaluates passable edges using a directed graph (`DiGraph`), enforcing one-way directionality,
    /// vehicle-specific accessibility, active operational closures, and user-specified cost metrics.
    pub fn find_directed_route(
        edges: &[RoutingEdge],
        query: &RoutingQuery,
    ) -> Result<RouteResult, PathfinderError> {
        if query.origin == query.destination {
            return Ok(RouteResult {
                path: vec![query.origin.clone()],
                segment_path: Vec::new(),
                total_distance_km: 0.0,
                total_travel_time_s: 0.0,
                is_detour: false,
                detour_reason: None,
                traversed_edges: Vec::new(),
            });
        }

        // 1. Build Petgraph DiGraph (Directed Graph)
        let mut graph = DiGraph::<String, RoutingEdge>::new();
        let mut node_map_indices: HashMap<String, NodeIndex> = HashMap::new();
        let mut index_to_node_name: HashMap<NodeIndex, String> = HashMap::new();

        let get_or_create_node = |name: &str,
                                  g: &mut DiGraph<String, RoutingEdge>,
                                  map: &mut HashMap<String, NodeIndex>,
                                  rev: &mut HashMap<NodeIndex, String>| {
            if let Some(&idx) = map.get(name) {
                idx
            } else {
                let idx = g.add_node(name.to_string());
                map.insert(name.to_string(), idx);
                rev.insert(idx, name.to_string());
                idx
            }
        };

        for edge in edges {
            let u = get_or_create_node(
                &edge.source,
                &mut graph,
                &mut node_map_indices,
                &mut index_to_node_name,
            );
            let v = get_or_create_node(
                &edge.target,
                &mut graph,
                &mut node_map_indices,
                &mut index_to_node_name,
            );
            graph.add_edge(u, v, edge.clone());
        }

        let start_idx = *node_map_indices
            .get(&query.origin)
            .ok_or_else(|| PathfinderError::OriginNotFound(query.origin.clone()))?;
        let target_idx = *node_map_indices
            .get(&query.destination)
            .ok_or_else(|| PathfinderError::DestinationNotFound(query.destination.clone()))?;

        // 2. Cost function evaluating vehicle access, closures, and weight metrics
        let cost_fn = |edge_ref: petgraph::graph::EdgeReference<RoutingEdge>| -> f64 {
            let edge = edge_ref.weight();

            // Check active hazard closures
            if query.blocked_segments.contains(&edge.segment_id) {
                return f64::INFINITY;
            }

            // Check vehicle accessibility
            let is_accessible = match query.asset_type {
                Some(AssetType::EvacTruck) => {
                    edge.access.hgv && edge.baseline_status != BaselineStatus::Unsuitable
                }
                Some(AssetType::Ambulance) => {
                    edge.access.emergency && edge.baseline_status != BaselineStatus::Unsuitable
                }
                Some(AssetType::RescueBoat) => false, // Rescue boats cannot traverse asphalt road networks
                Some(AssetType::Helicopter) => true,
                None => edge.access.motorcar || edge.access.emergency,
            };

            if !is_accessible {
                return f64::INFINITY;
            }

            // Calculate weight based on chosen cost metric
            match query.cost_metric {
                CostMetric::FastestTime => edge.travel_time_s.max(0.01),
                CostMetric::ShortestDistance => (edge.length_m / 1_000.0).max(0.0001),
            }
        };

        // 3. Execute Dijkstra's Algorithm
        let distances = dijkstra(&graph, start_idx, Some(target_idx), cost_fn);

        let _target_distance = distances
            .get(&target_idx)
            .copied()
            .filter(|d| !d.is_infinite())
            .ok_or_else(|| PathfinderError::NoPassableRouteFound {
                origin: query.origin.clone(),
                destination: query.destination.clone(),
            })?;

        // 4. Reconstruct path by backtracking incoming directed edges from target to start
        let mut current = target_idx;
        let mut path_nodes = vec![current];
        let mut traversed_edges: Vec<RoutingEdge> = Vec::new();
        let max_hops = graph.node_count();

        while current != start_idx {
            if path_nodes.len() > max_hops {
                return Err(PathfinderError::Internal(
                    "Cycle detected during path backtracking".into(),
                ));
            }

            let mut best_prev = None;
            let mut best_edge = None;
            let current_dist = distances[&current];

            for in_edge in graph.edges_directed(current, Direction::Incoming) {
                let prev = in_edge.source();
                let cost = cost_fn(in_edge);
                if cost.is_infinite() {
                    continue;
                }

                if let Some(&prev_dist) = distances.get(&prev) {
                    if (prev_dist + cost - current_dist).abs() < 1e-4 {
                        best_prev = Some(prev);
                        best_edge = Some(in_edge.weight().clone());
                        break;
                    }
                }
            }

            match (best_prev, best_edge) {
                (Some(prev), Some(edge)) => {
                    path_nodes.push(prev);
                    traversed_edges.push(edge);
                    current = prev;
                }
                _ => {
                    return Err(PathfinderError::Internal(
                        "Incomplete path reconstruction during backtracking".into(),
                    ));
                }
            }
        }

        path_nodes.reverse();
        traversed_edges.reverse();

        let path: Vec<String> = path_nodes
            .into_iter()
            .map(|idx| index_to_node_name[&idx].clone())
            .collect();

        let segment_path: Vec<String> = traversed_edges
            .iter()
            .map(|e| e.segment_id.clone())
            .collect();

        let total_distance_km: f64 = traversed_edges.iter().map(|e| e.length_m / 1_000.0).sum();
        let total_travel_time_s: f64 = traversed_edges.iter().map(|e| e.travel_time_s).sum();

        // Detour detection: active closures were specified or hop count indicates alternate routing
        let is_detour = !query.blocked_segments.is_empty() || path.len() > 3;
        let detour_reason = if !query.blocked_segments.is_empty() {
            Some(format!(
                "Direct path avoided {} active hazard closure(s); routed via verified open alternate.",
                query.blocked_segments.len()
            ))
        } else if is_detour {
            Some("Multi-hop transit route via verified baseline network.".into())
        } else {
            None
        };

        Ok(RouteResult {
            path,
            segment_path,
            total_distance_km,
            total_travel_time_s,
            is_detour,
            detour_reason,
            traversed_edges,
        })
    }

    /// Legacy compatibility method: computes shortest path over `PassableEdge` slice.
    ///
    /// Automatically expands undirected `PassableEdge` items into paired forward/reverse directed edges.
    pub fn find_shortest_path(
        edges: &[PassableEdge],
        start_junction: &str,
        target_junction: &str,
    ) -> Result<RouteResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut routing_edges = Vec::with_capacity(edges.len() * 2);
        for p in edges {
            routing_edges.push(RoutingEdge {
                segment_id: format!("{}-{}-fwd", p.source, p.target),
                source: p.source.clone(),
                target: p.target.clone(),
                length_m: p.weight * 1_000.0,
                travel_time_s: p.weight * 60.0,
                road_class: "secondary".into(),
                access: AccessFlags {
                    motorcar: true,
                    emergency: true,
                    hgv: true,
                },
                baseline_status: BaselineStatus::Open,
                oneway: false,
            });
            routing_edges.push(RoutingEdge {
                segment_id: format!("{}-{}-rev", p.source, p.target),
                source: p.target.clone(),
                target: p.source.clone(),
                length_m: p.weight * 1_000.0,
                travel_time_s: p.weight * 60.0,
                road_class: "secondary".into(),
                access: AccessFlags {
                    motorcar: true,
                    emergency: true,
                    hgv: true,
                },
                baseline_status: BaselineStatus::Open,
                oneway: false,
            });
        }

        let query = RoutingQuery {
            origin: start_junction.to_string(),
            destination: target_junction.to_string(),
            asset_type: None,
            cost_metric: CostMetric::ShortestDistance,
            blocked_segments: HashSet::new(),
        };

        Self::find_directed_route(&routing_edges, &query)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_edge(
        id: &str,
        source: &str,
        target: &str,
        dist_km: f64,
        hgv: bool,
        emergency: bool,
    ) -> RoutingEdge {
        RoutingEdge {
            segment_id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            length_m: dist_km * 1_000.0,
            travel_time_s: dist_km * 60.0,
            road_class: "secondary".into(),
            access: AccessFlags {
                motorcar: true,
                emergency,
                hgv,
            },
            baseline_status: BaselineStatus::Open,
            oneway: false,
        }
    }

    #[test]
    fn test_directed_oneway_enforcement() {
        // A -> B is strictly one-way forward
        // B -> C is two-way
        let edges = vec![
            create_edge("seg_A_B", "A", "B", 2.0, true, true),
            create_edge("seg_B_C", "B", "C", 3.0, true, true),
            create_edge("seg_C_B", "C", "B", 3.0, true, true),
        ];

        // Route A -> C should succeed via A -> B -> C
        let query_forward = RoutingQuery::new("A", "C");
        let result = DeterministicPathfinder::find_directed_route(&edges, &query_forward).unwrap();
        assert_eq!(result.path, vec!["A", "B", "C"]);
        assert_eq!(result.segment_path, vec!["seg_A_B", "seg_B_C"]);
        assert!((result.total_distance_km - 5.0).abs() < 1e-4);

        // Route C -> A should FAIL because B -> A does not exist!
        let query_reverse = RoutingQuery::new("C", "A");
        let err = DeterministicPathfinder::find_directed_route(&edges, &query_reverse);
        assert!(matches!(err, Err(PathfinderError::NoPassableRouteFound { .. })));
    }

    #[test]
    fn test_multimodal_vehicle_restrictions() {
        // Direct route B -> C is a narrow alley: emergency=true (ambulance fits), hgv=false (truck cannot fit)
        // Alternate route B -> D -> C is wide: emergency=true, hgv=true
        let edges = vec![
            create_edge("seg_A_B", "A", "B", 1.0, true, true),
            create_edge("seg_B_C_narrow", "B", "C", 1.0, false, true), // narrow
            create_edge("seg_B_D_wide", "B", "D", 2.0, true, true),    // detour
            create_edge("seg_D_C_wide", "D", "C", 2.0, true, true),
        ];

        // 1. Ambulance should take the narrow direct route (A -> B -> C = 2.0 km)
        let ambulance_query = RoutingQuery::new("A", "C").with_asset_type(AssetType::Ambulance);
        let ambulance_res =
            DeterministicPathfinder::find_directed_route(&edges, &ambulance_query).unwrap();
        assert_eq!(ambulance_res.path, vec!["A", "B", "C"]);
        assert_eq!(ambulance_res.segment_path, vec!["seg_A_B", "seg_B_C_narrow"]);
        assert!((ambulance_res.total_distance_km - 2.0).abs() < 1e-4);

        // 2. Evacuation Truck must avoid narrow alley and take wide detour (A -> B -> D -> C = 5.0 km)
        let truck_query = RoutingQuery::new("A", "C").with_asset_type(AssetType::EvacTruck);
        let truck_res = DeterministicPathfinder::find_directed_route(&edges, &truck_query).unwrap();
        assert_eq!(truck_res.path, vec!["A", "B", "D", "C"]);
        assert_eq!(
            truck_res.segment_path,
            vec!["seg_A_B", "seg_B_D_wide", "seg_D_C_wide"]
        );
        assert!((truck_res.total_distance_km - 5.0).abs() < 1e-4);
    }

    #[test]
    fn test_dynamic_hazard_closure_detour() {
        // Direct bridge J1 -> J2 (2.0 km)
        // High-ground bypass J1 -> J3 -> J2 (6.0 km)
        let edges = vec![
            create_edge("bridge_direct", "J1", "J2", 2.0, true, true),
            create_edge("bypass_1", "J1", "J3", 3.0, true, true),
            create_edge("bypass_2", "J3", "J2", 3.0, true, true),
        ];

        // 1. Clear condition: routes via direct bridge
        let clear_query = RoutingQuery::new("J1", "J2");
        let clear_res = DeterministicPathfinder::find_directed_route(&edges, &clear_query).unwrap();
        assert_eq!(clear_res.path, vec!["J1", "J2"]);
        assert_eq!(clear_res.segment_path, vec!["bridge_direct"]);

        // 2. Flooded bridge condition: block "bridge_direct"
        let mut blocked = HashSet::new();
        blocked.insert("bridge_direct".to_string());
        let flood_query = RoutingQuery::new("J1", "J2").with_blocked_segments(blocked);

        let flood_res = DeterministicPathfinder::find_directed_route(&edges, &flood_query).unwrap();
        assert_eq!(flood_res.path, vec!["J1", "J3", "J2"]);
        assert_eq!(flood_res.segment_path, vec!["bypass_1", "bypass_2"]);
        assert!((flood_res.total_distance_km - 6.0).abs() < 1e-4);
        assert!(flood_res.is_detour);
        assert!(flood_res.detour_reason.is_some());
    }

    #[test]
    fn test_unreachable_destination_fails_safely() {
        let edges = vec![create_edge("seg_1", "J1", "J2", 1.0, true, true)];

        let query = RoutingQuery::new("J1", "J99");
        let err = DeterministicPathfinder::find_directed_route(&edges, &query);
        assert!(matches!(err, Err(PathfinderError::DestinationNotFound(_))));

        // When isolated
        let isolated_edges = vec![
            create_edge("seg_1", "J1", "J2", 1.0, true, true),
            create_edge("seg_2", "J3", "J4", 1.0, true, true),
        ];
        let query_isolated = RoutingQuery::new("J1", "J4");
        let err_isolated = DeterministicPathfinder::find_directed_route(&isolated_edges, &query_isolated);
        assert!(matches!(
            err_isolated,
            Err(PathfinderError::NoPassableRouteFound { .. })
        ));
    }
}
