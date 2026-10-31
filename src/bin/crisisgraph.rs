//! CrisisGraph pipeline process. One binary, one role per process; scale a
//! role by running more processes.
//!
//! Example:
//! cargo run --bin crisisgraph -- --role api
//! cargo run --bin crisisgraph -- --role triage --port 9111
//!
//! Every role starts the shared runtime (config, Neo4j/Redis, operational
//! schema, `/healthz`, `/readyz`, `/metrics`, heartbeat, graceful shutdown).
//! Role work so far:
//! - `api`: intake API (`POST /api/v1/reports`, `GET /api/v1/reports/{id}`)
//!   and the hazard command (`/api/v1/hazards`, operator tokens)
//! - `relay`: outbox relay (leader-elected) publishing to Redis Streams
//! - `triage`: consumes `sos:extract`, extracts (llama.cpp/vLLM), applies the
//!   minimal deduplication rules and creates or attaches incidents
//! - `graph`: loads the baseline road graph once and keeps the hazard overlay
//!   current (`HAZARDS_CHANGED` + periodic refresh); `/readyz` waits for it
//! - `matcher`: leader-elected batch allocation (priority queue, cost table on
//!   its own graph snapshot, Hungarian per tier, hold → verify → commit)
//!
//! Other roles are added in later milestones. The legacy `server` binary is
//! unaffected.

use std::env;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

use crisis_graph::api::{hazard_routes, intake_routes, ApiState, HazardApiState, OperatorTokens};
use crisis_graph::config::Config;
use crisis_graph::db::{ensure_ops_schema, Neo4jClient, OpsRepository, RedisClient, StateManager};
use crisis_graph::graph::GraphSnapshot;
use crisis_graph::ingestion::TriageExtractor;
use crisis_graph::matcher::Matcher;
use crisis_graph::pipeline::PipelineStages;
use crisis_graph::triage::{StageOneEngine, TriageWorker};
use crisis_graph::relay::OutboxRelay;
use crisis_graph::runtime::{self, instrument, ops_routes, OpsState, ReadyGate, Role, Shutdown};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct Args {
    role: Role,
    port: u16,
}

fn usage() -> String {
    let roles: Vec<&str> = Role::ALL.iter().map(|r| r.as_str()).collect();
    format!(
        "Usage: cargo run --bin crisisgraph -- --role <{}> [--port <port>]",
        roles.join("|")
    )
}

fn parse_args() -> Result<Args, Box<dyn Error + Send + Sync>> {
    let mut role = None;
    let mut port = None;
    let mut values = env::args().skip(1);

    while let Some(flag) = values.next() {
        if flag == "--help" || flag == "-h" {
            println!("{}", usage());
            std::process::exit(0);
        }
        let value = values
            .next()
            .ok_or_else(|| format!("Missing value for {flag}.\n{}", usage()))?;
        match flag.as_str() {
            "--role" => {
                role = Some(
                    Role::parse(&value)
                        .ok_or_else(|| format!("Unknown role: {value}.\n{}", usage()))?,
                )
            }
            "--port" => {
                port = Some(
                    value
                        .parse::<u16>()
                        .map_err(|_| format!("Invalid port: {value}.\n{}", usage()))?,
                )
            }
            _ => return Err(format!("Unknown option: {flag}.\n{}", usage()).into()),
        }
    }

    let role = role.ok_or_else(|| format!("--role is required.\n{}", usage()))?;
    Ok(Args {
        role,
        port: port.unwrap_or_else(|| role.default_port()),
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args = parse_args()?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "crisis_graph=info,crisisgraph=info,tower_http=info".into()),
        )
        .with_target(true)
        .compact()
        .init();

    let role = args.role;
    let config = Config::from_env();
    info!(role = role.as_str(), port = args.port, "Starting CrisisGraph role");

    let metrics = runtime::metrics::install(role)?;
    let neo4j = Neo4jClient::connect(&config).await?;
    let redis = RedisClient::connect(&config).await?;
    ensure_ops_schema(&neo4j).await?;

    let shutdown = Shutdown::from_os_signals();
    let repo = OpsRepository::new(neo4j.clone());
    let repo_neo4j = neo4j.clone();

    let snapshot_gate = ReadyGate::new("graph_snapshot");
    let gates = if matches!(role, Role::Graph | Role::Matcher) { vec![snapshot_gate.clone()] } else { Vec::new() };
    let mut routes = ops_routes(OpsState {
        role,
        neo4j,
        redis: redis.clone(),
        metrics,
        gates,
    });
    if role == Role::Api {
        let (operators, problems) = OperatorTokens::from_env();
        for problem in &problems {
            warn!("{problem}");
        }
        if operators.is_empty() {
            warn!("No operator tokens configured; hazard commands will be refused (401)");
        } else {
            info!(operators = operators.len(), "Operator tokens loaded");
        }
        routes = routes
            .merge(intake_routes(ApiState { repo: repo.clone() }))
            .merge(hazard_routes(HazardApiState {
                repo: repo.clone(),
                operators: Arc::new(operators),
                default_dataset: config.params.graph_dataset.clone(),
            }));
    }
    let app = instrument(routes);

    let role_task = match role {
        Role::Relay => {
            let relay = OutboxRelay::new(repo.clone(), redis.clone(), config.params.clone());
            Some(tokio::spawn(relay.run(shutdown.clone())))
        }
        Role::Triage => {
            let stages = PipelineStages::new(
                TriageExtractor::new(&config),
                StateManager::new(repo_neo4j.clone()),
                redis.clone(),
            )
            .with_on_scene_radius_m(config.params.on_scene_radius_m);
            let consumer = format!("triage-{}", &uuid::Uuid::new_v4().to_string()[..8]);
            let worker = TriageWorker::new(
                repo.clone(),
                redis.clone(),
                std::sync::Arc::new(StageOneEngine::new(stages)),
                config.params.clone(),
                config.inference_max_concurrency,
                consumer,
            );
            Some(tokio::spawn(worker.run(shutdown.clone())))
        }
        Role::Graph => {
            let repo = repo.clone();
            let state = StateManager::new(repo_neo4j.clone());
            let redis = redis.clone();
            let params = config.params.clone();
            let shutdown = shutdown.clone();
            let gate = snapshot_gate.clone();
            Some(tokio::spawn(async move {
                let Some(snapshot) = load_snapshot(repo, &state, &params.graph_dataset, &shutdown).await else {
                    return;
                };
                gate.set_ready();
                snapshot
                    .run_refresher(
                        redis,
                        params.events_stream.clone(),
                        Duration::from_secs(params.overlay_refresh_secs),
                        shutdown,
                    )
                    .await;
            }))
        }
        Role::Matcher => {
            let repo = repo.clone();
            let neo4j = repo_neo4j.clone();
            let redis = redis.clone();
            let params = config.params.clone();
            let shutdown = shutdown.clone();
            let gate = snapshot_gate.clone();
            Some(tokio::spawn(async move {
                let state = StateManager::new(neo4j.clone());
                let Some(snapshot) = load_snapshot(repo.clone(), &state, &params.graph_dataset, &shutdown).await else {
                    return;
                };
                gate.set_ready();
                let refresher = tokio::spawn(snapshot.clone().run_refresher(
                    redis.clone(),
                    params.events_stream.clone(),
                    Duration::from_secs(params.overlay_refresh_secs),
                    shutdown.clone(),
                ));
                Matcher::new(repo, redis, StateManager::new(neo4j), snapshot, params).run(shutdown).await;
                let _ = refresher.await;
            }))
        }
        _ => None,
    };
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], args.port))).await?;
    info!(role = role.as_str(), addr = %listener.local_addr()?, "Ops endpoints listening");

    let server_shutdown = shutdown.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { server_shutdown.wait().await })
            .await
    });

    let heartbeat_shutdown = shutdown.clone();
    let heartbeat = tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    runtime::metrics::heartbeat(role);
                    tracing::debug!(role = role.as_str(), "heartbeat");
                }
                _ = heartbeat_shutdown.wait() => break,
            }
        }
    });

    shutdown.wait().await;
    runtime::metrics::role_down(role);
    info!(role = role.as_str(), "Draining");

    let drain = Duration::from_secs(config.params.role_drain_timeout_secs);
    let drained = tokio::time::timeout(drain, async {
        let _ = heartbeat.await;
        let _ = server.await;
        if let Some(task) = role_task {
            let _ = task.await;
        }
    })
    .await;

    match drained {
        Ok(()) => info!(role = role.as_str(), "Stopped cleanly"),
        Err(_) => warn!(
            role = role.as_str(),
            timeout_secs = drain.as_secs(),
            "Drain timeout exceeded; exiting"
        ),
    }
    Ok(())
}

/// Load the graph snapshot, retrying every 5 s until it loads (readiness
/// stays false meanwhile). `None` if shutdown came first.
async fn load_snapshot(
    repo: OpsRepository,
    state: &StateManager,
    dataset: &str,
    shutdown: &Shutdown,
) -> Option<Arc<GraphSnapshot>> {
    loop {
        match GraphSnapshot::load(repo.clone(), state, dataset).await {
            Ok(snapshot) => return Some(Arc::new(snapshot)),
            Err(error) => {
                warn!(%error, dataset, "Graph snapshot load failed; retrying");
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    _ = shutdown.wait() => return None,
                }
            }
        }
    }
}
