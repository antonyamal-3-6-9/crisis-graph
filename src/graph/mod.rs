//! Shared graph snapshot (M4, pipeline doc §4.6).
//!
//! The baseline road graph is loaded from Neo4j **once per process** into an
//! immutable [`RoutingGraph`]. Hazards are a small versioned overlay, swapped
//! in atomically (`arc-swap`) when `HAZARDS_CHANGED` arrives on the events
//! stream, with a periodic refresh as fallback. Routing masks the segments
//! whose hazard is in force at the moment of the query, so an expiring hazard
//! needs no new version.
//!
//! Every route carries the `overlay_version` it was computed on. The snapshot
//! never certifies a route: verification stays live against Neo4j
//! (`StateManager::verify_path_segments`), which catches any hazard that
//! arrived after the route was computed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use thiserror::Error;
use tracing::{info, warn};

use crate::db::{OpsRepoError, OpsRepository, OverlayRead, RedisClient, StateManager};
use crate::models::in_force;
use crate::runtime::Shutdown;
use crate::solver::{PathfinderError, RouteResult, RoutingGraph, RoutingQuery};

pub const HAZARDS_CHANGED: &str = "HAZARDS_CHANGED";

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("baseline graph for dataset '{0}' is empty")]
    EmptyBaseline(String),
    #[error("baseline load failed: {0}")]
    Baseline(String),
    #[error(transparent)]
    Repo(#[from] OpsRepoError),
}

/// Active hazards at one `overlay_version`.
#[derive(Debug, Clone, PartialEq)]
pub struct HazardOverlay {
    pub version: u64,
    pub loaded_at: DateTime<Utc>,
    hazards: HashMap<String, (DateTime<Utc>, Option<DateTime<Utc>>)>,
}

impl HazardOverlay {
    pub fn from_read(read: OverlayRead) -> Self {
        Self {
            version: read.overlay_version,
            loaded_at: Utc::now(),
            hazards: read
                .hazards
                .into_iter()
                .map(|(segment, from, until)| (segment, (from, until)))
                .collect(),
        }
    }

    /// Segments whose hazard is in force at `now`.
    pub fn blocked_at(&self, now: DateTime<Utc>) -> HashSet<String> {
        self.hazards
            .iter()
            .filter(|(_, (from, until))| in_force(*from, *until, now))
            .map(|(segment, _)| segment.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.hazards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hazards.is_empty()
    }
}

/// A route and the overlay version it was computed on.
#[derive(Debug, Clone)]
pub struct SnapshotRoute {
    pub route: RouteResult,
    pub overlay_version: u64,
}

pub struct GraphSnapshot {
    dataset: String,
    baseline: Arc<RoutingGraph>,
    overlay: ArcSwap<HazardOverlay>,
    repo: OpsRepository,
}

impl GraphSnapshot {
    /// Load the baseline (once) and the current overlay. Fails if the
    /// dataset has no baseline edges: routing on nothing must not look like
    /// "no route".
    pub async fn load(repo: OpsRepository, state: &StateManager, dataset: &str) -> Result<Self, SnapshotError> {
        let started = Instant::now();
        let edges = state
            .get_directed_baseline_edges(dataset)
            .await
            .map_err(|e| SnapshotError::Baseline(e.to_string()))?;
        if edges.is_empty() {
            return Err(SnapshotError::EmptyBaseline(dataset.to_string()));
        }
        let baseline = Arc::new(RoutingGraph::from_edges(&edges));
        let overlay = HazardOverlay::from_read(repo.read_overlay(dataset).await?);
        info!(
            dataset,
            nodes = baseline.node_count(),
            edges = baseline.edge_count(),
            overlay_version = overlay.version,
            hazards = overlay.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "Graph snapshot loaded"
        );
        let snapshot = Self { dataset: dataset.to_string(), baseline, overlay: ArcSwap::from_pointee(overlay), repo };
        snapshot.publish_metrics();
        Ok(snapshot)
    }

    pub fn dataset(&self) -> &str {
        &self.dataset
    }

    /// The immutable baseline (the same `Arc` for the life of the snapshot).
    pub fn baseline(&self) -> Arc<RoutingGraph> {
        self.baseline.clone()
    }

    pub fn overlay(&self) -> Arc<HazardOverlay> {
        self.overlay.load_full()
    }

    /// Route on the baseline with every hazard in force at `now` masked, in
    /// addition to any segments the query already blocks.
    pub fn route(&self, query: &RoutingQuery, now: DateTime<Utc>) -> Result<SnapshotRoute, PathfinderError> {
        let overlay = self.overlay.load();
        let mut query = query.clone();
        query.blocked_segments.extend(overlay.blocked_at(now));
        let route = self.baseline.route(&query)?;
        Ok(SnapshotRoute { route, overlay_version: overlay.version })
    }

    /// Travel cost from `query.origin` to every reachable junction, with the
    /// hazards in force at `now` masked, and the overlay version used.
    pub fn costs_from(
        &self,
        query: &RoutingQuery,
        now: DateTime<Utc>,
    ) -> Result<(HashMap<String, f64>, u64), PathfinderError> {
        let overlay = self.overlay.load();
        let mut query = query.clone();
        query.blocked_segments.extend(overlay.blocked_at(now));
        Ok((self.baseline.costs_from(&query)?, overlay.version))
    }

    /// Reload the overlay if its version changed. Returns whether it was
    /// swapped. Never goes back to an older version.
    pub async fn refresh(&self) -> Result<bool, OpsRepoError> {
        let current = self.overlay.load().version;
        if self.repo.overlay_version(&self.dataset).await? == current {
            self.publish_metrics();
            return Ok(false);
        }
        let fresh = HazardOverlay::from_read(self.repo.read_overlay(&self.dataset).await?);
        if fresh.version <= current {
            return Ok(false);
        }
        info!(dataset = %self.dataset, from = current, to = fresh.version, hazards = fresh.len(), "Hazard overlay swapped");
        self.overlay.store(Arc::new(fresh));
        self.publish_metrics();
        Ok(true)
    }

    fn publish_metrics(&self) {
        let overlay = self.overlay.load();
        metrics::gauge!("crisisgraph_graph_overlay_version").set(overlay.version as f64);
        metrics::gauge!("crisisgraph_graph_overlay_hazards").set(overlay.len() as f64);
        let age = (Utc::now() - overlay.loaded_at).num_milliseconds().max(0) as f64 / 1000.0;
        metrics::gauge!("crisisgraph_graph_overlay_age_seconds").set(age);
    }

    /// Keep the overlay current until shutdown: refresh on every
    /// `HAZARDS_CHANGED` on `events_stream` (read without a consumer group,
    /// so every process sees every event) and at least every `every`.
    pub async fn run_refresher(self: Arc<Self>, redis: RedisClient, events_stream: String, every: Duration, shutdown: Shutdown) {
        // Take the stream position first, then refresh: a change committed in
        // between is caught by the refresh or by its event.
        let mut last_id = redis.last_stream_id(&events_stream).await.unwrap_or_else(|e| {
            warn!(error = %e, "Could not read events stream position; starting from new events");
            "$".to_string()
        });
        let mut last_refresh = Instant::now();
        if let Err(e) = self.refresh().await {
            warn!(error = %e, "Overlay refresh failed");
        }
        info!(dataset = %self.dataset, stream = %events_stream, "Overlay refresher started");

        while !shutdown.is_triggered() {
            let read = tokio::select! {
                read = redis.read_after(&events_stream, &last_id, 100, 1000) => read,
                _ = shutdown.wait() => break,
            };
            let mut changed = false;
            match read {
                Ok(messages) => {
                    for message in &messages {
                        changed |= message.field("type") == Some(HAZARDS_CHANGED);
                    }
                    if let Some(last) = messages.last() {
                        last_id = last.id.clone();
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Events stream read failed; backing off");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            if changed || last_refresh.elapsed() >= every {
                match self.refresh().await {
                    Ok(_) => last_refresh = Instant::now(),
                    Err(e) => warn!(error = %e, "Overlay refresh failed"),
                }
            }
        }
        info!(dataset = %self.dataset, "Overlay refresher stopped");
    }
}
