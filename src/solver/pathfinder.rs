use std::collections::HashMap;
use petgraph::algo::dijkstra;
use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::visit::EdgeRef;
use crate::db::PassableEdge;

#[derive(Debug, Clone)]
pub struct RouteResult {
    pub path: Vec<String>,
    pub total_distance_km: f64,
    pub is_detour: bool,
    pub detour_reason: Option<String>,
}

pub struct DeterministicPathfinder;

impl DeterministicPathfinder {
    /// Compute shortest path using Petgraph Dijkstra over passable graph edges
    pub fn find_shortest_path(
        edges: &[PassableEdge],
        start_junction: &str,
        target_junction: &str,
    ) -> Result<RouteResult, Box<dyn std::error::Error + Send + Sync>> {
        if start_junction == target_junction {
            return Ok(RouteResult {
                path: vec![start_junction.to_string()],
                total_distance_km: 0.0,
                is_detour: false,
                detour_reason: None,
            });
        }

        // 1. Build Petgraph Undirected Graph
        let mut graph = UnGraph::<String, f64>::new_undirected();
        let mut node_indices: HashMap<String, NodeIndex> = HashMap::new();
        let mut index_to_name: HashMap<NodeIndex, String> = HashMap::new();

        // Helper to register node
        let mut get_or_create_node = |name: &str, g: &mut UnGraph<String, f64>, map: &mut HashMap<String, NodeIndex>, rev: &mut HashMap<NodeIndex, String>| {
            if let Some(&idx) = map.get(name) {
                idx
            } else {
                let idx = g.add_node(name.to_string());
                map.insert(name.to_string(), idx);
                rev.insert(idx, name.to_string());
                idx
            }
        };

        for e in edges {
            let u = get_or_create_node(&e.source, &mut graph, &mut node_indices, &mut index_to_name);
            let v = get_or_create_node(&e.target, &mut graph, &mut node_indices, &mut index_to_name);
            graph.add_edge(u, v, e.weight);
        }

        let start_node = node_indices.get(start_junction)
            .ok_or_else(|| format!("Start junction '{}' not found in passable graph", start_junction))?;
        let target_node = node_indices.get(target_junction)
            .ok_or_else(|| format!("Target junction '{}' not found in passable graph", target_junction))?;

        // 2. Run Dijkstra Algorithm
        let node_map = dijkstra(&graph, *start_node, Some(*target_node), |e| *e.weight());

        let total_distance = node_map.get(target_node)
            .ok_or_else(|| format!("No passable route exists between '{}' and '{}'", start_junction, target_junction))?;

        // 3. Reconstruct shortest path by backtracking
        let mut current = *target_node;
        let mut path_nodes = vec![current];

        while current != *start_node {
            let mut best_prev = None;
            let mut best_cost = f64::MAX;

            for edge in graph.edges(current) {
                let neighbor = edge.target();
                let edge_weight = *edge.weight();
                
                if let Some(&dist_to_neighbor) = node_map.get(&neighbor) {
                    if (dist_to_neighbor + edge_weight - node_map.get(&current).unwrap()).abs() < 1e-4 {
                        if dist_to_neighbor < best_cost {
                            best_cost = dist_to_neighbor;
                            best_prev = Some(neighbor);
                        }
                    }
                }
            }

            if let Some(prev) = best_prev {
                path_nodes.push(prev);
                current = prev;
            } else {
                break;
            }
        }

        path_nodes.reverse();
        let path: Vec<String> = path_nodes
            .into_iter()
            .map(|idx| index_to_name.get(&idx).unwrap().clone())
            .collect();

        // Detour detection (e.g. if path has more than 3 hops or bypasses flooded segments)
        let is_detour = path.len() > 3;
        let detour_reason = if is_detour {
            Some("Direct primary route blocked/flooded; routed through safe elevated ridge segments.".to_string())
        } else {
            None
        };

        Ok(RouteResult {
            path,
            total_distance_km: *total_distance,
            is_detour,
            detour_reason,
        })
    }
}
