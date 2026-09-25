use async_trait::async_trait;
use neo4rs::query;
use thiserror::Error;
use uuid::Uuid;

use super::Neo4jClient;
use crate::models::{AuditEvent, Incident, IncidentStatus};

#[derive(Clone)]
pub struct IncidentRepository {
    neo4j: Neo4jClient,
}

#[async_trait]
pub trait IncidentStore: Clone + Send + Sync + 'static {
    async fn create_incident(
        &self,
        incident: &Incident,
        receipt: &AuditEvent,
    ) -> Result<(), IncidentRepositoryError>;

    async fn save_incident_transition(
        &self,
        incident: &Incident,
        event: &AuditEvent,
    ) -> Result<(), IncidentRepositoryError>;

    async fn load_incident(
        &self,
        incident_id: &str,
    ) -> Result<Option<Incident>, IncidentRepositoryError>;
}

impl IncidentRepository {
    pub fn new(neo4j: Neo4jClient) -> Self {
        Self { neo4j }
    }

    pub async fn setup_schema(&self) -> Result<(), IncidentRepositoryError> {
        let statements = [
            "CREATE CONSTRAINT incident_id_unique IF NOT EXISTS \
             FOR (i:Incident) REQUIRE i.incident_id IS UNIQUE",
            "CREATE CONSTRAINT incident_audit_event_id_unique IF NOT EXISTS \
             FOR (e:IncidentAuditEvent) REQUIRE e.event_id IS UNIQUE",
            "CREATE INDEX incident_status IF NOT EXISTS FOR (i:Incident) ON (i.status)",
            "CREATE INDEX incident_updated_at IF NOT EXISTS FOR (i:Incident) ON (i.updated_at)",
        ];

        for statement in statements {
            self.neo4j
                .graph
                .run(query(statement))
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }

    /// Creates the incident and receipt audit event in one atomic Cypher write.
    pub async fn create(
        &self,
        incident: &Incident,
        receipt: &AuditEvent,
    ) -> Result<(), IncidentRepositoryError> {
        validate_create(incident, receipt)?;

        let incident_json = serde_json::to_string(incident)?;
        let event_json = serde_json::to_string(receipt)?;
        let creation_token = Uuid::new_v4().to_string();
        let q = query(
            "MERGE (i:Incident {incident_id: $incident_id}) \
             ON CREATE SET i.status = $status, i.version = $version, \
                 i.source_channel = $source_channel, i.created_at = $created_at, \
                 i.updated_at = $updated_at, i.snapshot_json = $snapshot_json, \
                 i.creation_token = $creation_token \
             WITH i, coalesce(i.creation_token = $creation_token, false) AS created \
             FOREACH (_ IN CASE WHEN created THEN [1] ELSE [] END | \
                 CREATE (e:IncidentAuditEvent { \
                     event_id: $event_id, incident_id: $incident_id, action: $action, \
                     actor_id: $actor_id, from_status: $from_status, to_status: $to_status, \
                     previous_version: $previous_version, new_version: $new_version, \
                     reason: $reason, occurred_at: $occurred_at, event_json: $event_json \
                 }) \
                 CREATE (i)-[:HAS_AUDIT_EVENT]->(e) \
             ) \
             REMOVE i.creation_token \
             RETURN created",
        )
        .param("incident_id", incident.incident_id.clone())
        .param("status", incident.status.as_str())
        .param("version", incident.version as i64)
        .param(
            "source_channel",
            incident
                .original_sos
                .source_channel
                .clone()
                .unwrap_or_default(),
        )
        .param("created_at", incident.created_at.to_rfc3339())
        .param("updated_at", incident.updated_at.to_rfc3339())
        .param("snapshot_json", incident_json)
        .param("creation_token", creation_token)
        .param("event_id", receipt.event_id.clone())
        .param("action", receipt.action.clone())
        .param("actor_id", receipt.actor_id.clone())
        .param("from_status", receipt.from_status.as_str())
        .param("to_status", receipt.to_status.as_str())
        .param("previous_version", receipt.previous_version as i64)
        .param("new_version", receipt.new_version as i64)
        .param("reason", receipt.reason.clone())
        .param("occurred_at", receipt.occurred_at.to_rfc3339())
        .param("event_json", event_json);

        let mut result = self.neo4j.graph.execute(q).await.map_err(database_error)?;
        let row = result
            .next()
            .await
            .map_err(database_error)?
            .ok_or_else(|| {
                IncidentRepositoryError::Database(
                    "atomic incident creation returned no result".to_string(),
                )
            })?;
        let created: bool = row.get("created").map_err(database_error)?;
        if created {
            Ok(())
        } else {
            Err(IncidentRepositoryError::AlreadyExists(
                incident.incident_id.clone(),
            ))
        }
    }

    pub async fn get(
        &self,
        incident_id: &str,
    ) -> Result<Option<Incident>, IncidentRepositoryError> {
        let q = query(
            "MATCH (i:Incident {incident_id: $incident_id}) \
             RETURN i.snapshot_json AS snapshot_json",
        )
        .param("incident_id", incident_id);
        let mut result = self.neo4j.graph.execute(q).await.map_err(database_error)?;
        let Some(row) = result.next().await.map_err(database_error)? else {
            return Ok(None);
        };
        let snapshot: String = row.get("snapshot_json").map_err(database_error)?;
        Ok(Some(serde_json::from_str(&snapshot)?))
    }

    /// Replaces the snapshot only when the stored version matches the audit
    /// event's previous version. Snapshot and event are written atomically.
    pub async fn save_transition(
        &self,
        incident: &Incident,
        event: &AuditEvent,
    ) -> Result<(), IncidentRepositoryError> {
        validate_transition_write(incident, event)?;

        let incident_json = serde_json::to_string(incident)?;
        let event_json = serde_json::to_string(event)?;
        let q = query(
            "MATCH (i:Incident {incident_id: $incident_id, version: $previous_version}) \
             SET i.status = $status, i.version = $new_version, \
                 i.updated_at = $updated_at, i.snapshot_json = $snapshot_json \
             CREATE (e:IncidentAuditEvent { \
                 event_id: $event_id, incident_id: $incident_id, action: $action, \
                 actor_id: $actor_id, from_status: $from_status, to_status: $to_status, \
                 previous_version: $previous_version, new_version: $new_version, \
                 reason: $reason, occurred_at: $occurred_at, event_json: $event_json \
             }) \
             CREATE (i)-[:HAS_AUDIT_EVENT]->(e) \
             RETURN i.version AS version",
        )
        .param("incident_id", incident.incident_id.clone())
        .param("status", incident.status.as_str())
        .param("previous_version", event.previous_version as i64)
        .param("new_version", event.new_version as i64)
        .param("updated_at", incident.updated_at.to_rfc3339())
        .param("snapshot_json", incident_json)
        .param("event_id", event.event_id.clone())
        .param("action", event.action.clone())
        .param("actor_id", event.actor_id.clone())
        .param("from_status", event.from_status.as_str())
        .param("to_status", event.to_status.as_str())
        .param("reason", event.reason.clone())
        .param("occurred_at", event.occurred_at.to_rfc3339())
        .param("event_json", event_json);

        let mut result = self.neo4j.graph.execute(q).await.map_err(database_error)?;
        if result.next().await.map_err(database_error)?.is_some() {
            return Ok(());
        }

        match self.get(&incident.incident_id).await? {
            Some(current) => Err(IncidentRepositoryError::VersionConflict {
                incident_id: incident.incident_id.clone(),
                expected: event.previous_version,
                actual: current.version,
            }),
            None => Err(IncidentRepositoryError::NotFound(
                incident.incident_id.clone(),
            )),
        }
    }

    pub async fn list_by_status(
        &self,
        status: IncidentStatus,
        limit: usize,
    ) -> Result<Vec<Incident>, IncidentRepositoryError> {
        let bounded_limit = limit.clamp(1, 500);
        let q = query(
            "MATCH (i:Incident {status: $status}) \
             RETURN i.snapshot_json AS snapshot_json \
             ORDER BY i.updated_at ASC LIMIT $limit",
        )
        .param("status", status.as_str())
        .param("limit", bounded_limit as i64);
        let mut result = self.neo4j.graph.execute(q).await.map_err(database_error)?;
        let mut incidents = Vec::new();
        while let Some(row) = result.next().await.map_err(database_error)? {
            let snapshot: String = row.get("snapshot_json").map_err(database_error)?;
            incidents.push(serde_json::from_str(&snapshot)?);
        }
        Ok(incidents)
    }

    /// Returns the most recently updated incidents for the read-only operator
    /// console. The hard upper bound keeps this inspection endpoint from
    /// turning into an unbounded graph scan.
    pub async fn list_recent(
        &self,
        limit: usize,
    ) -> Result<Vec<Incident>, IncidentRepositoryError> {
        let bounded_limit = limit.clamp(1, 200);
        let q = query(
            "MATCH (i:Incident) \
             RETURN i.snapshot_json AS snapshot_json \
             ORDER BY i.updated_at DESC LIMIT $limit",
        )
        .param("limit", bounded_limit as i64);
        let mut result = self.neo4j.graph.execute(q).await.map_err(database_error)?;
        let mut incidents = Vec::new();
        while let Some(row) = result.next().await.map_err(database_error)? {
            let snapshot: String = row.get("snapshot_json").map_err(database_error)?;
            incidents.push(serde_json::from_str(&snapshot)?);
        }
        Ok(incidents)
    }

    pub async fn audit_history(
        &self,
        incident_id: &str,
    ) -> Result<Vec<AuditEvent>, IncidentRepositoryError> {
        let q = query(
            "MATCH (:Incident {incident_id: $incident_id})-[:HAS_AUDIT_EVENT]->(e) \
             RETURN e.event_json AS event_json \
             ORDER BY e.new_version ASC, e.occurred_at ASC",
        )
        .param("incident_id", incident_id);
        let mut result = self.neo4j.graph.execute(q).await.map_err(database_error)?;
        let mut events = Vec::new();
        while let Some(row) = result.next().await.map_err(database_error)? {
            let event_json: String = row.get("event_json").map_err(database_error)?;
            events.push(serde_json::from_str(&event_json)?);
        }
        Ok(events)
    }
}

#[async_trait]
impl IncidentStore for IncidentRepository {
    async fn create_incident(
        &self,
        incident: &Incident,
        receipt: &AuditEvent,
    ) -> Result<(), IncidentRepositoryError> {
        self.create(incident, receipt).await
    }

    async fn save_incident_transition(
        &self,
        incident: &Incident,
        event: &AuditEvent,
    ) -> Result<(), IncidentRepositoryError> {
        self.save_transition(incident, event).await
    }

    async fn load_incident(
        &self,
        incident_id: &str,
    ) -> Result<Option<Incident>, IncidentRepositoryError> {
        self.get(incident_id).await
    }
}

fn validate_create(
    incident: &Incident,
    receipt: &AuditEvent,
) -> Result<(), IncidentRepositoryError> {
    if incident.version != 1
        || incident.status != IncidentStatus::Received
        || receipt.incident_id != incident.incident_id
        || receipt.previous_version != 0
        || receipt.new_version != 1
        || receipt.action != "INCIDENT_RECEIVED"
    {
        return Err(IncidentRepositoryError::InvalidWrite(
            "incident creation requires a RECEIVED v1 snapshot and matching receipt event"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_transition_write(
    incident: &Incident,
    event: &AuditEvent,
) -> Result<(), IncidentRepositoryError> {
    if event.incident_id != incident.incident_id
        || event.new_version != incident.version
        || event.previous_version + 1 != event.new_version
        || event.to_status != incident.status
    {
        return Err(IncidentRepositoryError::InvalidWrite(
            "incident snapshot and audit event versions/status do not agree".to_string(),
        ));
    }
    Ok(())
}

fn database_error(error: impl std::fmt::Display) -> IncidentRepositoryError {
    IncidentRepositoryError::Database(error.to_string())
}

#[derive(Debug, Error)]
pub enum IncidentRepositoryError {
    #[error("incident '{0}' already exists")]
    AlreadyExists(String),
    #[error("incident '{0}' was not found")]
    NotFound(String),
    #[error(
        "incident '{incident_id}' version conflict: expected {expected}, current version is {actual}"
    )]
    VersionConflict {
        incident_id: String,
        expected: u64,
        actual: u64,
    },
    #[error("invalid incident repository write: {0}")]
    InvalidWrite(String),
    #[error("incident serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Neo4j incident repository error: {0}")]
    Database(String),
}
