//! CrisisGraph pipeline process. One binary, one role per process; scale a
//! role by running more processes.
//!
//! Example:
//! cargo run --bin crisisgraph -- --role api
//! cargo run --bin crisisgraph -- --role triage --port 9111
//!
//! In M0 every role starts its runtime (config, Neo4j/Redis, operational
//! schema, `/healthz`, `/readyz`, `/metrics`, heartbeat, graceful shutdown).
//! Role-specific work is added in later milestones. The legacy `server`
//! binary is unaffected.

use std::env;
use std::error::Error;
use std::net::SocketAddr;
use std::time::Duration;

use tracing::{info, warn};

use crisis_graph::config::Config;
use crisis_graph::db::{ensure_ops_schema, Neo4jClient, RedisClient};
use crisis_graph::runtime::{self, ops_router, OpsState, Role, Shutdown};

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

    let app = ops_router(OpsState {
        role,
        neo4j,
        redis,
        metrics,
    });
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
