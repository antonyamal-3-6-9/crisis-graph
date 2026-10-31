//! Batch assignment (allocation design §3): priority tiers, the cost table
//! and the Hungarian solve, plus the one-at-a-time baseline it is measured
//! against. Pure functions: no I/O, so every rule is unit-tested here.

use chrono::{DateTime, Utc};
use pathfinding::kuhn_munkres::kuhn_munkres_min;
use pathfinding::matrix::Matrix;

/// Priority tier, matched in this order. A lower tier never takes a vehicle
/// a higher tier in the same cycle could use (strict tiers, design §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Critical,
    /// URGENT and UNKNOWN, plus STABLE tasks that have aged.
    Urgent,
    Stable,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Critical, Tier::Urgent, Tier::Stable];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::Urgent => "URGENT",
            Self::Stable => "STABLE",
        }
    }
}

/// The tier a task is matched in: its urgency rank, with STABLE promoted to
/// URGENT after waiting `aging_secs`. Aging never reaches CRITICAL.
pub fn effective_tier(priority_rank: u8, queued_at: DateTime<Utc>, now: DateTime<Utc>, aging_secs: u64) -> Tier {
    match priority_rank {
        2.. => Tier::Critical,
        1 => Tier::Urgent,
        0 if (now - queued_at).num_seconds() >= aging_secs as i64 => Tier::Urgent,
        _ => Tier::Stable,
    }
}

/// Offset that keeps CRITICAL ahead of every other score.
const CRITICAL_OFFSET_S: f64 = 1e10;

/// Score in the priority queue (lower is served first): queue time, moved
/// earlier by the tier. URGENT is moved by the aging time, so a STABLE task
/// that has waited that long sorts with an URGENT task queued now; nothing
/// changes as time passes, so scores never need rewriting.
pub fn queue_score(priority_rank: u8, queued_at: DateTime<Utc>, aging_secs: u64) -> f64 {
    let at = queued_at.timestamp() as f64;
    match priority_rank {
        2.. => at - CRITICAL_OFFSET_S,
        1 => at - aging_secs as f64,
        _ => at,
    }
}

/// Travel time in whole seconds from each vehicle to each task, `None`
/// where the pair is impossible (wrong asset, too many people, no route).
#[derive(Debug, Clone, PartialEq)]
pub struct CostTable {
    /// `costs[task][vehicle]`
    costs: Vec<Vec<Option<u32>>>,
    vehicles: usize,
}

impl CostTable {
    pub fn new(tasks: usize, vehicles: usize, cost: impl Fn(usize, usize) -> Option<u32>) -> Self {
        let costs = (0..tasks).map(|t| (0..vehicles).map(|v| cost(t, v)).collect()).collect();
        Self { costs, vehicles }
    }

    pub fn from_rows(costs: Vec<Vec<Option<u32>>>) -> Self {
        let vehicles = costs.first().map_or(0, Vec::len);
        assert!(costs.iter().all(|row| row.len() == vehicles), "ragged cost table");
        Self { costs, vehicles }
    }

    pub fn get(&self, task: usize, vehicle: usize) -> Option<u32> {
        self.costs[task][vehicle]
    }

    pub fn tasks(&self) -> usize {
        self.costs.len()
    }

    pub fn vehicles(&self) -> usize {
        self.vehicles
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assignment {
    pub task: usize,
    pub vehicle: usize,
    pub cost_s: u32,
}

/// Cost of an impossible pair inside the solver: larger than any sum of real
/// costs, so the solve first maximises how many tasks are served, then
/// minimises their total travel time. Such pairs are dropped afterwards.
const INFEASIBLE: i64 = 1_000_000_000;

/// Assign vehicles tier by tier: each tier is one Hungarian solve over the
/// vehicles the higher tiers left free, minimising the tier's total travel
/// time. Returned in tier order, then task index.
pub fn solve_tiers(tiers: &[Tier], costs: &CostTable) -> Vec<Assignment> {
    assert_eq!(tiers.len(), costs.tasks(), "one tier per task");
    let mut free = vec![true; costs.vehicles()];
    let mut assigned = Vec::new();
    for tier in Tier::ALL {
        // Only tasks and vehicles that can form at least one pair.
        let tasks: Vec<usize> = (0..costs.tasks())
            .filter(|&t| tiers[t] == tier)
            .filter(|&t| (0..costs.vehicles()).any(|v| free[v] && costs.get(t, v).is_some()))
            .collect();
        let vehicles: Vec<usize> = (0..costs.vehicles())
            .filter(|&v| free[v] && tasks.iter().any(|&t| costs.get(t, v).is_some()))
            .collect();
        let mut tier_assigned = solve(&tasks, &vehicles, costs);
        tier_assigned.sort_by_key(|a| a.task);
        for a in &tier_assigned {
            free[a.vehicle] = false;
        }
        assigned.extend(tier_assigned);
    }
    assigned
}

/// One Hungarian solve over the given tasks and vehicles.
fn solve(tasks: &[usize], vehicles: &[usize], costs: &CostTable) -> Vec<Assignment> {
    if tasks.is_empty() || vehicles.is_empty() {
        return Vec::new();
    }
    let weight = |t: usize, v: usize| costs.get(t, v).map_or(INFEASIBLE, i64::from);
    // The solver needs rows <= columns: put the smaller side on the rows.
    let pairs: Vec<(usize, usize)> = if tasks.len() <= vehicles.len() {
        let matrix = Matrix::from_fn(tasks.len(), vehicles.len(), |(r, c)| weight(tasks[r], vehicles[c]));
        let (_, columns) = kuhn_munkres_min(&matrix);
        columns.iter().enumerate().map(|(r, &c)| (tasks[r], vehicles[c])).collect()
    } else {
        let matrix = Matrix::from_fn(vehicles.len(), tasks.len(), |(r, c)| weight(tasks[c], vehicles[r]));
        let (_, columns) = kuhn_munkres_min(&matrix);
        columns.iter().enumerate().map(|(r, &c)| (tasks[c], vehicles[r])).collect()
    };
    pairs
        .into_iter()
        .filter_map(|(task, vehicle)| costs.get(task, vehicle).map(|cost_s| Assignment { task, vehicle, cost_s }))
        .collect()
}

/// One-at-a-time baseline: tasks in `order` (queue order), each taking its
/// nearest free vehicle. Used to measure what batch matching gains.
pub fn greedy(order: &[usize], costs: &CostTable) -> Vec<Assignment> {
    let mut free = vec![true; costs.vehicles()];
    let mut assigned = Vec::new();
    for &task in order {
        let best = (0..costs.vehicles())
            .filter(|&v| free[v])
            .filter_map(|v| costs.get(task, v).map(|cost| (cost, v)))
            .min();
        if let Some((cost_s, vehicle)) = best {
            free[vehicle] = false;
            assigned.push(Assignment { task, vehicle, cost_s });
        }
    }
    assigned
}

/// Total and worst-case travel time of a set of assignments.
pub fn totals(assignments: &[Assignment]) -> (u64, u32) {
    let total = assignments.iter().map(|a| u64::from(a.cost_s)).sum();
    let worst = assignments.iter().map(|a| a.cost_s).max().unwrap_or(0);
    (total, worst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn table(rows: &[&[Option<u32>]]) -> CostTable {
        CostTable::from_rows(rows.iter().map(|r| r.to_vec()).collect())
    }

    #[test]
    fn batch_beats_one_at_a_time_on_the_design_example() {
        // Allocation design §3.1: rows I1, I2; columns A1, A2 (minutes).
        let costs = table(&[&[Some(3), Some(5)], &[Some(4), Some(15)]]);
        let batch = solve_tiers(&[Tier::Urgent, Tier::Urgent], &costs);
        assert_eq!(totals(&batch), (9, 5));
        assert_eq!(batch[0], Assignment { task: 0, vehicle: 1, cost_s: 5 });
        assert_eq!(totals(&greedy(&[0, 1], &costs)), (18, 15));
    }

    #[test]
    fn higher_tier_keeps_the_only_vehicle_even_when_a_lower_tier_is_closer() {
        // Task 0 STABLE is 1 s away; task 1 CRITICAL is 100 s away.
        let costs = table(&[&[Some(1)], &[Some(100)]]);
        let assigned = solve_tiers(&[Tier::Stable, Tier::Critical], &costs);
        assert_eq!(assigned, vec![Assignment { task: 1, vehicle: 0, cost_s: 100 }]);
    }

    #[test]
    fn lower_tier_gets_vehicles_the_higher_tier_cannot_use() {
        // CRITICAL task 0 can use only vehicle 0; STABLE task 1 takes vehicle 1.
        let costs = table(&[&[Some(10), None], &[Some(1), Some(50)]]);
        let assigned = solve_tiers(&[Tier::Critical, Tier::Stable], &costs);
        assert_eq!(
            assigned,
            vec![
                Assignment { task: 0, vehicle: 0, cost_s: 10 },
                Assignment { task: 1, vehicle: 1, cost_s: 50 },
            ]
        );
    }

    #[test]
    fn impossible_pairs_are_never_assigned() {
        let costs = table(&[&[None, None], &[None, Some(7)]]);
        let assigned = solve_tiers(&[Tier::Critical, Tier::Critical], &costs);
        assert_eq!(assigned, vec![Assignment { task: 1, vehicle: 1, cost_s: 7 }]);
        assert!(solve_tiers(&[Tier::Urgent], &table(&[&[None]])).is_empty());
    }

    #[test]
    fn serves_as_many_tasks_as_possible_before_minimising_time() {
        // Task 0 could take vehicle 0 cheaply, but then task 1 has nothing.
        let costs = table(&[&[Some(1), Some(30)], &[Some(2), None]]);
        let assigned = solve_tiers(&[Tier::Urgent, Tier::Urgent], &costs);
        assert_eq!(assigned.len(), 2);
        assert_eq!(totals(&assigned), (32, 30));
    }

    #[test]
    fn more_tasks_than_vehicles_and_more_vehicles_than_tasks() {
        let three_by_one = table(&[&[Some(9)], &[Some(3)], &[Some(5)]]);
        let assigned = solve_tiers(&[Tier::Urgent; 3], &three_by_one);
        assert_eq!(assigned, vec![Assignment { task: 1, vehicle: 0, cost_s: 3 }]);

        let one_by_three = table(&[&[Some(9), Some(3), Some(5)]]);
        let assigned = solve_tiers(&[Tier::Urgent], &one_by_three);
        assert_eq!(assigned, vec![Assignment { task: 0, vehicle: 1, cost_s: 3 }]);
        assert!(solve_tiers(&[], &CostTable::new(0, 0, |_, _| None)).is_empty());
    }

    #[test]
    fn stable_ages_into_urgent_but_never_into_critical() {
        let now = Utc::now();
        let aging = 30 * 60;
        assert_eq!(effective_tier(0, now - Duration::minutes(29), now, aging), Tier::Stable);
        assert_eq!(effective_tier(0, now - Duration::minutes(30), now, aging), Tier::Urgent);
        assert_eq!(effective_tier(1, now - Duration::days(3), now, aging), Tier::Urgent);
        assert_eq!(effective_tier(2, now, now, aging), Tier::Critical);
    }

    #[test]
    fn queue_scores_order_by_tier_then_wait() {
        let now = Utc::now();
        let aging = 30 * 60;
        let critical_new = queue_score(2, now, aging);
        let urgent_old = queue_score(1, now - Duration::hours(5), aging);
        let urgent_new = queue_score(1, now, aging);
        let stable_aged = queue_score(0, now - Duration::minutes(31), aging);
        let stable_new = queue_score(0, now, aging);
        assert!(critical_new < urgent_old && urgent_old < urgent_new);
        assert!(stable_aged < urgent_new, "an aged STABLE task sorts with URGENT");
        assert!(urgent_new < stable_new);
        // Even a STABLE task queued years ago stays behind a new CRITICAL one.
        assert!(critical_new < queue_score(0, now - Duration::days(3650), aging));
    }
}
