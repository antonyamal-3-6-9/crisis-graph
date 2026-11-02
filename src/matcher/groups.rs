//! Group plans (T8, owner decision 2026-11-02), run by the matcher each
//! cycle before matching.
//!
//! A queued road pickup that cannot be served as requested is fixed
//! automatically, with a dispatcher alert:
//! - **no route**: no vehicle of the requested type can reach the victims
//!   from any depot (e.g. trucks on `hgv=no` lanes), but another road type
//!   can;
//! - **too big**: more people than the largest vehicle of the requested type.
//!
//! The choice (`models::group_plan`) uses graph reachability only: ambulances
//! for a medical need, else one truck, else ambulances; as many vehicles as
//! the group needs. A pickup no road type can reach is left to the
//! boat-then-road check. Planned incidents keep their pickups covering the
//! headcount: if it rises later, vehicles are added.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde_json::json;
use tracing::info;

use super::{escalated, CycleReport, Matcher, MatcherResult};
use crate::db::ESCALATION_NO_ELIGIBLE_VEHICLE;
use crate::models::{group_plan, split_persons, AssetType, QueuedTask, TaskKind};
use crate::solver::RoutingQuery;

/// Road vehicle types a group plan may choose.
const ROAD_TYPES: [AssetType; 2] = [AssetType::Ambulance, AssetType::EvacTruck];

/// Reachability is reused while the overlay version is unchanged, for at
/// most this long (a hazard can also end at its `valid_until`).
const REACH_MAX_AGE: Duration = Duration::from_secs(30);

type Reach = Arc<HashMap<AssetType, HashSet<String>>>;

/// Reachability by road type, cached per overlay version.
#[derive(Clone, Default)]
pub(super) struct ReachCache(Arc<Mutex<Option<(u64, Instant, Reach)>>>);

impl Matcher {
    /// Plan the window's road pickups that cannot be served as requested,
    /// and top up planned incidents whose headcount rose. Returns how many
    /// tasks changed or were added (the queue must then be re-read).
    pub(super) async fn plan_groups(
        &self,
        tasks: &[QueuedTask],
        fleet: &HashMap<String, u32>,
        now: DateTime<Utc>,
        report: &mut CycleReport,
    ) -> MatcherResult<usize> {
        let mut changed = self.top_up_groups(fleet, report).await?;

        let road_pickups: Vec<&QueuedTask> = tasks
            .iter()
            .filter(|t| t.kind == TaskKind::RoadPickup && ROAD_TYPES.contains(&t.required_asset))
            .collect();
        if road_pickups.is_empty() {
            return Ok(changed);
        }
        let too_big = |t: &QueuedTask| fleet.get(&t.required_asset.to_string()).is_none_or(|c| *c < t.persons());
        // Reachability per type is only computed when a candidate may need it.
        let reach = self.reach_by_type(now).await?;
        for task in road_pickups {
            let reachable: Vec<AssetType> =
                ROAD_TYPES.iter().filter(|a| reach.get(*a).is_some_and(|r| r.contains(&task.incident_junction_id))).cloned().collect();
            let no_route = !reachable.contains(&task.required_asset);
            if !no_route && !too_big(task) {
                continue;
            }
            if reachable.is_empty() {
                continue; // cut off by water: the boat-then-road check decides
            }
            let medical = self.repo.incident_has_medical_need(&task.incident_id).await?;
            let Some(plan) = group_plan(&task.required_asset, task.persons(), medical, &reachable, fleet) else {
                if !escalated(task, ESCALATION_NO_ELIGIBLE_VEHICLE) {
                    let details = json!({ "required_asset": task.required_asset.to_string(), "persons": task.persons(),
                                          "reachable_types": reachable.iter().map(|a| a.to_string()).collect::<Vec<_>>() });
                    if self.repo.escalate_task(&task.task_id, ESCALATION_NO_ELIGIBLE_VEHICLE, details).await? {
                        report.escalated += 1;
                    }
                }
                continue;
            };
            if let Some(added) = self.repo.apply_group_plan(task, &plan).await? {
                report.grouped += 1;
                changed += 1 + added.len();
                metrics::counter!("crisisgraph_matcher_group_plans_total", "reason" => plan.reason.as_str()).increment(1);
                info!(task_id = %task.task_id, requested = %task.required_asset, asset = %plan.asset,
                      shares = ?plan.shares, reason = plan.reason.as_str(), "Road pickup re-planned (T8)");
            }
        }
        Ok(changed)
    }

    /// Add vehicles for planned incidents whose headcount rose.
    async fn top_up_groups(&self, fleet: &HashMap<String, u32>, report: &mut CycleReport) -> MatcherResult<usize> {
        let mut added_total = 0;
        for gap in self.repo.coverage_gaps(self.snapshot.dataset()).await? {
            let Some(seats) = fleet.get(&gap.asset.to_string()).copied().filter(|s| *s > 0) else { continue };
            let shares = split_persons(gap.missing(), seats);
            if let Some(added) = self.repo.top_up_group(&gap, &shares).await? {
                report.grouped += 1;
                added_total += added.len();
                metrics::counter!("crisisgraph_matcher_group_plans_total", "reason" => "HEADCOUNT_ROSE").increment(1);
                info!(incident_id = %gap.incident_id, asset = %gap.asset, ?shares, "Planned incident topped up (headcount rose)");
            }
        }
        Ok(added_total)
    }

    /// Junctions each road vehicle type can reach from any depot (hazards in
    /// force masked; access rules such as `hgv=no` applied).
    async fn reach_by_type(&self, now: DateTime<Utc>) -> MatcherResult<Reach> {
        let version = self.snapshot.overlay().version;
        if let Some((cached_version, at, reach)) = self.reach_cache.0.lock().expect("reach cache").clone() {
            if cached_version == version && at.elapsed() < REACH_MAX_AGE {
                return Ok(reach);
            }
        }
        let reach = Arc::new(self.compute_reach(now).await?);
        *self.reach_cache.0.lock().expect("reach cache") = Some((version, Instant::now(), reach.clone()));
        Ok(reach)
    }

    async fn compute_reach(&self, now: DateTime<Utc>) -> MatcherResult<HashMap<AssetType, HashSet<String>>> {
        let depots = self.repo.depot_junctions(self.snapshot.dataset()).await?;
        let snapshot = self.snapshot.clone();
        Ok(tokio::task::spawn_blocking(move || {
            ROAD_TYPES
                .iter()
                .map(|asset| {
                    let mut reach = HashSet::new();
                    for depot in &depots {
                        let query = RoutingQuery::new(depot.clone(), String::new()).with_asset_type(asset.clone());
                        if let Ok((costs, _)) = snapshot.costs_from(&query, now) {
                            reach.extend(costs.into_keys());
                        }
                    }
                    (asset.clone(), reach)
                })
                .collect()
        })
        .await?)
    }
}
