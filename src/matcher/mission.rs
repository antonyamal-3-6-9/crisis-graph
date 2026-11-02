//! Boat-then-road conversion (M6b), run by the matcher each cycle.
//!
//! A queued road pickup that no road vehicle can serve is checked for being
//! cut off by flood water: its junction must be unreachable by road from
//! every depot (not merely waiting for a free vehicle). Then the multi-modal
//! planner runs from each boat start on the shared snapshot; a terminal or
//! island plan converts the task into a boat leg and a road leg (owner
//! decision 2026-11-01: automatic, with a dispatcher alert). A fragmented
//! cut, or no plan at all, is escalated `NO_SURFACE_ROUTE` instead.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tracing::info;

use super::{escalated, CycleReport, Matcher, MatcherResult};
use crate::db::{MissionConversion, ESCALATION_NO_SURFACE_ROUTE};
use crate::models::{road_leg_asset, AssetType, QueuedTask};
use crate::solver::{MitigationTopology, MultiModalPlan, RoutingQuery, TransportMode};

/// Seats assumed for an ambulance when the fleet has none to read from.
const DEFAULT_AMBULANCE_SEATS: u32 = 4;

impl Matcher {
    /// Convert or escalate the cut-off tasks among `candidates` (queued road
    /// pickups with no feasible vehicle this cycle).
    pub(super) async fn convert_cut_off(
        &self,
        candidates: Vec<&QueuedTask>,
        fleet: &HashMap<String, u32>,
        now: DateTime<Utc>,
        report: &mut CycleReport,
    ) -> MatcherResult<()> {
        let candidates: Vec<&QueuedTask> = candidates.into_iter().filter(|t| !t.boat_plan_declined).collect();
        if candidates.is_empty() {
            return Ok(());
        }
        let dataset = self.snapshot.dataset().to_string();
        // Cut off: no road vehicle type can reach the victims from any depot,
        // with the same per-type access rules the routes use (an unsuitable
        // track or an hgv=no lane counts as closed to that type). The same
        // reachability decides group plans (T8), so a task is never left
        // between the two checks.
        let reach = self.reach_by_type(now).await?;
        let reachable = |junction: &str| reach.values().any(|r| r.contains(junction));
        let cut_off: Vec<&QueuedTask> = candidates.into_iter().filter(|t| !reachable(&t.incident_junction_id)).collect();
        if cut_off.is_empty() {
            return Ok(());
        }
        let boat_starts = self.repo.boat_starts(&dataset).await?;
        let ambulance_seats = fleet.get(&AssetType::Ambulance.to_string()).copied().unwrap_or(DEFAULT_AMBULANCE_SEATS);

        for task in cut_off {
            let plan = self.best_boat_plan(&boat_starts, &task.incident_junction_id, now).await?;
            let Some((plan, overlay_version)) = plan else {
                if !escalated(task, ESCALATION_NO_SURFACE_ROUTE) {
                    let details = json!({ "reason": "no boat-then-road plan reaches the victims",
                                          "boat_starts": boat_starts.len() });
                    if self.repo.escalate_task(&task.task_id, ESCALATION_NO_SURFACE_ROUTE, details).await? {
                        report.escalated += 1;
                    }
                }
                continue;
            };
            let medical = self.repo.incident_has_medical_need(&task.incident_id).await?;
            let road_asset = road_leg_asset(&task.required_asset, task.persons(), medical, ambulance_seats);
            let (water_legs, water_eta_s) = water_part(&plan);
            let conversion = MissionConversion {
                task: task.clone(),
                staging_junction_id: plan.staging_junction.clone().unwrap_or_else(|| plan.origin.clone()),
                water_legs,
                water_eta_s,
                topology: format!("{:?}", plan.topology),
                road_asset,
                overlay_version,
            };
            if let Some((water, road)) = self.repo.convert_to_mission(&conversion).await? {
                report.converted += 1;
                metrics::counter!("crisisgraph_matcher_missions_converted_total").increment(1);
                info!(task_id = %task.task_id, %water, %road, staging = %conversion.staging_junction_id,
                      topology = %conversion.topology, "Cut-off task converted to a boat-then-road mission");
            }
        }
        Ok(())
    }

    /// The fastest terminal or island plan from any boat start, with the
    /// overlay version it used.
    async fn best_boat_plan(&self, starts: &[String], target: &str, now: DateTime<Utc>) -> MatcherResult<Option<(MultiModalPlan, u64)>> {
        let snapshot = self.snapshot.clone();
        let starts = starts.to_vec();
        let target = target.to_string();
        let speed = self.params.boat_speed_kmh;
        Ok(tokio::task::spawn_blocking(move || {
            starts
                .iter()
                .filter_map(|start| {
                    let query = RoutingQuery::new(start.clone(), target.clone()).with_asset_type(AssetType::RescueBoat);
                    snapshot.plan_mission(&query, now, speed).ok()
                })
                .filter(|(plan, _)| matches!(plan.topology, MitigationTopology::TerminalHazard | MitigationTopology::IslandHazard))
                .min_by(|a, b| a.0.total_travel_time_s.total_cmp(&b.0.total_travel_time_s))
        })
        .await?)
    }
}

/// The part of the plan after the staging point (water, then any dry pocket
/// on foot) and its estimated time. Recorded for the crew; never verified.
fn water_part(plan: &MultiModalPlan) -> (Value, f64) {
    let legs: Vec<&crate::solver::RouteLeg> = plan.legs.iter().filter(|l| l.mode != TransportMode::RoadVehicle).collect();
    let eta = legs.iter().map(|l| l.travel_time_s).sum();
    let json = legs
        .iter()
        .map(|l| {
            json!({
                "mode": format!("{:?}", l.mode), "from": l.origin_junction, "to": l.destination_junction,
                "waypoints": l.waypoints, "segment_ids": l.segment_ids, "distance_km": l.distance_km,
                "travel_time_s": l.travel_time_s, "verified": false,
            })
        })
        .collect();
    (Value::Array(json), eta)
}
