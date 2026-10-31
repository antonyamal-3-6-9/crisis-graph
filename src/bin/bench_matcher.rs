//! Batch-quality bench (M5 acceptance): batch matching (Hungarian per tier)
//! against one-at-a-time nearest-vehicle allocation, on simulated surges over
//! the real Aluva road graph and the seeded pilot fleet. Read-only: it loads
//! the graph snapshot and the available vehicles and writes nothing.
//!
//! Example:
//! cargo run --release --bin bench_matcher -- --sizes 10,25,40,60 --seeds 20 --seed 2026
//!
//! Each surge places N incidents at random junctions (fixed seed, SplitMix64)
//! with a mix of tiers (20% CRITICAL, 50% URGENT, 30% STABLE), assets (50%
//! ambulance, 30% truck, 20% boat) and 1-4 people. Both methods use the
//! same cost table, the same tiers and the same queue order.
//!
//! Two fleet scenarios: `depot` (every vehicle at its home depot, as seeded)
//! and `dispersed` (each vehicle at a random junction, as mid-operation
//! after earlier missions).

use std::env;
use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;

use crisis_graph::config::Config;
use crisis_graph::db::{Neo4jClient, OpsRepository, StateManager};
use crisis_graph::graph::GraphSnapshot;
use crisis_graph::matcher::{
    build_cost_table, effective_tier, greedy, solve_tiers, Assignment, Tier,
};
use crisis_graph::models::{AssetType, QueuedTask};

struct Args {
    sizes: Vec<usize>,
    seeds: u64,
    seed: u64,
    dataset: String,
}

fn parse_args() -> Result<Args, Box<dyn Error + Send + Sync>> {
    let mut args = Args {
        sizes: vec![10, 25, 40, 60],
        seeds: 20,
        seed: 2026,
        dataset: String::new(),
    };
    let mut values = env::args().skip(1);
    while let Some(flag) = values.next() {
        let value = values
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--sizes" => {
                args.sizes = value
                    .split(',')
                    .map(|v| v.trim().parse())
                    .collect::<Result<_, _>>()?
            }
            "--seeds" => args.seeds = value.parse()?,
            "--seed" => args.seed = value.parse()?,
            "--dataset" => args.dataset = value,
            _ => return Err(format!("unknown option {flag}").into()),
        }
    }
    Ok(args)
}

/// SplitMix64: small, deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Default)]
struct Outcome {
    served: usize,
    total_s: u64,
    worst_s: u32,
    critical_worst_s: u32,
}

fn outcome(assignments: &[Assignment], tiers: &[Tier]) -> Outcome {
    Outcome {
        served: assignments.len(),
        total_s: assignments.iter().map(|a| u64::from(a.cost_s)).sum(),
        worst_s: assignments.iter().map(|a| a.cost_s).max().unwrap_or(0),
        critical_worst_s: assignments
            .iter()
            .filter(|a| tiers[a.task] == Tier::Critical)
            .map(|a| a.cost_s)
            .max()
            .unwrap_or(0),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut args = parse_args()?;
    let config = Config::from_env();
    if args.dataset.is_empty() {
        args.dataset = config.params.graph_dataset.clone();
    }
    let neo4j = Neo4jClient::connect(&config).await?;
    let repo = OpsRepository::new(neo4j.clone());
    let snapshot = Arc::new(
        GraphSnapshot::load(repo.clone(), &StateManager::new(neo4j), &args.dataset).await?,
    );
    let depot_fleet = repo.list_available_vehicles_in(&args.dataset).await?;
    let mut junctions: Vec<String> = snapshot
        .baseline()
        .edges()
        .iter()
        .map(|e| e.source.clone())
        .collect();
    junctions.sort();
    junctions.dedup();
    println!(
        "dataset {} · {} junctions · {} available vehicles · overlay v{} ({} hazards) · seed {} · {} surges per size",
        args.dataset,
        junctions.len(),
        depot_fleet.len(),
        snapshot.overlay().version,
        snapshot.overlay().len(),
        args.seed,
        args.seeds
    );
    let now = Utc::now();
    let mut rng = Rng(args.seed);
    for scenario in ["depot", "dispersed"] {
        println!("\n**Fleet: {scenario}**\n");
        println!(
        "| Incidents | Served (batch / greedy) | Total travel, min (batch / greedy) | Worst, min (batch / greedy) | CRITICAL worst, min (batch / greedy) | Batch total better / equal / worse | Batch worst better / equal / worse |"
    );
        println!("|---|---|---|---|---|---|---|");
        for &size in &args.sizes {
            let (mut batch_sum, mut greedy_sum) = (Outcome::default(), Outcome::default());
            let (mut total_cmp, mut worst_cmp) = ([0u32; 3], [0u32; 3]);
            let mut cost_ms = 0u128;
            for _ in 0..args.seeds {
                let mut vehicles = depot_fleet.clone();
                if scenario == "dispersed" {
                    for vehicle in &mut vehicles {
                        vehicle.junction_id =
                            Some(junctions[rng.below(junctions.len() as u64) as usize].clone());
                    }
                }
                let tasks: Vec<QueuedTask> = (0..size)
                    .map(|n| {
                        let rank = match rng.below(10) {
                            0..=1 => 2,
                            2..=6 => 1,
                            _ => 0,
                        };
                        let asset = match rng.below(10) {
                            0..=4 => AssetType::Ambulance,
                            5..=7 => AssetType::EvacTruck,
                            _ => AssetType::RescueBoat,
                        };
                        QueuedTask {
                            task_id: format!("bench-{n}"),
                            incident_id: format!("bench-{n}"),
                            version: 0,
                            required_asset: asset,
                            priority_rank: rank,
                            queued_at: now,
                            headcount: Some(1 + rng.below(4) as u32),
                            junction_id: junctions[rng.below(junctions.len() as u64) as usize]
                                .clone(),
                            verify_failures: 0,
                            escalations: Vec::new(),
                        }
                    })
                    .collect();
                let tiers: Vec<Tier> = tasks
                    .iter()
                    .map(|t| effective_tier(t.priority_rank, t.queued_at, now, 30 * 60))
                    .collect();
                let started = Instant::now();
                let costs = build_cost_table(&snapshot, &tasks, &vehicles, now).await?;
                cost_ms += started.elapsed().as_millis();
                // Queue order: tier first, then arrival (all arrive together).
                let mut order: Vec<usize> = (0..tasks.len()).collect();
                order.sort_by_key(|&t| tiers[t]);

                let batch = outcome(&solve_tiers(&tiers, &costs), &tiers);
                let one_by_one = outcome(&greedy(&order, &costs), &tiers);
                total_cmp[cmp_index(batch.total_s, one_by_one.total_s)] += 1;
                worst_cmp[cmp_index(u64::from(batch.worst_s), u64::from(one_by_one.worst_s))] += 1;
                for (sum, run) in [(&mut batch_sum, &batch), (&mut greedy_sum, &one_by_one)] {
                    sum.served += run.served;
                    sum.total_s += run.total_s;
                    sum.worst_s += run.worst_s;
                    sum.critical_worst_s += run.critical_worst_s;
                }
            }
            let n = args.seeds as f64;
            let minutes = |seconds: f64| seconds / n / 60.0;
            println!(
            "| {size} | {:.1} / {:.1} | {:.1} / {:.1} | {:.1} / {:.1} | {:.1} / {:.1} | {} / {} / {} | {} / {} / {} |",
            batch_sum.served as f64 / n,
            greedy_sum.served as f64 / n,
            minutes(batch_sum.total_s as f64),
            minutes(greedy_sum.total_s as f64),
            minutes(batch_sum.worst_s as f64),
            minutes(greedy_sum.worst_s as f64),
            minutes(batch_sum.critical_worst_s as f64),
            minutes(greedy_sum.critical_worst_s as f64),
            total_cmp[0],
            total_cmp[1],
            total_cmp[2],
            worst_cmp[0],
            worst_cmp[1],
            worst_cmp[2],
        );
            eprintln!(
                "{scenario} size {size}: cost table {:.0} ms per surge",
                cost_ms as f64 / n
            );
        }
    }
    println!("\nMeans over the surges. Served counts can differ only when one method leaves a reachable task without a vehicle.");
    Ok(())
}

/// 0 = batch better (lower), 1 = equal, 2 = worse.
fn cmp_index(batch: u64, one_by_one: u64) -> usize {
    match batch.cmp(&one_by_one) {
        std::cmp::Ordering::Less => 0,
        std::cmp::Ordering::Equal => 1,
        std::cmp::Ordering::Greater => 2,
    }
}
