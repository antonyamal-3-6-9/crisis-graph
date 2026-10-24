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

/// Constant penalty in seconds added to blocked/hazardous edges during relaxed cut-edge search.
/// Must be strictly greater than any conceivable dry path across the pilot bounding region.
pub const HAZARD_PENALTY_S: f64 = 1_000_000.0;

/// Whether `asset_type` may legally and physically drive `edge` on dry road.
///
/// Rescue boats are moved over dry road on a trailer towed by an emergency-cleared
/// vehicle; they only operate as watercraft on segments under an active hazard overlay.
pub fn road_access_allowed(asset_type: Option<&AssetType>, edge: &RoutingEdge) -> bool {
    match asset_type {
        Some(AssetType::EvacTruck) => {
            edge.access.hgv && edge.baseline_status != BaselineStatus::Unsuitable
        }
        Some(AssetType::Ambulance) | Some(AssetType::RescueBoat) => {
            edge.access.emergency && edge.baseline_status != BaselineStatus::Unsuitable
        }
        Some(AssetType::Helicopter) => true,
        None => edge.access.motorcar || edge.access.emergency,
    }
}

/// Transportation mode for a discrete leg of an evacuation route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportMode {
    /// Wheeled road vehicle (Ambulance, Evacuation Truck)
    RoadVehicle,
    /// Watercraft bridging flooded road or river corridor (Rescue Boat, Inflatable)
    Watercraft,
    /// Foot evacuation or stretcher carry on an isolated dry pocket or shallow edge
    FootWading,
    /// Direct aerial evacuation bypassing surface bottlenecks entirely
    Aerial,
}

/// A discrete leg within a multi-modal evacuation plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteLeg {
    /// Zero-based leg index in transit sequence
    pub leg_index: usize,
    /// Transportation mode used for this leg
    pub mode: TransportMode,
    /// Starting junction ID for this leg
    pub origin_junction: String,
    /// Ending junction ID for this leg
    pub destination_junction: String,
    /// Sequence of junction IDs traversed in this leg
    pub waypoints: Vec<String>,
    /// Sequence of OSM directed segment IDs traversed in this leg
    pub segment_ids: Vec<String>,
    /// Total distance in kilometers for this leg
    pub distance_km: f64,
    /// Estimated travel time in seconds for this leg
    pub travel_time_s: f64,
    /// Whether this leg traverses active hazard/flooded segments
    pub is_hazard_stretch: bool,
    /// Specific hazard condition if applicable (e.g. "FLOODED", "BLOCKED")
    pub hazard_type: Option<String>,
}

/// High-level topological classification of the route bottleneck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MitigationTopology {
    /// 100% open dry road network (no mitigation required)
    DirectPassable,
    /// Terminal hazard: Road transit to dry staging hub, watercraft across final flooded corridor to victim
    TerminalHazard,
    /// Island hazard: Road transit to staging, watercraft crossing, followed by dry island/pocket to victim
    IslandHazard,
    /// Fragmented cut: Multiple alternating hazard barriers requiring aerial evacuation
    FragmentedCut,
}

/// Synthesized actionable multi-modal disaster mitigation plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiModalPlan {
    pub origin: String,
    pub destination: String,
    pub requires_mitigation: bool,
    pub topology: MitigationTopology,
    pub staging_junction: Option<String>,
    pub landfall_junction: Option<String>,
    pub legs: Vec<RouteLeg>,
    pub total_distance_km: f64,
    pub total_travel_time_s: f64,
    pub primary_asset: AssetType,
    pub mitigation_asset: Option<AssetType>,
    pub recommendation_summary: String,
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
            if !road_access_allowed(query.asset_type.as_ref(), edge) {
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

    /// Actionable Multi-Modal Routing & Cut-Edge Mitigation Solver.
    ///
    /// 1. Primary Pass: Evaluates strict passable directed routing via `find_directed_route`.
    ///    If an uncompromised path exists, returns a single-leg `DirectPassable` plan.
    ///
    /// 2. Secondary Pass (Penalty Dijkstra): If all direct paths are blocked by hazards,
    ///    runs relaxed Dijkstra where flooded/blocked segments incur `HAZARD_PENALTY_S`.
    ///    Mathematically forces Petgraph to maximize dry road travel and isolate the exact
    ///    transition boundary (staging junction) where water begins.
    ///
    /// 3. Leg Segmentation: Decomposes the traversed segments into homogeneous operational legs
    ///    (Road Vehicle -> Staging Junction -> Rescue Boat -> Destination).
    pub fn find_multimodal_route(
        edges: &[RoutingEdge],
        query: &RoutingQuery,
    ) -> Result<MultiModalPlan, PathfinderError> {
        let primary_asset = query.asset_type.clone().unwrap_or(AssetType::Ambulance);

        if query.origin == query.destination {
            let single_leg = RouteLeg {
                leg_index: 0,
                mode: TransportMode::RoadVehicle,
                origin_junction: query.origin.clone(),
                destination_junction: query.destination.clone(),
                waypoints: vec![query.origin.clone()],
                segment_ids: Vec::new(),
                distance_km: 0.0,
                travel_time_s: 0.0,
                is_hazard_stretch: false,
                hazard_type: None,
            };

            return Ok(MultiModalPlan {
                origin: query.origin.clone(),
                destination: query.destination.clone(),
                requires_mitigation: false,
                topology: MitigationTopology::DirectPassable,
                staging_junction: None,
                landfall_junction: None,
                legs: vec![single_leg],
                total_distance_km: 0.0,
                total_travel_time_s: 0.0,
                primary_asset,
                mitigation_asset: None,
                recommendation_summary: "Origin and destination are identical; unit on site.".into(),
            });
        }

        // Pass 1: Try strict passable directed route
        match Self::find_directed_route(edges, query) {
            Ok(direct_route) => {
                let single_leg = RouteLeg {
                    leg_index: 0,
                    mode: TransportMode::RoadVehicle,
                    origin_junction: query.origin.clone(),
                    destination_junction: query.destination.clone(),
                    waypoints: direct_route.path,
                    segment_ids: direct_route.segment_path,
                    distance_km: direct_route.total_distance_km,
                    travel_time_s: direct_route.total_travel_time_s,
                    is_hazard_stretch: false,
                    hazard_type: None,
                };

                return Ok(MultiModalPlan {
                    origin: query.origin.clone(),
                    destination: query.destination.clone(),
                    requires_mitigation: false,
                    topology: MitigationTopology::DirectPassable,
                    staging_junction: None,
                    landfall_junction: None,
                    legs: vec![single_leg],
                    total_distance_km: direct_route.total_distance_km,
                    total_travel_time_s: direct_route.total_travel_time_s,
                    primary_asset,
                    mitigation_asset: None,
                    recommendation_summary: format!(
                        "Direct passable road corridor verified ({:.2} km, {:.0} s). No hazard mitigation required.",
                        direct_route.total_distance_km, direct_route.total_travel_time_s
                    ),
                });
            }
            Err(PathfinderError::NoPassableRouteFound { .. }) => {
                // Pass 1 failed because all routes are blocked or restricted.
                // Fall through to Pass 2: Big-M Penalty Dijkstra.
            }
            Err(e) => return Err(e), // Origin or Destination not found in graph
        }

        // Pass 2: Relaxed Dijkstra with Big-M Hazard Penalty
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

        // Cost function: Dry edges cost normal travel_time_s; blocked or vehicle-restricted edges cost time + HAZARD_PENALTY_S
        let penalty_cost_fn = |edge_ref: petgraph::graph::EdgeReference<RoutingEdge>| -> f64 {
            let edge = edge_ref.weight();
            let is_blocked = query.blocked_segments.contains(&edge.segment_id);
            let is_accessible = road_access_allowed(query.asset_type.as_ref(), edge);

            let base_cost = match query.cost_metric {
                CostMetric::FastestTime => edge.travel_time_s.max(0.01),
                CostMetric::ShortestDistance => (edge.length_m / 1_000.0).max(0.0001),
            };

            if is_blocked || !is_accessible {
                base_cost + HAZARD_PENALTY_S
            } else {
                base_cost
            }
        };

        let distances = dijkstra(&graph, start_idx, Some(target_idx), penalty_cost_fn);

        let _target_distance = distances
            .get(&target_idx)
            .copied()
            .filter(|d| !d.is_infinite())
            .ok_or_else(|| PathfinderError::NoPassableRouteFound {
                origin: query.origin.clone(),
                destination: query.destination.clone(),
            })?;

        // Backtrack path
        let mut current = target_idx;
        let mut path_nodes = vec![current];
        let mut traversed_edges: Vec<RoutingEdge> = Vec::new();
        let max_hops = graph.node_count();

        while current != start_idx {
            if path_nodes.len() > max_hops {
                return Err(PathfinderError::Internal(
                    "Cycle detected during relaxed path backtracking".into(),
                ));
            }

            let mut best_prev = None;
            let mut best_edge = None;
            let mut best_diff = f64::MAX;
            let current_dist = distances[&current];

            for in_edge in graph.edges_directed(current, Direction::Incoming) {
                let prev = in_edge.source();
                let cost = penalty_cost_fn(in_edge);
                if cost.is_infinite() {
                    continue;
                }

                if let Some(&prev_dist) = distances.get(&prev) {
                    let diff = (prev_dist + cost - current_dist).abs();
                    // Tolerance accounts for float accumulation with 1e6 penalty weights
                    if diff < 1e-2 && diff < best_diff {
                        best_diff = diff;
                        best_prev = Some(prev);
                        best_edge = Some(in_edge.weight().clone());
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
                        "Incomplete path reconstruction during relaxed backtracking".into(),
                    ));
                }
            }
        }

        path_nodes.reverse();
        traversed_edges.reverse();

        // Pass 3: Decompose traversed edges into operational legs.
        //
        // The relaxed search penalises two different conditions: active hazard overlays
        // (which a boat can bridge) and vehicle access restrictions on dry road (which it
        // cannot). Only hazard overlays become water legs; a restricted dry edge on the
        // road approach means no safe plan exists and must fail closed.
        let is_edge_hazard = |edge: &RoutingEdge| -> bool {
            query.blocked_segments.contains(&edge.segment_id)
        };

        struct RawLeg {
            is_hazard: bool,
            edges: Vec<RoutingEdge>,
        }

        let mut raw_legs: Vec<RawLeg> = Vec::new();

        for edge in traversed_edges {
            let is_hz = is_edge_hazard(&edge);
            if let Some(last_leg) = raw_legs.last_mut() {
                if last_leg.is_hazard == is_hz {
                    last_leg.edges.push(edge);
                    continue;
                }
            }
            raw_legs.push(RawLeg {
                is_hazard: is_hz,
                edges: vec![edge],
            });
        }

        let hazard_leg_count = raw_legs.iter().filter(|l| l.is_hazard).count();
        let no_safe_plan = || PathfinderError::NoPassableRouteFound {
            origin: query.origin.clone(),
            destination: query.destination.clone(),
        };

        // Pass 1 failed, so a plan without any hazard stretch can only exist because the
        // relaxed search crossed access-restricted road. A boat does not fix that.
        if hazard_leg_count == 0 {
            return Err(no_safe_plan());
        }

        let mut legs: Vec<RouteLeg> = Vec::new();
        let mut staging_junction: Option<String> = None;
        let mut landfall_junction: Option<String> = None;
        let mut seen_hazard = false;

        for (idx, raw_leg) in raw_legs.iter().enumerate() {
            let mode = if raw_leg.is_hazard {
                TransportMode::Watercraft
            } else if seen_hazard {
                // Dry leg following a water stretch represents an isolated island pocket
                TransportMode::FootWading
            } else {
                TransportMode::RoadVehicle
            };

            if mode == TransportMode::RoadVehicle
                && raw_leg
                    .edges
                    .iter()
                    .any(|e| !road_access_allowed(Some(&primary_asset), e))
            {
                return Err(no_safe_plan());
            }

            let first_edge = raw_leg.edges.first().expect("non-empty leg");
            let last_edge = raw_leg.edges.last().expect("non-empty leg");

            let leg_origin = first_edge.source.clone();
            let leg_dest = last_edge.target.clone();

            let mut waypoints = vec![first_edge.source.clone()];
            for e in &raw_leg.edges {
                waypoints.push(e.target.clone());
            }

            let segment_ids: Vec<String> = raw_leg.edges.iter().map(|e| e.segment_id.clone()).collect();
            let distance_km: f64 = raw_leg.edges.iter().map(|e| e.length_m / 1_000.0).sum();
            let travel_time_s: f64 = raw_leg.edges.iter().map(|e| e.travel_time_s).sum();

            if raw_leg.is_hazard && staging_junction.is_none() {
                staging_junction = Some(leg_origin.clone());
            }

            if mode == TransportMode::FootWading && landfall_junction.is_none() {
                landfall_junction = Some(leg_origin.clone());
            }

            let hazard_type = if raw_leg.is_hazard {
                Some("FLOODED".to_string())
            } else {
                None
            };

            seen_hazard |= raw_leg.is_hazard;

            legs.push(RouteLeg {
                leg_index: idx,
                mode,
                origin_junction: leg_origin,
                destination_junction: leg_dest,
                waypoints,
                segment_ids,
                distance_km,
                travel_time_s,
                is_hazard_stretch: raw_leg.is_hazard,
                hazard_type,
            });
        }

        let total_distance_km: f64 = legs.iter().map(|l| l.distance_km).sum();
        let total_travel_time_s: f64 = legs.iter().map(|l| l.travel_time_s).sum();

        let topology = if hazard_leg_count > 1 {
            MitigationTopology::FragmentedCut
        } else if legs.last().is_some_and(|l| l.is_hazard_stretch) {
            MitigationTopology::TerminalHazard
        } else {
            MitigationTopology::IslandHazard
        };

        let is_boat_mission = primary_asset == AssetType::RescueBoat;
        // A boat mission already reserves the boat as its primary asset.
        let mitigation_asset = if is_boat_mission {
            None
        } else {
            Some(AssetType::RescueBoat)
        };

        let staging = staging_junction.clone().unwrap_or_else(|| query.origin.clone());
        let road_km: f64 = legs
            .iter()
            .take_while(|l| l.mode == TransportMode::RoadVehicle)
            .map(|l| l.distance_km)
            .sum();
        let water_km: f64 = legs
            .iter()
            .filter(|l| l.is_hazard_stretch)
            .map(|l| l.distance_km)
            .sum();
        let road_approach = if road_km > 0.0 {
            if is_boat_mission {
                format!(
                    "Transport Rescue Boat by road on a trailer for {:.2} km to Dry Staging Junction '{}'. ",
                    road_km, staging
                )
            } else {
                format!(
                    "Drive {:.2} km via {} to Dry Staging Junction '{}'. ",
                    road_km, primary_asset, staging
                )
            }
        } else {
            format!("Road network is flooded from origin; launch at '{}'. ", staging)
        };
        const WATER_CAVEAT: &str = "Water corridor is UNVERIFIED: confirm depth, current and obstructions with field teams before launch.";

        let recommendation_summary = match topology {
            MitigationTopology::TerminalHazard => format!(
                "Direct road blocked by active hazard overlay. {}Rescue Boat required for remaining {:.2} km across submerged corridor to reach victim. {}",
                road_approach, water_km, WATER_CAVEAT
            ),
            MitigationTopology::IslandHazard => {
                let landfall = landfall_junction.clone().unwrap_or_else(|| query.destination.clone());
                format!(
                    "Victim located on isolated dry pocket cut off by flood corridor. {}Launch Rescue Boat from Staging '{}' across {:.2} km of water to Landfall Dock '{}', complete final stretch on foot/stretcher. {} (If water current is severe, escalate for Aerial Helicopter Winch.)",
                    road_approach, staging, water_km, landfall, WATER_CAVEAT
                )
            }
            MitigationTopology::FragmentedCut => format!(
                "Complex disaster topology: {} disjoint flood barriers detected ({} legs, {:.2} km). Ground/water transfers infeasible; direct Aerial Evacuation (HELICOPTER) recommended.",
                hazard_leg_count,
                legs.len(),
                total_distance_km
            ),
            MitigationTopology::DirectPassable => unreachable!("plans with hazard legs are never DirectPassable"),
        };

        Ok(MultiModalPlan {
            origin: query.origin.clone(),
            destination: query.destination.clone(),
            requires_mitigation: true,
            topology,
            staging_junction,
            landfall_junction,
            legs,
            total_distance_km,
            total_travel_time_s,
            primary_asset,
            mitigation_asset,
            recommendation_summary,
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

    #[test]
    fn test_multimodal_direct_passable_no_mitigation() {
        let edges = vec![
            create_edge("seg_1", "J1", "J2", 2.0, true, true),
            create_edge("seg_2", "J2", "J3", 3.0, true, true),
        ];

        let query = RoutingQuery::new("J1", "J3").with_asset_type(AssetType::Ambulance);
        let plan = DeterministicPathfinder::find_multimodal_route(&edges, &query).unwrap();

        assert!(!plan.requires_mitigation);
        assert_eq!(plan.topology, MitigationTopology::DirectPassable);
        assert_eq!(plan.legs.len(), 1);
        assert_eq!(plan.legs[0].mode, TransportMode::RoadVehicle);
        assert_eq!(plan.legs[0].origin_junction, "J1");
        assert_eq!(plan.legs[0].destination_junction, "J3");
        assert_eq!(plan.staging_junction, None);
        assert_eq!(plan.landfall_junction, None);
        assert!((plan.total_distance_km - 5.0).abs() < 1e-4);
    }

    #[test]
    fn test_multimodal_terminal_hazard_staging_detection() {
        // Path: Depot A -> Hub B -> Staging C -> Victim D
        // Segments:
        //   A -> B: 2.0 km (Dry)
        //   B -> C: 3.0 km (Dry)
        //   C -> D: 1.0 km (Flooded!)
        let edges = vec![
            create_edge("seg_A_B", "A", "B", 2.0, true, true),
            create_edge("seg_B_C", "B", "C", 3.0, true, true),
            create_edge("seg_C_D_flooded", "C", "D", 1.0, true, true),
        ];

        let mut blocked = HashSet::new();
        blocked.insert("seg_C_D_flooded".to_string());

        let query = RoutingQuery::new("A", "D")
            .with_asset_type(AssetType::Ambulance)
            .with_blocked_segments(blocked);

        // 1. Direct pathfinder MUST fail closed
        let direct_err = DeterministicPathfinder::find_directed_route(&edges, &query);
        assert!(matches!(direct_err, Err(PathfinderError::NoPassableRouteFound { .. })));

        // 2. Multi-modal mitigation solver MUST succeed and identify cut
        let plan = DeterministicPathfinder::find_multimodal_route(&edges, &query).unwrap();
        assert!(plan.requires_mitigation);
        assert_eq!(plan.topology, MitigationTopology::TerminalHazard);
        assert_eq!(plan.staging_junction, Some("C".to_string()));
        assert_eq!(plan.landfall_junction, None);
        assert_eq!(plan.legs.len(), 2);

        // Leg 0: Road vehicle (Ambulance) from A to C
        assert_eq!(plan.legs[0].mode, TransportMode::RoadVehicle);
        assert_eq!(plan.legs[0].origin_junction, "A");
        assert_eq!(plan.legs[0].destination_junction, "C");
        assert_eq!(plan.legs[0].waypoints, vec!["A", "B", "C"]);
        assert_eq!(plan.legs[0].segment_ids, vec!["seg_A_B", "seg_B_C"]);
        assert!((plan.legs[0].distance_km - 5.0).abs() < 1e-4);
        assert!(!plan.legs[0].is_hazard_stretch);

        // Leg 1: Rescue Boat from C to D
        assert_eq!(plan.legs[1].mode, TransportMode::Watercraft);
        assert_eq!(plan.legs[1].origin_junction, "C");
        assert_eq!(plan.legs[1].destination_junction, "D");
        assert_eq!(plan.legs[1].waypoints, vec!["C", "D"]);
        assert_eq!(plan.legs[1].segment_ids, vec!["seg_C_D_flooded"]);
        assert!((plan.legs[1].distance_km - 1.0).abs() < 1e-4);
        assert!(plan.legs[1].is_hazard_stretch);
        assert_eq!(plan.legs[1].hazard_type.as_deref(), Some("FLOODED"));

        // Assets
        assert_eq!(plan.primary_asset, AssetType::Ambulance);
        assert_eq!(plan.mitigation_asset, Some(AssetType::RescueBoat));
        assert!(plan.recommendation_summary.contains("Dry Staging Junction 'C'"));
        assert!(plan.recommendation_summary.contains("Rescue Boat"));
    }

    #[test]
    fn test_multimodal_island_hazard_scenario() {
        // Path: A -> B -> C -> D -> E
        //   A -> B: 2.0 km (Dry)
        //   B -> C: 1.0 km (Flooded River Cut)
        //   C -> D: 1.5 km (Dry Island)
        //   D -> E: 0.5 km (Dry Island to Victim)
        let edges = vec![
            create_edge("seg_A_B", "A", "B", 2.0, true, true),
            create_edge("seg_B_C_flooded", "B", "C", 1.0, true, true),
            create_edge("seg_C_D", "C", "D", 1.5, true, true),
            create_edge("seg_D_E", "D", "E", 0.5, true, true),
        ];

        let mut blocked = HashSet::new();
        blocked.insert("seg_B_C_flooded".to_string());

        let query = RoutingQuery::new("A", "E")
            .with_asset_type(AssetType::Ambulance)
            .with_blocked_segments(blocked);

        let plan = DeterministicPathfinder::find_multimodal_route(&edges, &query).unwrap();
        assert!(plan.requires_mitigation);
        assert_eq!(plan.topology, MitigationTopology::IslandHazard);
        assert_eq!(plan.staging_junction, Some("B".to_string()));
        assert_eq!(plan.landfall_junction, Some("C".to_string()));
        assert_eq!(plan.legs.len(), 3);

        // Leg 0: Road (A -> B)
        assert_eq!(plan.legs[0].mode, TransportMode::RoadVehicle);
        assert_eq!(plan.legs[0].origin_junction, "A");
        assert_eq!(plan.legs[0].destination_junction, "B");
        assert!((plan.legs[0].distance_km - 2.0).abs() < 1e-4);

        // Leg 1: Watercraft (B -> C)
        assert_eq!(plan.legs[1].mode, TransportMode::Watercraft);
        assert_eq!(plan.legs[1].origin_junction, "B");
        assert_eq!(plan.legs[1].destination_junction, "C");
        assert!((plan.legs[1].distance_km - 1.0).abs() < 1e-4);

        // Leg 2: FootWading / Stretcher on dry pocket (C -> E)
        assert_eq!(plan.legs[2].mode, TransportMode::FootWading);
        assert_eq!(plan.legs[2].origin_junction, "C");
        assert_eq!(plan.legs[2].destination_junction, "E");
        assert_eq!(plan.legs[2].waypoints, vec!["C", "D", "E"]);
        assert!((plan.legs[2].distance_km - 2.0).abs() < 1e-4);

        assert!(plan.recommendation_summary.contains("Staging 'B'"));
        assert!(plan.recommendation_summary.contains("Landfall Dock 'C'"));
    }

    #[test]
    fn test_boat_mission_on_dry_road_is_trailer_road_leg() {
        // No hazard overlay: the boat travels by road and the route is a normal road leg.
        let edges = vec![
            create_edge("seg_depot_1", "DEPOT", "J1", 0.5, true, true),
            create_edge("seg_1_victim", "J1", "VICTIM", 0.5, true, true),
        ];

        let query = RoutingQuery::new("DEPOT", "VICTIM").with_asset_type(AssetType::RescueBoat);

        let plan = DeterministicPathfinder::find_multimodal_route(&edges, &query).unwrap();
        assert!(!plan.requires_mitigation);
        assert_eq!(plan.topology, MitigationTopology::DirectPassable);
        assert_eq!(plan.legs.len(), 1);
        assert_eq!(plan.legs[0].mode, TransportMode::RoadVehicle);
        assert_eq!(plan.mitigation_asset, None);
    }

    #[test]
    fn test_boat_mission_launches_only_on_flooded_segments() {
        let edges = vec![
            create_edge("seg_depot_slip", "DEPOT", "SLIP", 1.0, true, true),
            create_edge("seg_slip_victim_flooded", "SLIP", "VICTIM", 0.8, true, true),
        ];
        let blocked = HashSet::from(["seg_slip_victim_flooded".to_string()]);

        let query = RoutingQuery::new("DEPOT", "VICTIM")
            .with_asset_type(AssetType::RescueBoat)
            .with_blocked_segments(blocked);

        let plan = DeterministicPathfinder::find_multimodal_route(&edges, &query).unwrap();
        assert!(plan.requires_mitigation);
        assert_eq!(plan.topology, MitigationTopology::TerminalHazard);
        assert_eq!(plan.staging_junction, Some("SLIP".to_string()));
        assert_eq!(plan.legs.len(), 2);
        assert_eq!(plan.legs[0].mode, TransportMode::RoadVehicle);
        assert_eq!(plan.legs[1].mode, TransportMode::Watercraft);
        assert_eq!(plan.mitigation_asset, None);
        assert!(plan.recommendation_summary.contains("trailer"));
        assert!(plan.recommendation_summary.contains("UNVERIFIED"));
    }

    #[test]
    fn test_access_restriction_without_hazard_fails_closed() {
        // Only way to C is a no-HGV alley and nothing is flooded: no boat can help.
        let edges = vec![
            create_edge("seg_A_B", "A", "B", 1.0, true, true),
            create_edge("seg_B_C_alley", "B", "C", 0.1, false, true),
        ];

        let query = RoutingQuery::new("A", "C").with_asset_type(AssetType::EvacTruck);
        let result = DeterministicPathfinder::find_multimodal_route(&edges, &query);
        assert!(matches!(result, Err(PathfinderError::NoPassableRouteFound { .. })));
    }

    #[test]
    fn test_access_restriction_on_road_approach_to_flood_fails_closed() {
        // Truck would have to use a no-HGV alley before reaching the flooded stretch.
        let edges = vec![
            create_edge("seg_A_B_alley", "A", "B", 0.2, false, true),
            create_edge("seg_B_C_flooded", "B", "C", 0.5, true, true),
        ];
        let blocked = HashSet::from(["seg_B_C_flooded".to_string()]);

        let query = RoutingQuery::new("A", "C")
            .with_asset_type(AssetType::EvacTruck)
            .with_blocked_segments(blocked);
        let result = DeterministicPathfinder::find_multimodal_route(&edges, &query);
        assert!(matches!(result, Err(PathfinderError::NoPassableRouteFound { .. })));
    }

    #[test]
    fn test_vehicle_restriction_after_landfall_is_walked() {
        // Island pocket reached by boat may use a footpath the truck could not drive.
        let edges = vec![
            create_edge("seg_A_B", "A", "B", 1.0, true, true),
            create_edge("seg_B_C_flooded", "B", "C", 0.5, true, true),
            create_edge("seg_C_D_footpath", "C", "D", 0.1, false, false),
        ];
        let blocked = HashSet::from(["seg_B_C_flooded".to_string()]);

        let query = RoutingQuery::new("A", "D")
            .with_asset_type(AssetType::EvacTruck)
            .with_blocked_segments(blocked);
        let plan = DeterministicPathfinder::find_multimodal_route(&edges, &query).unwrap();
        assert_eq!(plan.topology, MitigationTopology::IslandHazard);
        assert_eq!(plan.legs[2].mode, TransportMode::FootWading);
        assert_eq!(plan.landfall_junction, Some("C".to_string()));
    }
}
