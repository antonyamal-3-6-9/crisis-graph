//! Repository for the operational data model. Each write that another stage
//! must react to records its `:AuditEvent` and `:OutboxEvent` in the same
//! Cypher statement as the change itself, so they commit atomically.

mod beds;
mod outbox;
mod reports;
mod reservations;
mod tasks;
mod vehicles;

pub use reservations::HoldOutcome;
pub use vehicles::{FleetCapacities, BASELINE_FLEET};

use chrono::{DateTime, Utc};
use thiserror::Error;

use super::Neo4jClient;
use crate::models::AssetType;

const CONSTRAINT_VIOLATION: &str = "Neo.ClientError.Schema.ConstraintValidationFailed";
const TRANSIENT: &str = "Neo.TransientError";

/// Attempts for a write that fails transiently (e.g. deadlock detected).
const MAX_WRITE_ATTEMPTS: u32 = 5;

/// Actor recorded on audit events written by the pipeline itself.
pub const SYSTEM_ACTOR: &str = "crisisgraph-pipeline";

#[derive(Debug, Error)]
pub enum OpsRepoError {
    /// A uniqueness constraint rejected the write: another active reservation
    /// already holds the vehicle or the task.
    #[error("conflict: {0}")]
    Conflict(String),
    /// The record exists but is not in the expected state or version.
    #[error("not available: {0}")]
    NotAvailable(String),
    /// Neo4j aborted the transaction for a retryable reason (deadlock,
    /// lock timeout). Writes are retried internally before this surfaces.
    #[error("transient database error: {0}")]
    Transient(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid data: {0}")]
    InvalidData(String),
    #[error("database error: {0}")]
    Database(String),
}

impl OpsRepoError {
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Conflict(_))
    }
}

impl From<neo4rs::Error> for OpsRepoError {
    fn from(error: neo4rs::Error) -> Self {
        match &error {
            neo4rs::Error::Neo4j(e) if e.code() == CONSTRAINT_VIOLATION => {
                Self::Conflict(e.message().to_string())
            }
            // neo4rs 0.8 reports failures raised while streaming results
            // (PULL) as an unexpected message carrying the server's code.
            neo4rs::Error::UnexpectedMessage(message) if message.contains(CONSTRAINT_VIOLATION) => {
                Self::Conflict(message.clone())
            }
            neo4rs::Error::Neo4j(e) if e.code().starts_with(TRANSIENT) => {
                Self::Transient(e.message().to_string())
            }
            neo4rs::Error::UnexpectedMessage(message) if message.contains(TRANSIENT) => {
                Self::Transient(message.clone())
            }
            _ => Self::Database(error.to_string()),
        }
    }
}

impl From<neo4rs::DeError> for OpsRepoError {
    fn from(error: neo4rs::DeError) -> Self {
        Self::InvalidData(error.to_string())
    }
}

/// Repository over the operational nodes. Cheap to clone.
#[derive(Clone)]
pub struct OpsRepository {
    neo4j: Neo4jClient,
}

impl OpsRepository {
    pub fn new(neo4j: Neo4jClient) -> Self {
        Self { neo4j }
    }

    /// Run a single write statement and return its first row. If Neo4j
    /// aborts it transiently (e.g. a deadlock between competing writers),
    /// the whole statement is retried with jittered backoff so retries do not
    /// collide again. Each attempt is its own transaction, so a failed
    /// attempt leaves nothing behind.
    async fn write_first_row(&self, q: neo4rs::Query) -> Result<Option<neo4rs::Row>, OpsRepoError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let outcome = async {
                let mut result = self.neo4j.graph.execute(q.clone()).await?;
                Ok::<_, OpsRepoError>(result.next().await?)
            }
            .await;
            match outcome {
                Err(OpsRepoError::Transient(_)) if attempt < MAX_WRITE_ATTEMPTS => {
                    tokio::time::sleep(jittered_backoff(attempt)).await;
                }
                other => return other,
            }
        }
    }
}

/// 10–40 ms × attempt, randomised so competing writers spread out.
fn jittered_backoff(attempt: u32) -> std::time::Duration {
    let jitter = (uuid::Uuid::new_v4().as_u128() % 30) as u64;
    std::time::Duration::from_millis((10 + jitter) * attempt as u64)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn parse_asset(value: &str) -> Result<AssetType, OpsRepoError> {
    match value {
        "RESCUE_BOAT" => Ok(AssetType::RescueBoat),
        "AMBULANCE" => Ok(AssetType::Ambulance),
        "EVAC_TRUCK" => Ok(AssetType::EvacTruck),
        "HELICOPTER" => Ok(AssetType::Helicopter),
        other => Err(OpsRepoError::InvalidData(format!("unknown asset type '{other}'"))),
    }
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, OpsRepoError> {
    DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| OpsRepoError::InvalidData(format!("bad timestamp '{value}': {e}")))
}

fn parse_enum<T>(value: &str, parse: fn(&str) -> Option<T>, what: &str) -> Result<T, OpsRepoError> {
    parse(value).ok_or_else(|| OpsRepoError::InvalidData(format!("unknown {what} '{value}'")))
}
