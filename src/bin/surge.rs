//! Surge load test for the new pipeline (M8). Replaces the legacy
//! `stream_simulator`.
//!
//! ```text
//! cargo run --release --bin surge -- run --run-id capacity-1 --seed 2026 \
//!     --steps 1:120,5:120,20:120 --neighbours 0.1 --repeats 0.1 --hazards-at 150,300
//! cargo run --release --bin surge -- cleanup --run-id capacity-1
//! cargo run --release --bin surge -- page          # docs/results/*.json -> docs/load-test-results.md
//! ```
//!
//! `run` posts the scenario's reports to the intake API at the planned rate,
//! samples queue lag every 2 s, floods roads near reported places at the
//! given times (transaction T7, so routes and boat missions change for real),
//! waits until the pipeline has drained, and writes `docs/results/<run>.json`.
//! Reports carry the scenario's ground truth for the simulated extraction
//! engine unless `--no-tag` is given (a real-model run).
//!
//! `cleanup` removes everything the run created and returns vehicles, beds
//! and hazards to their state before the run.

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use neo4rs::query;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex, Semaphore};

use crisis_graph::config::Config;
use crisis_graph::db::{Neo4jClient, OpsRepository, RedisClient};
use crisis_graph::models::{ActorContext, ActorRole, HazardCommand, HazardSource, HazardStatus};
use crisis_graph::sim::surge::{cleanup_run, plan, quantile, Place, Scenario, Step};
use crisis_graph::triage::TRIAGE_GROUP;

type AnyResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug)]
struct RunArgs {
    scenario: Scenario,
    api: String,
    tag: bool,
    drain_timeout: u64,
    out_dir: PathBuf,
    metrics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Sample {
    t_s: f64,
    stream_length: i64,
    pending: i64,
    oldest_pending_s: f64,
    queue_length: i64,
}

fn usage() -> &'static str {
    "Usage:\n  surge run --run-id <id> [--seed N] [--steps rate:secs,...] [--neighbours F] [--repeats F]\n            [--hazards-at s,...] [--extraction label] [--no-tag] [--notes text] [--api URL]\n            [--drain-timeout secs] [--out dir] [--metrics host:port,...]\n  surge cleanup --run-id <id>\n  surge page [--out dir] [--page docs/load-test-results.md]"
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();
    let flags = parse_flags(&rest)?;
    let config = Config::from_env();
    match command.as_str() {
        "run" => run(&config, run_args(&flags)?).await,
        "cleanup" => {
            let run_id = flags.get("run-id").ok_or("--run-id is required")?;
            cleanup(&config, run_id).await
        }
        "page" => {
            let dir = PathBuf::from(flags.get("out").map(String::as_str).unwrap_or("docs/results"));
            let page = PathBuf::from(flags.get("page").map(String::as_str).unwrap_or("docs/load-test-results.md"));
            write_page(&dir, &page)
        }
        _ => {
            eprintln!("{}", usage());
            std::process::exit(2);
        }
    }
}

fn parse_flags(args: &[String]) -> AnyResult<HashMap<String, String>> {
    let mut flags = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let name = args[i].strip_prefix("--").ok_or_else(|| format!("unexpected argument {}\n{}", args[i], usage()))?;
        if name == "no-tag" {
            flags.insert(name.to_string(), "true".to_string());
            i += 1;
            continue;
        }
        let value = args.get(i + 1).ok_or_else(|| format!("missing value for --{name}"))?;
        flags.insert(name.to_string(), value.clone());
        i += 2;
    }
    Ok(flags)
}

fn run_args(flags: &HashMap<String, String>) -> AnyResult<RunArgs> {
    let get = |k: &str, d: &str| flags.get(k).cloned().unwrap_or_else(|| d.to_string());
    let run_id = flags.get("run-id").ok_or("--run-id is required")?.clone();
    if !run_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("--run-id may contain letters, digits and '-' only".into());
    }
    let steps = get("steps", "1:60")
        .split(',')
        .map(|s| {
            let (rate, secs) = s.split_once(':').ok_or("steps look like rate:secs")?;
            Ok(Step { rate: rate.parse()?, seconds: secs.parse()? })
        })
        .collect::<AnyResult<Vec<_>>>()?;
    let hazards_at = get("hazards-at", "")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u64>().map_err(Into::into))
        .collect::<AnyResult<Vec<_>>>()?;
    let tag = !flags.contains_key("no-tag");
    Ok(RunArgs {
        scenario: Scenario {
            run_id,
            seed: get("seed", "2026").parse()?,
            steps,
            neighbours: get("neighbours", "0.1").parse()?,
            repeats: get("repeats", "0.1").parse()?,
            hazards_at,
            extraction: get("extraction", if tag { "simulated" } else { "model" }),
            notes: get("notes", ""),
        },
        api: get("api", "http://localhost:3100"),
        tag,
        drain_timeout: get("drain-timeout", "900").parse()?,
        out_dir: PathBuf::from(get("out", "docs/results")),
        metrics: get("metrics", "localhost:9101,localhost:9102,localhost:9112")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
    })
}

/// Junctions with a road in and a road out, as report places.
async fn places(neo4j: &Neo4jClient, dataset: &str) -> AnyResult<Vec<Place>> {
    let q = query(
        "MATCH (j:Junction {dataset: $d}) \
         WHERE EXISTS { MATCH ()-[:CONNECTS_TO]->(j) } AND EXISTS { MATCH (j)-[:CONNECTS_TO]->() } \
         RETURN j.id AS id, j.lat AS lat, j.lon AS lon ORDER BY j.id",
    )
    .param("d", dataset);
    let mut result = neo4j.graph.execute(q).await?;
    let mut places = Vec::new();
    while let Some(row) = result.next().await? {
        places.push(Place { junction_id: row.get("id")?, lat: row.get("lat")?, lon: row.get("lon")? });
    }
    Ok(places)
}

async fn run(config: &Config, args: RunArgs) -> AnyResult<()> {
    let dataset = config.params.graph_dataset.clone();
    let neo4j = Neo4jClient::connect(config).await?;
    let redis = RedisClient::connect(config).await?;
    let repo = OpsRepository::new(neo4j.clone());
    let scenario = args.scenario.clone();
    let planned = plan(&scenario, &places(&neo4j, &dataset).await?);
    println!(
        "Run {}: {} reports over {} s (seed {}, extraction: {}{})",
        scenario.run_id,
        planned.len(),
        scenario.duration_secs(),
        scenario.seed,
        scenario.extraction,
        if args.tag { ", ground-truth tags" } else { "" }
    );
    let before = scrape_all(&args.metrics).await;
    let started = Instant::now();
    let started_at = Utc::now();

    // Queue sampling every 2 s until the run is drained.
    let samples = Arc::new(Mutex::new(Vec::<Sample>::new()));
    let sampling = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let sampler = {
        let (redis, samples, sampling) = (redis.clone(), samples.clone(), sampling.clone());
        let (stream, queue) = (config.params.extract_stream.clone(), format!("alloc:queue:{dataset}"));
        tokio::spawn(async move {
            while sampling.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(sample) = sample(&redis, &stream, &queue, started).await {
                    samples.lock().await.push(sample);
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        })
    };

    // Hazards at the planned times, near a place already reported.
    let hazards = {
        let (repo, neo4j, scenario, planned) = (repo.clone(), neo4j.clone(), scenario.clone(), planned.clone());
        let dataset = dataset.clone();
        tokio::spawn(async move {
            let actor = ActorContext::new("surge-tool", [ActorRole::HazardVerifier]);
            let mut applied = Vec::new();
            for (i, at) in scenario.hazards_at.iter().enumerate() {
                tokio::time::sleep_until(tokio::time::Instant::from_std(started + Duration::from_secs(*at))).await;
                let Some(report) = planned.iter().rev().find(|r| r.at_ms <= at * 1000) else { continue };
                let segment = incoming_segment(&neo4j, &dataset, &report.place.junction_id).await.ok().flatten();
                let Some(segment) = segment else { continue };
                let command = HazardCommand {
                    request_id: format!("SURGE-{}-HZ-{i}", scenario.run_id),
                    dataset: dataset.clone(),
                    segment_id: segment.clone(),
                    status: HazardStatus::Flooded,
                    valid_until: Some(Utc::now() + chrono::Duration::hours(2)),
                    reason: format!("surge {}: river over the road", scenario.run_id),
                    source: HazardSource::Operator,
                };
                match repo.apply_hazard(&actor, &command).await {
                    Ok(_) => {
                        println!("  t={at}s flooded {segment}");
                        applied.push(segment);
                    }
                    Err(error) => eprintln!("  hazard at t={at}s failed: {error}"),
                }
            }
            applied
        })
    };

    // Post every report at its planned time.
    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let permits = Arc::new(Semaphore::new(64));
    let intake = Arc::new(Mutex::new(Vec::<(bool, f64)>::new()));
    let mut posts = tokio::task::JoinSet::new();
    for report in planned.clone() {
        tokio::time::sleep_until(tokio::time::Instant::from_std(started + Duration::from_millis(report.at_ms))).await;
        let permit = permits.clone().acquire_owned().await?;
        let (http, url, intake) = (http.clone(), format!("{}/api/v1/reports", args.api), intake.clone());
        let text = if args.tag { format!("{} {}", report.text, report.truth.tag()) } else { report.text.clone() };
        posts.spawn(async move {
            let body = json!({
                "request_id": report.request_id, "device_id": report.device_id, "raw_text": text,
                "latitude": report.place.lat, "longitude": report.place.lon, "accuracy_m": 20.0,
                "channel": "surge_load_test",
            });
            let sent = Instant::now();
            let ok = matches!(http.post(&url).json(&body).send().await, Ok(r) if r.status().is_success());
            intake.lock().await.push((ok, sent.elapsed().as_secs_f64()));
            drop(permit);
        });
    }
    while posts.join_next().await.is_some() {}
    let posted_secs = started.elapsed().as_secs_f64();
    println!("Posted {} reports in {posted_secs:.0} s; waiting for the pipeline to drain…", planned.len());

    // Drain: every report linked to an incident and no task still waiting
    // without an escalation, or the timeout.
    let prefix = format!("SURGE-{}-", scenario.run_id);
    let deadline = Instant::now() + Duration::from_secs(args.drain_timeout);
    let drained = loop {
        let (unlinked, waiting) = progress(&neo4j, &prefix).await?;
        if unlinked == 0 && waiting == 0 {
            break true;
        }
        if Instant::now() >= deadline {
            eprintln!("Drain timeout: {unlinked} report(s) not triaged, {waiting} task(s) still waiting");
            break false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    sampling.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = sampler.await;
    let flooded = hazards.await.unwrap_or_default();
    let after = scrape_all(&args.metrics).await;

    let samples = samples.lock().await.clone();
    let intake = intake.lock().await.clone();
    let results = results(&neo4j, &redis, config, &scenario, &prefix, posted_secs, drained, &samples, &intake, &before, &after, flooded).await?;
    let results = json!({
        "scenario": scenario, "started_at": started_at.to_rfc3339(), "git_commit": git_commit(),
        "machine": machine(), "dataset": dataset, "results": results, "samples": samples,
    });
    std::fs::create_dir_all(&args.out_dir)?;
    let path = args.out_dir.join(format!("{}.json", scenario.run_id));
    std::fs::write(&path, serde_json::to_string_pretty(&results)?)?;
    println!("Results written to {}", path.display());
    print_summary(&results["results"]);
    Ok(())
}

async fn sample(redis: &RedisClient, stream: &str, queue: &str, started: Instant) -> AnyResult<Sample> {
    let mut conn = redis.manager.clone();
    let stream_length: i64 = redis::cmd("XLEN").arg(stream).query_async(&mut conn).await?;
    let summary: redis::Value = redis::cmd("XPENDING").arg(stream).arg(TRIAGE_GROUP).query_async(&mut conn).await?;
    let (pending, oldest) = match &summary {
        redis::Value::Array(parts) => (
            match parts.first() {
                Some(redis::Value::Int(n)) => *n,
                _ => 0,
            },
            match parts.get(1) {
                Some(redis::Value::BulkString(b)) => String::from_utf8(b.clone()).ok(),
                _ => None,
            },
        ),
        _ => (0, None),
    };
    let oldest_pending_s = oldest
        .and_then(|id| id.split('-').next().and_then(|ms| ms.parse::<i64>().ok()))
        .map(|ms| (Utc::now().timestamp_millis() - ms).max(0) as f64 / 1000.0)
        .unwrap_or(0.0);
    let queue_length: i64 = redis::cmd("ZCARD").arg(queue).query_async(&mut conn).await?;
    Ok(Sample { t_s: started.elapsed().as_secs_f64(), stream_length, pending, oldest_pending_s, queue_length })
}

async fn incoming_segment(neo4j: &Neo4jClient, dataset: &str, junction: &str) -> AnyResult<Option<String>> {
    let q = query(
        "MATCH ()-[r:CONNECTS_TO {dataset: $d}]->(:Junction {id: $j, dataset: $d}) \
         WHERE NOT EXISTS { MATCH (h:Hazard {active_key: $d + ':' + r.segment_id}) } \
         RETURN r.segment_id AS id ORDER BY r.segment_id LIMIT 1",
    )
    .param("d", dataset)
    .param("j", junction);
    let mut result = neo4j.graph.execute(q).await?;
    Ok(match result.next().await? {
        Some(row) => Some(row.get("id")?),
        None => None,
    })
}

/// Reports not yet linked to an incident, and tasks still waiting for a
/// vehicle without an escalation.
async fn progress(neo4j: &Neo4jClient, prefix: &str) -> AnyResult<(i64, i64)> {
    let q = query(
        "MATCH (r:Report) WHERE r.request_id STARTS WITH $p \
         OPTIONAL MATCH (r)-[:REPORTS]->(i:OpsIncident) \
         WITH collect(DISTINCT CASE WHEN i IS NULL THEN r END) AS unlinked, collect(DISTINCT i) AS incidents \
         UNWIND (CASE WHEN size(incidents) = 0 THEN [null] ELSE incidents END) AS i \
         OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) \
         WHERE t.status IN ['QUEUED', 'PENDING_TRIGGER'] AND size(coalesce(t.escalations, [])) = 0 \
         RETURN size(unlinked) AS unlinked, count(DISTINCT t) AS waiting",
    )
    .param("p", prefix);
    let mut result = neo4j.graph.execute(q).await?;
    let row = result.next().await?.ok_or("progress query returned no row")?;
    Ok((row.get("unlinked")?, row.get("waiting")?))
}

#[allow(clippy::too_many_arguments)]
async fn results(
    neo4j: &Neo4jClient,
    redis: &RedisClient,
    config: &Config,
    scenario: &Scenario,
    prefix: &str,
    posted_secs: f64,
    drained: bool,
    samples: &[Sample],
    intake: &[(bool, f64)],
    before: &HashMap<String, f64>,
    after: &HashMap<String, f64>,
    flooded: Vec<String>,
) -> AnyResult<Value> {
    // Per report: arrival, extraction, model latency, link kind.
    let q = query(
        "MATCH (r:Report) WHERE r.request_id STARTS WITH $p \
         OPTIONAL MATCH (r)-[l:REPORTS]->(:OpsIncident) \
         RETURN r.received_at.epochMillis AS received, r.extracted_at.epochMillis AS extracted, \
                r.extraction_status AS status, r.extraction_json AS json, collect(l.kind) AS kinds",
    )
    .param("p", prefix);
    let mut rows = neo4j.graph.execute(q).await?;
    let (mut triage_s, mut model_s, mut extracted_at) = (Vec::new(), Vec::new(), Vec::new());
    let (mut failed, mut follow_ups, mut reports) = (0, 0, 0);
    let mut first_received = i64::MAX;
    while let Some(row) = rows.next().await? {
        reports += 1;
        let received: i64 = row.get("received")?;
        first_received = first_received.min(received);
        if let Some(extracted) = row.get::<Option<i64>>("extracted")? {
            triage_s.push((extracted - received) as f64 / 1000.0);
            extracted_at.push(extracted);
        }
        if row.get::<Option<String>>("status")?.as_deref() == Some("FAILED") {
            failed += 1;
        }
        if let Some(text) = row.get::<Option<String>>("json")? {
            if let Some(ms) = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["triage"]["inference"]["latency_ms"].as_f64()) {
                model_s.push(ms / 1000.0);
            }
        }
        if row.get::<Vec<String>>("kinds")?.iter().any(|k| k == "FOLLOW_UP") {
            follow_ups += 1;
        }
    }

    // Per incident: tier, first report, first vehicle committed.
    let q = query(
        "MATCH (r:Report)-[:REPORTS]->(i:OpsIncident) WHERE r.request_id STARTS WITH $p \
         WITH i, min(r.received_at.epochMillis) AS first \
         OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) \
         WITH i, first, collect(t) AS tasks \
         RETURN i.incident_id AS id, i.status AS status, coalesce(i.urgency, 'UNKNOWN') AS urgency, \
                i.mission AS mission, first, \
                reduce(m = null, t IN tasks | CASE WHEN t.assigned_at IS NULL THEN m \
                    WHEN m IS NULL OR t.assigned_at.epochMillis < m THEN t.assigned_at.epochMillis ELSE m END) AS assigned, \
                reduce(v = 0, t IN tasks | v + coalesce(t.verify_failures, 0)) AS verify_failures, \
                [t IN tasks | coalesce(t.escalations, [])] AS escalations, \
                EXISTS { MATCH (i)-[:POSSIBLE_DUPLICATE]-() } AS flagged",
    )
    .param("p", prefix);
    let mut rows = neo4j.graph.execute(q).await?;
    let mut tta: HashMap<&'static str, Vec<f64>> = HashMap::new();
    let mut tta_all = Vec::new();
    let mut assigned_at = Vec::new();
    let (mut incidents, mut review, mut missions, mut flagged, mut verify_failures) = (0, 0, 0, 0, 0i64);
    let mut escalations: HashMap<String, i64> = HashMap::new();
    let mut per_step: Vec<Vec<f64>> = vec![Vec::new(); scenario.steps.len()];
    while let Some(row) = rows.next().await? {
        incidents += 1;
        let status: String = row.get("status")?;
        if status == "REVIEW_REQUIRED" {
            review += 1;
        }
        if row.get::<Option<String>>("mission")?.is_some() {
            missions += 1;
        }
        if row.get::<bool>("flagged")? {
            flagged += 1;
        }
        verify_failures += row.get::<i64>("verify_failures")?;
        for list in row.get::<Vec<Vec<String>>>("escalations")? {
            for reason in list {
                *escalations.entry(reason).or_default() += 1;
            }
        }
        let first: i64 = row.get("first")?;
        if let Some(assigned) = row.get::<Option<i64>>("assigned")? {
            let secs = (assigned - first) as f64 / 1000.0;
            let tier = match row.get::<String>("urgency")?.as_str() {
                "CRITICAL" => "CRITICAL",
                "STABLE" => "STABLE",
                _ => "URGENT",
            };
            tta.entry(tier).or_default().push(secs);
            tta_all.push(secs);
            assigned_at.push(assigned);
            // Which load step the incident arrived in.
            let offset = (first - first_received) as f64 / 1000.0;
            let mut edge = 0.0;
            for (i, step) in scenario.steps.iter().enumerate() {
                edge += step.seconds as f64;
                if offset < edge || i + 1 == scenario.steps.len() {
                    per_step[i].push(secs);
                    break;
                }
            }
        }
    }

    // Per task: queued to vehicle committed (waiting for a free vehicle).
    let q = query(
        "MATCH (r:Report)-[:REPORTS]->(:OpsIncident)-[:HAS_TASK]->(t:Task) \
         WHERE r.request_id STARTS WITH $p AND t.assigned_at IS NOT NULL \
         WITH DISTINCT t \
         RETURN t.assigned_at.epochMillis - coalesce(t.queued_at, t.created_at).epochMillis AS wait_ms",
    )
    .param("p", prefix);
    let mut rows = neo4j.graph.execute(q).await?;
    let mut queue_wait = Vec::new();
    while let Some(row) = rows.next().await? {
        queue_wait.push(row.get::<i64>("wait_ms")? as f64 / 1000.0);
    }

    let pct = |v: &[f64]| json!({ "n": v.len(), "p50": quantile(v, 0.5), "p95": quantile(v, 0.95), "p99": quantile(v, 0.99) });
    let rate = |times: &[i64]| -> Option<f64> {
        let (min, max) = (times.iter().min()?, times.iter().max()?);
        (max > min).then(|| times.len() as f64 / ((max - min) as f64 / 1000.0))
    };
    let accepted = intake.iter().filter(|(ok, _)| *ok).count();
    let intake_s: Vec<f64> = intake.iter().map(|(_, s)| *s).collect();
    let lag: Vec<f64> = samples.iter().map(|s| s.oldest_pending_s).collect();
    let diff = |name: &str| after.get(name).copied().unwrap_or(0.0) - before.get(name).copied().unwrap_or(0.0);
    let mut conn = redis.manager.clone();
    let dead: i64 = redis::cmd("XLEN").arg(format!("{}:dead", config.params.extract_stream)).query_async(&mut conn).await.unwrap_or(0);

    Ok(json!({
        "drained": drained,
        "reports": { "planned": scenario.total_reports(), "stored": reports, "accepted": accepted,
                     "failed_extraction": failed, "follow_ups_attached": follow_ups },
        "incidents": { "total": incidents, "review_required": review, "possible_duplicates_flagged": flagged,
                       "boat_missions": missions, "escalations": escalations },
        "throughput_per_s": {
            "offered": scenario.total_reports() as f64 / scenario.duration_secs().max(1) as f64,
            "accepted": accepted as f64 / posted_secs.max(1.0),
            "triaged": rate(&extracted_at),
            "assigned": rate(&assigned_at),
        },
        "intake_latency_s": pct(&intake_s),
        "triage_latency_s": pct(&triage_s),
        "model_latency_s": pct(&model_s),
        "time_to_assignment_s": {
            "all": pct(&tta_all),
            "CRITICAL": pct(tta.get("CRITICAL").map(Vec::as_slice).unwrap_or(&[])),
            "URGENT": pct(tta.get("URGENT").map(Vec::as_slice).unwrap_or(&[])),
            "STABLE": pct(tta.get("STABLE").map(Vec::as_slice).unwrap_or(&[])),
        },
        "queue_wait_s": pct(&queue_wait),
        "time_to_assignment_by_step_s": scenario.steps.iter().zip(&per_step).map(|(step, v)| {
            json!({ "rate": step.rate, "seconds": step.seconds, "p50": quantile(v, 0.5), "p95": quantile(v, 0.95), "n": v.len() })
        }).collect::<Vec<_>>(),
        "queue_lag_s": { "max": lag.iter().cloned().fold(0.0, f64::max), "p95": quantile(&lag, 0.95) },
        "max_stream_pending": samples.iter().map(|s| s.pending).max().unwrap_or(0),
        "max_alloc_queue": samples.iter().map(|s| s.queue_length).max().unwrap_or(0),
        "verification_failures": verify_failures,
        "overbooking_attempts_blocked": diff("crisisgraph_reservation_conflicts_total"),
        "rate_limited_retries": diff("crisisgraph_triage_rate_limited_total"),
        "dead_letters_total": dead,
        "hazards_flooded": flooded,
    }))
}

/// Sum of each metric (all label sets) across the given `/metrics` endpoints.
async fn scrape_all(endpoints: &[String]) -> HashMap<String, f64> {
    let mut totals = HashMap::new();
    for endpoint in endpoints {
        let Ok(response) = reqwest::get(format!("http://{endpoint}/metrics")).await else { continue };
        let Ok(text) = response.text().await else { continue };
        for line in text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let (name, value) = match line.rsplit_once(' ') {
                Some((left, value)) => (left.split('{').next().unwrap_or(left), value),
                None => continue,
            };
            if let Ok(value) = value.parse::<f64>() {
                *totals.entry(name.to_string()).or_insert(0.0) += value;
            }
        }
    }
    totals
}

fn git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn machine() -> Value {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("model name")).map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string()))
        .unwrap_or_default();
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    let memory_gb = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|t| t.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|kb| kb.parse::<f64>().ok()))
        .map(|kb| (kb / 1024.0 / 1024.0).round())
        .unwrap_or(0.0);
    json!({ "cpu": cpu, "cores": cores, "memory_gb": memory_gb, "gpu": "none", "services": "Neo4j 5 + Redis, native, same machine" })
}

fn print_summary(results: &Value) {
    let tta = &results["time_to_assignment_s"];
    println!(
        "  accepted {}/{} · triaged {:.2}/s · assigned {:.2}/s · time to assignment p50 {:?} s, p95 {:?} s · queue lag max {:.1} s · blocked {}",
        results["reports"]["accepted"],
        results["reports"]["planned"],
        results["throughput_per_s"]["triaged"].as_f64().unwrap_or(0.0),
        results["throughput_per_s"]["assigned"].as_f64().unwrap_or(0.0),
        tta["all"]["p50"].as_f64().map(|v| (v * 10.0).round() / 10.0),
        tta["all"]["p95"].as_f64().map(|v| (v * 10.0).round() / 10.0),
        results["queue_lag_s"]["max"].as_f64().unwrap_or(0.0),
        results["overbooking_attempts_blocked"],
    );
}

// ------------------------------------------------------------------ cleanup

/// Remove a run's records and restore vehicles, beds and hazards.
async fn cleanup(config: &Config, run_id: &str) -> AnyResult<()> {
    let neo4j = Neo4jClient::connect(config).await?;
    let repo = OpsRepository::new(neo4j.clone());
    let done = cleanup_run(&neo4j, &repo, &config.params.graph_dataset, run_id).await?;
    println!("Run {run_id}: removed {} reports, {} incidents, {} tasks", done.reports, done.incidents, done.tasks);
    for segment in &done.hazards_cleared {
        println!("  cleared hazard on {segment}");
    }
    println!("Cleanup done.");
    Ok(())
}

// --------------------------------------------------------------------- page

/// Build the results page from every `<run>.json` in `dir`.
fn write_page(dir: &PathBuf, page: &PathBuf) -> AnyResult<()> {
    let mut runs: Vec<Value> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str(&t).ok())
        .collect();
    runs.sort_by_key(|r| r["started_at"].as_str().unwrap_or_default().to_string());
    let mut md = String::from(
        "# CrisisGraph load-test results\n\n\
         Generated by `cargo run --release --bin surge -- page` from `docs/results/*.json`; every number comes from a recorded run.\n\
         Each run lists its scenario, seed and git commit, so it can be reproduced with `surge run` and the same flags.\n\n\
         **Read with care.** These are single-machine runs (Neo4j, Redis and every role on one laptop, no GPU). \
         Runs labelled *extraction: simulated* replace the language model with the scenario's ground truth after a \
         realistic delay, so they measure CrisisGraph itself (queues, triage workers, urgency rules, deduplication, \
         matcher, routing, live verification), not the model. Runs with a real model are limited by that model's rate.\n\n",
    );
    let f = |v: &Value| v.as_f64().map(|x| format!("{:.1}", x)).unwrap_or_else(|| "—".into());
    let f2 = |v: &Value| v.as_f64().map(|x| format!("{:.2}", x)).unwrap_or_else(|| "—".into());
    for run in &runs {
        let s = &run["scenario"];
        let r = &run["results"];
        let steps = s["steps"].as_array().map(|a| a.iter().map(|st| format!("{} /s × {} s", st["rate"], st["seconds"])).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        md += &format!("## {} (extraction: {})\n\n", s["run_id"].as_str().unwrap_or(""), s["extraction"].as_str().unwrap_or(""));
        md += &format!(
            "- **Scenario:** {steps}; seed {}; neighbours {}; repeat callers {}; hazards at {} s. {}\n",
            s["seed"], s["neighbours"], s["repeats"], s["hazards_at"], s["notes"].as_str().unwrap_or("")
        );
        md += &format!(
            "- **Run:** {} · commit `{}` · {} ({} cores, {} GB) · drained: {}\n\n",
            run["started_at"].as_str().unwrap_or(""),
            run["git_commit"].as_str().unwrap_or(""),
            run["machine"]["cpu"].as_str().unwrap_or(""),
            run["machine"]["cores"],
            run["machine"]["memory_gb"],
            r["drained"]
        );
        md += "| Measure | Value |\n|---|---|\n";
        md += &format!("| Reports planned / accepted | {} / {} |\n", r["reports"]["planned"], r["reports"]["accepted"]);
        md += &format!(
            "| Throughput offered / accepted / triaged / assigned | {} / {} / {} / {} per s |\n",
            f2(&r["throughput_per_s"]["offered"]), f2(&r["throughput_per_s"]["accepted"]), f2(&r["throughput_per_s"]["triaged"]), f2(&r["throughput_per_s"]["assigned"])
        );
        md += &format!("| Intake API latency p50 / p95 | {} / {} s |\n", f2(&r["intake_latency_s"]["p50"]), f2(&r["intake_latency_s"]["p95"]));
        md += &format!("| Report → extracted p50 / p95 | {} / {} s |\n", f(&r["triage_latency_s"]["p50"]), f(&r["triage_latency_s"]["p95"]));
        md += &format!("| Model (or simulated) extraction p50 / p95 | {} / {} s |\n", f(&r["model_latency_s"]["p50"]), f(&r["model_latency_s"]["p95"]));
        for tier in ["CRITICAL", "URGENT", "STABLE"] {
            let t = &r["time_to_assignment_s"][tier];
            md += &format!("| Time to assignment {tier} p50 / p95 (n) | {} / {} s ({}) |\n", f(&t["p50"]), f(&t["p95"]), t["n"]);
        }
        md += &format!("| Task queued → vehicle committed p50 / p95 | {} / {} s |\n", f(&r["queue_wait_s"]["p50"]), f(&r["queue_wait_s"]["p95"]));
        md += &format!("| Queue lag (oldest pending) max / p95 | {} / {} s |\n", f(&r["queue_lag_s"]["max"]), f(&r["queue_lag_s"]["p95"]));
        md += &format!("| Largest allocation queue | {} tasks |\n", r["max_alloc_queue"]);
        md += &format!("| Verification failures | {} |\n", r["verification_failures"]);
        md += &format!("| Overbooking attempts blocked | {} |\n", r["overbooking_attempts_blocked"]);
        md += &format!(
            "| Review / duplicates flagged / follow-ups attached / boat missions | {} / {} / {} / {} |\n",
            r["incidents"]["review_required"], r["incidents"]["possible_duplicates_flagged"], r["reports"]["follow_ups_attached"], r["incidents"]["boat_missions"]
        );
        md += &format!("| Escalations | {} |\n", r["incidents"]["escalations"]);
        md += &format!("| Dead letters (stream total) / 429 retries | {} / {} |\n\n", r["dead_letters_total"], r["rate_limited_retries"]);
        if let Some(steps) = r["time_to_assignment_by_step_s"].as_array().filter(|a| a.len() > 1) {
            md += "Time to assignment by load step:\n\n| Offered rate | p50 | p95 | Incidents |\n|---|---|---|---|\n";
            for st in steps {
                md += &format!("| {} /s for {} s | {} s | {} s | {} |\n", st["rate"], st["seconds"], f(&st["p50"]), f(&st["p95"]), st["n"]);
            }
            md += "\n";
        }
    }
    std::fs::write(page, md)?;
    println!("Wrote {} ({} runs)", page.display(), runs.len());
    Ok(())
}
