//! Matcher (M5, allocation design §3): one leader per dataset assigns
//! vehicles to queued tasks in batch cycles.
//!
//! Each cycle:
//! 1. expire overdue holds (T3) and resume holds whose commit never happened
//!    (a crash between hold and commit keeps its reservation);
//! 2. take the first tasks from the priority queue and all available vehicles;
//! 3. build the cost table on the shared graph snapshot: one single-source
//!    search per vehicle start (depot and asset type), in parallel;
//! 4. solve tier by tier (strict tiers, Hungarian per tier);
//! 5. per pair, T2: hold → route → live verification → commit with drop-off
//!    beds. A failed verification releases the hold, refreshes the overlay
//!    and retries next cycle; repeated failures escalate;
//! 6. escalate tasks no vehicle can ever serve and tasks waiting too long.
//!
//! The matcher never certifies a route from the snapshot alone: every route
//! is checked segment by segment against live Neo4j state before commit.

pub mod assign;
pub mod queue;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde_json::json;
use tracing::{info, warn};

use crate::config::PipelineParams;
use crate::db::{
    AssignmentCommit, OpsRepoError, OpsRepository, RedisClient, StateManager, TraversalProfile,
    ESCALATION_NO_ELIGIBLE_VEHICLE, ESCALATION_OVERDUE,
};
use crate::graph::{GraphSnapshot, HAZARDS_CHANGED};
use crate::models::{AssetType, DropoffShelter, QueuedTask, Vehicle};
use crate::runtime::{Lease, Shutdown};
use crate::solver::RoutingQuery;

pub use assign::{effective_tier, greedy, queue_score, solve_tiers, totals, Assignment, CostTable, Tier};
pub use queue::AllocQueue;

type MatcherResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Events that change the queued tasks (the queue is re-synced).
const QUEUE_EVENTS: &[&str] = &[
    "INCIDENT_READY",
    "INCIDENT_UPDATED",
    "TASK_QUEUED",
    "RESERVATION_RELEASED",
    "RESERVATION_EXPIRED",
];
/// Events that also make a cycle worth running soon.
const CYCLE_EVENTS: &[&str] = &["VEHICLE_AVAILABLE", HAZARDS_CHANGED];

/// Lease (and fence) name of the matcher for `dataset`.
pub fn lease_name(dataset: &str) -> String {
    format!("matcher:{dataset}")
}

/// What one cycle did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CycleReport {
    pub expired_holds: u64,
    pub resumed_holds: usize,
    pub tasks: usize,
    pub vehicles: usize,
    /// `(task_id, vehicle_id)` committed this cycle.
    pub assigned: Vec<(String, String)>,
    /// Holds or commits lost to a concurrent change (dispatcher, follow-up).
    pub conflicts: usize,
    pub verify_failures: usize,
    pub escalated: usize,
    pub dropoff_unresolved: usize,
}

impl CycleReport {
    fn is_quiet(&self) -> bool {
        self.expired_holds == 0
            && self.resumed_holds == 0
            && self.assigned.is_empty()
            && self.conflicts == 0
            && self.verify_failures == 0
            && self.escalated == 0
    }
}

#[derive(Clone)]
pub struct Matcher {
    repo: OpsRepository,
    redis: RedisClient,
    state: StateManager,
    snapshot: Arc<GraphSnapshot>,
    queue: AllocQueue,
    params: PipelineParams,
    lease_name: String,
}

impl Matcher {
    pub fn new(
        repo: OpsRepository,
        redis: RedisClient,
        state: StateManager,
        snapshot: Arc<GraphSnapshot>,
        params: PipelineParams,
    ) -> Self {
        let queue = AllocQueue::new(redis.clone(), snapshot.dataset(), params.aging_stable_to_urgent_secs);
        let lease_name = lease_name(snapshot.dataset());
        Self { repo, redis, state, snapshot, queue, params, lease_name }
    }

    pub fn lease(&self) -> Lease {
        Lease::new(self.redis.clone(), self.lease_name.clone(), self.params.lease_ttl_ms)
    }

    pub fn lease_name(&self) -> &str {
        &self.lease_name
    }

    pub fn queue(&self) -> &AllocQueue {
        &self.queue
    }

    pub fn snapshot(&self) -> &Arc<GraphSnapshot> {
        &self.snapshot
    }

    /// Rebuild the priority queue from the dataset's `QUEUED` tasks.
    pub async fn sync_queue(&self) -> MatcherResult<usize> {
        let tasks = self.repo.queued_tasks(self.snapshot.dataset(), None).await?;
        self.queue.sync(&tasks).await?;
        Ok(tasks.len())
    }

    /// One matching cycle as leader with fencing token `token`. Returns
    /// `OpsRepoError::Fenced` (boxed) if a newer matcher has taken over.
    pub async fn run_cycle(&self, token: u64) -> MatcherResult<CycleReport> {
        let started = Instant::now();
        let dataset = self.snapshot.dataset().to_string();
        let mut report = CycleReport { expired_holds: self.repo.expire_due_holds(100).await?, ..Default::default() };
        let mut shelters = self.repo.dropoff_shelters(&dataset).await?;

        // A hold without a commit (the matcher stopped in between) is
        // finished, not matched again: the reservation is kept.
        for hold in self.repo.orphaned_holds(&dataset).await? {
            let task = self.repo.queued_tasks(&dataset, Some(std::slice::from_ref(&hold.task_id))).await?;
            let vehicle = self.repo.get_vehicle(&hold.vehicle_id).await?;
            match (task.first(), vehicle) {
                (Some(task), Some(vehicle)) => {
                    report.resumed_holds += 1;
                    self.finish(token, task, &vehicle, &hold.reservation_id, &mut shelters, &mut report).await?;
                }
                _ => {
                    self.repo.release_reservation(&hold.reservation_id, "hold could not be resumed").await?;
                }
            }
        }

        let now = Utc::now();
        let ids = self.queue.first(self.params.matcher_batch_cap).await?;
        let order: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
        let mut tasks = self.repo.queued_tasks(&dataset, Some(&ids)).await?;
        tasks.sort_by_key(|t| order.get(t.task_id.as_str()).copied().unwrap_or(usize::MAX));
        let vehicles = self.repo.list_available_vehicles_in(&dataset).await?;
        report.tasks = tasks.len();
        report.vehicles = vehicles.len();

        // A task no vehicle in the fleet could ever serve goes to a person now.
        let fleet = self.repo.fleet_max_capacity(&dataset).await?;
        for task in &tasks {
            let capacity = fleet.get(&task.required_asset.to_string()).copied();
            if capacity.is_none_or(|c| c < task.persons()) && !escalated(task, ESCALATION_NO_ELIGIBLE_VEHICLE) {
                let details = json!({ "required_asset": task.required_asset.to_string(), "persons": task.persons(),
                                      "largest_capacity": capacity });
                if self.repo.escalate_task(&task.task_id, ESCALATION_NO_ELIGIBLE_VEHICLE, details).await? {
                    report.escalated += 1;
                }
            }
        }

        let tiers: Vec<Tier> = tasks
            .iter()
            .map(|t| effective_tier(t.priority_rank, t.queued_at, now, self.params.aging_stable_to_urgent_secs))
            .collect();
        let costs = build_cost_table(&self.snapshot, &tasks, &vehicles, now).await?;
        let mut assigned_tasks = Vec::new();
        for pair in solve_tiers(&tiers, &costs) {
            let (task, vehicle) = (&tasks[pair.task], &vehicles[pair.vehicle]);
            let held_until = Utc::now() + chrono::Duration::seconds(self.params.hold_expiry_secs as i64);
            match self.repo.hold_vehicle(&task.task_id, &task.incident_id, &vehicle.vehicle_id, vehicle.version, held_until).await {
                Ok(hold) => {
                    if self.finish(token, task, vehicle, &hold.reservation.reservation_id, &mut shelters, &mut report).await? {
                        assigned_tasks.push(pair.task);
                    }
                }
                // Taken meanwhile (dispatcher action): the task waits for the next cycle.
                Err(OpsRepoError::Conflict(_) | OpsRepoError::NotAvailable(_)) => report.conflicts += 1,
                Err(error) => return Err(error.into()),
            }
        }

        for (index, task) in tasks.iter().enumerate() {
            if assigned_tasks.contains(&index) || escalated(task, ESCALATION_OVERDUE) {
                continue;
            }
            let waited = (now - task.queued_at).num_seconds().max(0) as u64;
            let threshold = self.escalation_threshold(task.priority_rank);
            if waited >= threshold {
                let details = json!({ "tier": tiers[index].as_str(), "waited_s": waited, "threshold_s": threshold });
                if self.repo.escalate_task(&task.task_id, ESCALATION_OVERDUE, details).await? {
                    report.escalated += 1;
                }
            }
        }

        self.publish_metrics(&tasks, &tiers, now, &report, started.elapsed());
        Ok(report)
    }

    /// The rest of T2 for a held vehicle: route on the snapshot, verify live,
    /// pick the drop-off and commit. Returns whether the task was assigned.
    /// Any failure before the commit releases the hold.
    async fn finish(
        &self,
        token: u64,
        task: &QueuedTask,
        vehicle: &Vehicle,
        reservation_id: &str,
        shelters: &mut [DropoffShelter],
        report: &mut CycleReport,
    ) -> MatcherResult<bool> {
        let asset = vehicle.asset_type.clone();
        let Some(origin) = vehicle.junction_id.clone() else {
            self.repo.release_reservation(reservation_id, "vehicle has no position").await?;
            return Ok(false);
        };
        let query = RoutingQuery::new(origin, task.junction_id.clone()).with_asset_type(asset.clone());
        let routed = match self.snapshot.route(&query, Utc::now()) {
            Ok(routed) => routed,
            Err(error) => {
                self.repo.release_reservation(reservation_id, &format!("no route on snapshot: {error}")).await?;
                return Ok(false);
            }
        };

        let unsafe_segments = match self
            .state
            .verify_path_segments(&routed.route.segment_path, &TraversalProfile::Vehicle(asset.clone()))
            .await
        {
            Ok(segments) => segments,
            Err(error) => {
                // Fail closed: an unverified route is never committed.
                self.repo.release_reservation(reservation_id, "route verification unavailable").await?;
                return Err(error);
            }
        };
        if !unsafe_segments.is_empty() {
            self.repo.release_reservation(reservation_id, "route failed live verification").await?;
            report.verify_failures += 1;
            metrics::counter!("crisisgraph_matcher_verify_failures_total").increment(1);
            warn!(task_id = %task.task_id, vehicle_id = %vehicle.vehicle_id, segments = ?unsafe_segments,
                  overlay_version = routed.overlay_version, "Route failed live verification; retrying next cycle");
            if self
                .repo
                .record_verify_failure(&task.task_id, self.params.matcher_max_verify_failures, &unsafe_segments)
                .await?
            {
                report.escalated += 1;
            }
            // The snapshot missed a change: catch up before the next cycle.
            if let Err(error) = self.snapshot.refresh().await {
                warn!(%error, "Overlay refresh after failed verification failed");
            }
            return Ok(false);
        }

        let dropoff = self.choose_dropoff(task, &asset, shelters).await?;
        let commit = AssignmentCommit {
            reservation_id: reservation_id.to_string(),
            task_id: task.task_id.clone(),
            vehicle_id: vehicle.vehicle_id.clone(),
            task_version: task.version,
            fence_name: self.lease_name.clone(),
            fence_token: token,
            route_json: serde_json::to_string(&routed.route)?,
            overlay_version: routed.overlay_version,
            eta_s: routed.route.total_travel_time_s,
            dropoff,
        };
        match self.repo.commit_assignment(&commit).await {
            Ok(outcome) => {
                if outcome.dropoff_shelter.is_none() {
                    report.dropoff_unresolved += 1;
                }
                report.assigned.push((task.task_id.clone(), vehicle.vehicle_id.clone()));
                metrics::counter!("crisisgraph_matcher_assignments_total").increment(1);
                if let Err(error) = self.queue.remove(&task.task_id).await {
                    warn!(%error, task_id = %task.task_id, "Could not remove assigned task from the queue");
                }
                info!(task_id = %task.task_id, vehicle_id = %vehicle.vehicle_id, eta_s = commit.eta_s,
                      overlay_version = commit.overlay_version, dropoff = ?outcome.dropoff_shelter, "Task assigned");
                Ok(true)
            }
            // Leave the hold: the new leader resumes it.
            Err(OpsRepoError::Fenced(message)) => Err(OpsRepoError::Fenced(message).into()),
            Err(error) => {
                self.repo.release_reservation(reservation_id, "assignment commit failed").await?;
                match error {
                    OpsRepoError::NotAvailable(_) | OpsRepoError::Conflict(_) => {
                        report.conflicts += 1;
                        Ok(false)
                    }
                    other => Err(other.into()),
                }
            }
        }
    }

    /// Nearest reachable shelter (from the incident, for this vehicle) with
    /// enough free beds. The cycle's own reservations are counted, so two
    /// tasks in one cycle are not sent to the same last beds.
    async fn choose_dropoff(
        &self,
        task: &QueuedTask,
        asset: &AssetType,
        shelters: &mut [DropoffShelter],
    ) -> MatcherResult<Option<(String, u32)>> {
        let persons = task.persons();
        if !shelters.iter().any(|s| s.free_beds >= persons) {
            return Ok(None);
        }
        let snapshot = self.snapshot.clone();
        let query = RoutingQuery::new(task.junction_id.clone(), String::new()).with_asset_type(asset.clone());
        let now = Utc::now();
        let reach = tokio::task::spawn_blocking(move || snapshot.costs_from(&query, now).map(|(costs, _)| costs))
            .await?
            .unwrap_or_default();
        let best = shelters
            .iter_mut()
            .filter(|s| s.free_beds >= persons)
            .filter_map(|s| reach.get(&s.junction_id).map(|cost| (*cost, s)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        Ok(best.map(|(_, shelter)| {
            shelter.free_beds -= persons;
            (shelter.shelter_id.clone(), persons)
        }))
    }

    fn escalation_threshold(&self, priority_rank: u8) -> u64 {
        match priority_rank {
            2.. => self.params.escalate_critical_secs,
            1 => self.params.escalate_urgent_secs,
            _ => self.params.escalate_stable_secs,
        }
    }

    fn publish_metrics(&self, tasks: &[QueuedTask], tiers: &[Tier], now: DateTime<Utc>, report: &CycleReport, took: Duration) {
        metrics::histogram!("crisisgraph_matcher_cycle_seconds").record(took.as_secs_f64());
        metrics::counter!("crisisgraph_matcher_conflicts_total").increment(report.conflicts as u64);
        for tier in Tier::ALL {
            let waiting: Vec<&QueuedTask> = tasks.iter().zip(tiers).filter(|(_, t)| **t == tier).map(|(task, _)| task).collect();
            let oldest = waiting.iter().map(|t| (now - t.queued_at).num_seconds().max(0)).max().unwrap_or(0);
            metrics::gauge!("crisisgraph_matcher_waiting_tasks", "tier" => tier.as_str()).set(waiting.len() as f64);
            metrics::gauge!("crisisgraph_matcher_oldest_wait_seconds", "tier" => tier.as_str()).set(oldest as f64);
        }
    }

    /// Run until shutdown: hold the lease, and while leader run a cycle every
    /// `matcher_cycle_ms`, or `matcher_debounce_ms` after an event that
    /// changes tasks, vehicles or hazards. Releases the lease on shutdown.
    pub async fn run(self, shutdown: Shutdown) {
        let events_stream = self.params.events_stream.clone();
        let cycle_every = Duration::from_millis(self.params.matcher_cycle_ms);
        let debounce = Duration::from_millis(self.params.matcher_debounce_ms);
        let renew_every = Duration::from_millis(self.params.lease_renew_ms);
        let rebuild_every = Duration::from_secs(self.params.matcher_rebuild_secs);
        let mut lease = self.lease();
        let mut last_id = self.redis.last_stream_id(&events_stream).await.unwrap_or_else(|error| {
            warn!(%error, "Could not read events stream position; starting from new events");
            "$".to_string()
        });
        let mut last_renew = Instant::now();
        let mut next_cycle = Instant::now();
        let mut last_sync: Option<Instant> = None;
        let mut sync_needed = true;
        info!(dataset = %self.snapshot.dataset(), lease = %self.lease_name, "Matcher started");

        while !shutdown.is_triggered() {
            match lease.token() {
                None => match lease.try_acquire().await {
                    Ok(Some(token)) => {
                        info!(lease = lease.name(), token, "Matcher became leader");
                        last_renew = Instant::now();
                        sync_needed = true;
                        next_cycle = Instant::now();
                    }
                    Ok(None) => {}
                    Err(error) => warn!(%error, "Lease acquire failed"),
                },
                Some(_) if last_renew.elapsed() >= renew_every => match lease.renew().await {
                    Ok(true) => last_renew = Instant::now(),
                    Ok(false) => {
                        warn!(lease = lease.name(), "Matcher lost its lease");
                        metrics::counter!("crisisgraph_lease_lost_total", "lease" => self.lease_name.clone()).increment(1);
                    }
                    Err(error) => warn!(%error, "Lease renew failed"),
                },
                Some(_) => {}
            }
            metrics::gauge!("crisisgraph_matcher_is_leader").set(if lease.token().is_some() { 1.0 } else { 0.0 });

            // Wait for events until the next cycle is due (never longer than
            // the renewal interval, so the lease is kept).
            let wait = next_cycle.saturating_duration_since(Instant::now()).min(renew_every / 2);
            let block_ms = (wait.as_millis() as usize).max(1);
            let read = tokio::select! {
                read = self.redis.read_after(&events_stream, &last_id, 100, block_ms) => read,
                _ = shutdown.wait() => break,
            };
            match read {
                Ok(messages) => {
                    let mut hazards_changed = false;
                    for message in &messages {
                        let kind = message.field("type").unwrap_or_default();
                        if QUEUE_EVENTS.contains(&kind) {
                            sync_needed = true;
                        }
                        if QUEUE_EVENTS.contains(&kind) || CYCLE_EVENTS.contains(&kind) {
                            next_cycle = next_cycle.min(Instant::now() + debounce);
                        }
                        hazards_changed |= kind == HAZARDS_CHANGED;
                    }
                    if let Some(last) = messages.last() {
                        last_id = last.id.clone();
                    }
                    if hazards_changed {
                        if let Err(error) = self.snapshot.refresh().await {
                            warn!(%error, "Overlay refresh failed");
                        }
                    }
                }
                Err(error) => {
                    warn!(%error, "Events stream read failed; backing off");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }

            let Some(token) = lease.token() else {
                next_cycle = Instant::now() + cycle_every;
                continue;
            };
            if Instant::now() < next_cycle {
                continue;
            }
            if sync_needed || last_sync.is_none_or(|at| at.elapsed() >= rebuild_every) {
                match self.sync_queue().await {
                    Ok(_) => {
                        sync_needed = false;
                        last_sync = Some(Instant::now());
                    }
                    Err(error) => warn!(%error, "Queue sync failed"),
                }
            }
            match self.run_cycle(token).await {
                Ok(report) if !report.is_quiet() => info!(?report, "Matcher cycle"),
                Ok(_) => {}
                Err(error) => {
                    if matches!(error.downcast_ref::<OpsRepoError>(), Some(OpsRepoError::Fenced(_))) {
                        warn!(lease = lease.name(), token, "Fenced by a newer matcher; stepping down");
                        let _ = lease.release().await;
                    } else {
                        warn!(%error, "Matcher cycle failed; will retry");
                    }
                }
            }
            // An event can bring this forward, but never closer than one
            // debounce after this cycle ended.
            next_cycle = Instant::now() + cycle_every;
        }

        if lease.token().is_some() {
            let _ = lease.release().await;
            info!(lease = lease.name(), "Matcher released its lease");
        }
        metrics::gauge!("crisisgraph_matcher_is_leader").set(0.0);
    }
}

/// Travel time from every vehicle to every task on the snapshot (hazards in
/// force at `now` masked). Vehicles that start at the same junction with the
/// same asset type share one single-source search; searches run in
/// parallel. A pair is impossible when the asset type differs, the task has
/// more people than the vehicle carries, or there is no passable route.
pub async fn build_cost_table(
    snapshot: &Arc<GraphSnapshot>,
    tasks: &[QueuedTask],
    vehicles: &[Vehicle],
    now: DateTime<Utc>,
) -> MatcherResult<CostTable> {
    let mut starts: Vec<(String, AssetType)> = Vec::new();
    let mut start_of = Vec::with_capacity(vehicles.len());
    for vehicle in vehicles {
        let key = (vehicle.junction_id.clone().unwrap_or_default(), vehicle.asset_type.clone());
        let index = starts.iter().position(|s| *s == key).unwrap_or_else(|| {
            starts.push(key);
            starts.len() - 1
        });
        start_of.push(index);
    }
    let snapshot = snapshot.clone();
    let reach: Vec<HashMap<String, f64>> = tokio::task::spawn_blocking(move || {
        starts
            .par_iter()
            .map(|(junction, asset)| {
                let query = RoutingQuery::new(junction.clone(), String::new()).with_asset_type(asset.clone());
                snapshot.costs_from(&query, now).map(|(costs, _)| costs).unwrap_or_default()
            })
            .collect()
    })
    .await?;
    Ok(CostTable::new(tasks.len(), vehicles.len(), |t, v| {
        let (task, vehicle) = (&tasks[t], &vehicles[v]);
        if vehicle.asset_type != task.required_asset || vehicle.capacity_persons < task.persons() {
            return None;
        }
        reach[start_of[v]].get(&task.junction_id).map(|seconds| seconds.ceil() as u32)
    }))
}

fn escalated(task: &QueuedTask, reason: &str) -> bool {
    task.escalations.iter().any(|r| r == reason)
}
