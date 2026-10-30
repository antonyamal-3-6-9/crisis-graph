//! Transaction T7, hazard change: the hazard record, `overlay_version` + 1,
//! audit and the `HAZARDS_CHANGED` outbox event, committed together. Every
//! change for a dataset first locks its `:OverlayState`, so changes are
//! serialised and versions strictly increase.

use chrono::{DateTime, Utc};
use neo4rs::{query, Row, Txn};
use serde_json::json;

use super::{new_id, parse_enum, parse_time, txn_first_row, OpsRepoError, OpsRepository, MAX_WRITE_ATTEMPTS};
use crate::models::{
    in_force, ActorContext, HazardCommand, HazardOutcome, HazardRecord, HazardSource, HazardStatus, Permission,
};

const HAZARD_FIELDS: &str = "h.hazard_id AS hazard_id, h.request_id AS request_id, h.dataset AS dataset, \
     h.segment_id AS segment_id, h.status AS status, toString(h.valid_from) AS valid_from, \
     toString(h.valid_until) AS valid_until, h.actor_id AS actor_id, h.reason AS reason, \
     h.source AS source, h.overlay_version AS overlay_version";

const MAX_REASON_CHARS: usize = 500;
const MAX_ID_CHARS: usize = 128;

/// In-memory form of the overlay: the version and every active hazard.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayRead {
    pub overlay_version: u64,
    /// `(segment_id, valid_from, valid_until)` of each active hazard.
    pub hazards: Vec<(String, DateTime<Utc>, Option<DateTime<Utc>>)>,
}

impl OpsRepository {
    /// Apply a hazard (T7). Requires `VerifyHazard`. The segment must exist
    /// in the dataset's baseline. Replaces the segment's active hazard, if
    /// any. A repeated `request_id` returns `Replayed` and changes nothing.
    pub async fn apply_hazard(
        &self,
        actor: &ActorContext,
        command: &HazardCommand,
    ) -> Result<HazardOutcome, OpsRepoError> {
        authorize(actor)?;
        validate(command)?;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut txn = self.neo4j.graph.start_txn().await?;
            let outcome = finish(txn_result(apply_in(&mut txn, actor, command).await), txn).await;
            match outcome {
                Err(OpsRepoError::Transient(_)) if attempt < MAX_WRITE_ATTEMPTS => {
                    tokio::time::sleep(super::jittered_backoff(attempt)).await;
                }
                // A concurrent command with the same request_id won the race.
                Err(OpsRepoError::Conflict(_)) => {
                    return match self.hazard_by_request(&command.request_id).await? {
                        Some(hazard) => Ok(HazardOutcome::Replayed { hazard }),
                        None => Err(OpsRepoError::Conflict("concurrent hazard change".into())),
                    };
                }
                other => return other,
            }
        }
    }

    /// End the segment's active hazard (T7). Requires `VerifyHazard`.
    /// Returns `NothingToClear` (and changes nothing) if none is active.
    pub async fn clear_hazard(
        &self,
        actor: &ActorContext,
        dataset: &str,
        segment_id: &str,
        reason: &str,
    ) -> Result<HazardOutcome, OpsRepoError> {
        authorize(actor)?;
        check_reason(reason)?;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut txn = self.neo4j.graph.start_txn().await?;
            let outcome = finish(txn_result(clear_in(&mut txn, actor, dataset, segment_id, reason).await), txn).await;
            match outcome {
                Err(OpsRepoError::Transient(_)) if attempt < MAX_WRITE_ATTEMPTS => {
                    tokio::time::sleep(super::jittered_backoff(attempt)).await;
                }
                other => return other,
            }
        }
    }

    pub async fn hazard_by_request(&self, request_id: &str) -> Result<Option<HazardRecord>, OpsRepoError> {
        let q = query(&format!("MATCH (h:Hazard {{request_id: $request_id}}) RETURN {HAZARD_FIELDS}"))
            .param("request_id", request_id);
        let mut result = self.neo4j.graph.execute(q).await?;
        match result.next().await? {
            Some(row) => Ok(Some(hazard_from_row(&row)?)),
            None => Ok(None),
        }
    }

    /// Active hazards of a dataset (including ones not yet started or
    /// already past `valid_until`; see [`HazardRecord::in_force_at`]).
    pub async fn active_hazards(&self, dataset: &str) -> Result<Vec<HazardRecord>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (h:Hazard {{dataset: $dataset}}) WHERE h.active_key IS NOT NULL \
             RETURN {HAZARD_FIELDS} ORDER BY h.segment_id"
        ))
        .param("dataset", dataset);
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut hazards = Vec::new();
        while let Some(row) = result.next().await? {
            hazards.push(hazard_from_row(&row)?);
        }
        Ok(hazards)
    }

    /// Current `overlay_version` of a dataset (0 if it has none yet).
    pub async fn overlay_version(&self, dataset: &str) -> Result<u64, OpsRepoError> {
        let q = query("OPTIONAL MATCH (o:OverlayState {dataset: $dataset}) RETURN coalesce(o.overlay_version, 0) AS v")
            .param("dataset", dataset);
        let mut result = self.neo4j.graph.execute(q).await?;
        match result.next().await? {
            Some(row) => Ok(row.get::<i64>("v")? as u64),
            None => Ok(0),
        }
    }

    /// The version and active hazards, read in one statement.
    pub async fn read_overlay(&self, dataset: &str) -> Result<OverlayRead, OpsRepoError> {
        let q = query(
            "OPTIONAL MATCH (o:OverlayState {dataset: $dataset}) \
             WITH coalesce(o.overlay_version, 0) AS version \
             OPTIONAL MATCH (h:Hazard {dataset: $dataset}) WHERE h.active_key IS NOT NULL \
             RETURN version, collect([h.segment_id, toString(h.valid_from), toString(h.valid_until)]) AS hazards",
        )
        .param("dataset", dataset);
        let mut result = self.neo4j.graph.execute(q).await?;
        let row = result
            .next()
            .await?
            .ok_or_else(|| OpsRepoError::Database("overlay read returned no row".into()))?;
        let mut hazards = Vec::new();
        for entry in row.get::<Vec<Vec<Option<String>>>>("hazards")? {
            let (Some(Some(segment)), Some(Some(from))) = (entry.first(), entry.get(1)) else { continue };
            let until = entry.get(2).cloned().flatten().map(|t| parse_time(&t)).transpose()?;
            hazards.push((segment.clone(), parse_time(from)?, until));
        }
        Ok(OverlayRead { overlay_version: row.get::<i64>("version")? as u64, hazards })
    }

    /// Move hazards stored the old way (`operational_status` / `valid_until`
    /// on a `CONNECTS_TO` road segment) into hazard records, then remove those
    /// properties from the road. Hazards still in force become records
    /// through T7 (source `MIGRATION`); expired or `OPEN` ones are dropped.
    /// Idempotent: the request id is derived from the old hazard, and a second
    /// run finds no road properties. Returns how many records were created.
    pub async fn migrate_segment_hazards(&self) -> Result<u64, OpsRepoError> {
        let q = query(
            "MATCH ()-[r:CONNECTS_TO]->() WHERE r.operational_status IS NOT NULL \
             RETURN r.dataset AS dataset, r.segment_id AS segment_id, r.operational_status AS status, \
                    toString(r.valid_until) AS valid_until",
        );
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut old = Vec::new();
        while let Some(row) = result.next().await? {
            old.push((
                row.get::<String>("dataset")?,
                row.get::<String>("segment_id")?,
                row.get::<String>("status")?,
                row.get::<Option<String>>("valid_until")?,
            ));
        }

        let actor = ActorContext::new("migrate-ops", [crate::models::ActorRole::EocAdmin]);
        let mut migrated = 0;
        for (dataset, segment_id, status, valid_until) in old {
            let Some(status) = HazardStatus::parse(&status) else { continue };
            let until = valid_until.as_deref().map(parse_time).transpose()?;
            if until.is_some_and(|t| t <= Utc::now()) {
                continue;
            }
            let command = HazardCommand {
                request_id: format!("migration:{dataset}:{segment_id}:{}", valid_until.as_deref().unwrap_or("open")),
                dataset,
                segment_id,
                status,
                valid_until: until,
                reason: "migrated from road segment properties".into(),
                source: HazardSource::Migration,
            };
            if let HazardOutcome::Applied { .. } = self.apply_hazard(&actor, &command).await? {
                migrated += 1;
            }
        }
        self.neo4j
            .graph
            .run(query(
                "MATCH ()-[r:CONNECTS_TO]->() WHERE r.operational_status IS NOT NULL \
                 REMOVE r.operational_status, r.valid_until",
            ))
            .await?;
        Ok(migrated)
    }

    /// Whether `segment_id` has a hazard in force now (fail closed: an
    /// open-ended hazard is in force).
    pub async fn segment_blocked(&self, dataset: &str, segment_id: &str) -> Result<bool, OpsRepoError> {
        let overlay = self.read_overlay(dataset).await?;
        let now = Utc::now();
        Ok(overlay
            .hazards
            .iter()
            .any(|(segment, from, until)| segment == segment_id && in_force(*from, *until, now)))
    }
}

/// Only a change is committed; `Replayed` and `NothingToClear` roll back.
fn txn_result(outcome: Result<HazardOutcome, OpsRepoError>) -> (Result<HazardOutcome, OpsRepoError>, bool) {
    let commit = matches!(outcome, Ok(HazardOutcome::Applied { .. } | HazardOutcome::Cleared { .. }));
    (outcome, commit)
}

async fn finish(
    (outcome, commit): (Result<HazardOutcome, OpsRepoError>, bool),
    txn: Txn,
) -> Result<HazardOutcome, OpsRepoError> {
    if commit {
        txn.commit().await?;
    } else {
        let _ = txn.rollback().await;
    }
    outcome
}

fn authorize(actor: &ActorContext) -> Result<(), OpsRepoError> {
    if actor.can(Permission::VerifyHazard) {
        Ok(())
    } else {
        Err(OpsRepoError::Forbidden(format!("{} may not change hazards", actor.actor_id)))
    }
}

fn check_reason(reason: &str) -> Result<(), OpsRepoError> {
    let chars = reason.trim().chars().count();
    if chars == 0 || chars > MAX_REASON_CHARS {
        return Err(OpsRepoError::InvalidData(format!("reason must be 1..={MAX_REASON_CHARS} characters")));
    }
    Ok(())
}

fn validate(command: &HazardCommand) -> Result<(), OpsRepoError> {
    for (value, name) in [(&command.request_id, "request_id"), (&command.segment_id, "segment_id"), (&command.dataset, "dataset")] {
        if value.trim().is_empty() || value.chars().count() > MAX_ID_CHARS {
            return Err(OpsRepoError::InvalidData(format!("{name} must be 1..={MAX_ID_CHARS} characters")));
        }
    }
    check_reason(&command.reason)?;
    if command.valid_until.is_some_and(|until| until <= Utc::now()) {
        return Err(OpsRepoError::InvalidData("valid_until must be in the future".into()));
    }
    Ok(())
}

fn active_key(dataset: &str, segment_id: &str) -> String {
    format!("{dataset}:{segment_id}")
}

/// Lock the dataset's overlay state and return its next version.
async fn lock_next_version(txn: &mut Txn, dataset: &str) -> Result<u64, OpsRepoError> {
    let row = txn_first_row(
        txn,
        query(
            "MERGE (o:OverlayState {dataset: $dataset}) \
             ON CREATE SET o.overlay_version = 0, o.updated_at = datetime() \
             SET o._lock = true \
             REMOVE o._lock \
             RETURN o.overlay_version + 1 AS next",
        )
        .param("dataset", dataset),
    )
    .await?
    .ok_or_else(|| OpsRepoError::Database("overlay lock returned no row".into()))?;
    Ok(row.get::<i64>("next")? as u64)
}

async fn apply_in(txn: &mut Txn, actor: &ActorContext, command: &HazardCommand) -> Result<HazardOutcome, OpsRepoError> {
    if let Some(row) = txn_first_row(
        txn,
        query(&format!("MATCH (h:Hazard {{request_id: $request_id}}) RETURN {HAZARD_FIELDS}"))
            .param("request_id", command.request_id.clone()),
    )
    .await?
    {
        return Ok(HazardOutcome::Replayed { hazard: hazard_from_row(&row)? });
    }

    let exists = txn_first_row(
        txn,
        query(
            "OPTIONAL MATCH ()-[r:CONNECTS_TO {dataset: $dataset, segment_id: $segment_id}]->() \
             RETURN count(r) AS n",
        )
        .param("dataset", command.dataset.clone())
        .param("segment_id", command.segment_id.clone()),
    )
    .await?
    .map(|row| row.get::<i64>("n"))
    .transpose()?
    .unwrap_or(0);
    if exists == 0 {
        return Err(OpsRepoError::NotFound(format!(
            "segment {} in dataset {}",
            command.segment_id, command.dataset
        )));
    }

    let version = lock_next_version(txn, &command.dataset).await?;
    let hazard_id = new_id();
    let payload = json!({
        "dataset": command.dataset, "overlay_version": version, "change": "APPLIED",
        "segment_id": command.segment_id, "hazard_id": hazard_id, "status": command.status.as_str(),
    })
    .to_string();
    let row = txn_first_row(
        txn,
        query(&format!(
            "MATCH (o:OverlayState {{dataset: $dataset}}) \
             SET o.overlay_version = $version, o.updated_at = datetime() \
             WITH o \
             OPTIONAL MATCH (old:Hazard {{active_key: $key}}) \
             FOREACH (x IN CASE WHEN old IS NULL THEN [] ELSE [old] END | \
                 SET x.ended_at = datetime(), x.end_reason = 'SUPERSEDED', x.superseded_by = $hazard_id, \
                     x.ended_by = $actor_id \
                 REMOVE x.active_key) \
             WITH old \
             CREATE (h:Hazard {{ \
                 hazard_id: $hazard_id, request_id: $request_id, dataset: $dataset, segment_id: $segment_id, \
                 status: $status, valid_from: datetime(), \
                 valid_until: CASE WHEN $valid_until IS NULL THEN null ELSE datetime($valid_until) END, \
                 actor_id: $actor_id, reason: $reason, source: $source, overlay_version: $version, \
                 active_key: $key, created_at: datetime() }}) \
             CREATE (h)-[:HAS_AUDIT_EVENT]->(:AuditEvent {{ \
                 event_id: $audit_id, aggregate_type: 'Hazard', aggregate_id: $hazard_id, \
                 action: 'HAZARD_APPLIED', actor_id: $actor_id, to_state: $status, new_version: $version, \
                 reason: $reason, occurred_at: datetime(), details_json: $payload }}) \
             CREATE (:OutboxEvent {{ \
                 event_id: $outbox_id, aggregate_id: $dataset, type: 'HAZARDS_CHANGED', \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }}) \
             RETURN {HAZARD_FIELDS}, old.hazard_id AS replaced"
        ))
        .param("dataset", command.dataset.clone())
        .param("segment_id", command.segment_id.clone())
        .param("key", active_key(&command.dataset, &command.segment_id))
        .param("version", version as i64)
        .param("hazard_id", hazard_id)
        .param("request_id", command.request_id.clone())
        .param("status", command.status.as_str())
        .param("valid_until", command.valid_until.map(|t| t.to_rfc3339()))
        .param("actor_id", actor.actor_id.clone())
        .param("reason", command.reason.trim())
        .param("source", command.source.as_str())
        .param("audit_id", new_id())
        .param("outbox_id", new_id())
        .param("payload", payload),
    )
    .await?
    .ok_or_else(|| OpsRepoError::Database("hazard write returned no row".into()))?;
    Ok(HazardOutcome::Applied {
        hazard: hazard_from_row(&row)?,
        overlay_version: version,
        replaced: row.get("replaced")?,
    })
}

async fn clear_in(
    txn: &mut Txn,
    actor: &ActorContext,
    dataset: &str,
    segment_id: &str,
    reason: &str,
) -> Result<HazardOutcome, OpsRepoError> {
    let version = lock_next_version(txn, dataset).await?;
    let key = active_key(dataset, segment_id);
    let active = txn_first_row(
        txn,
        query("MATCH (h:Hazard {active_key: $key}) RETURN h.hazard_id AS hazard_id").param("key", key.clone()),
    )
    .await?;
    let Some(active) = active else {
        // Nothing changes; the transaction is rolled back.
        return Ok(HazardOutcome::NothingToClear);
    };
    let hazard_id: String = active.get("hazard_id")?;
    let payload = json!({
        "dataset": dataset, "overlay_version": version, "change": "CLEARED",
        "segment_id": segment_id, "hazard_id": hazard_id,
    })
    .to_string();
    txn_first_row(
        txn,
        query(
            "MATCH (o:OverlayState {dataset: $dataset}), (h:Hazard {active_key: $key}) \
             SET o.overlay_version = $version, o.updated_at = datetime(), \
                 h.ended_at = datetime(), h.end_reason = 'CLEARED', h.ended_by = $actor_id, \
                 h.clear_reason = $reason \
             REMOVE h.active_key \
             CREATE (h)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Hazard', aggregate_id: h.hazard_id, \
                 action: 'HAZARD_CLEARED', actor_id: $actor_id, to_state: 'CLEARED', new_version: $version, \
                 reason: $reason, occurred_at: datetime(), details_json: $payload }) \
             CREATE (:OutboxEvent { \
                 event_id: $outbox_id, aggregate_id: $dataset, type: 'HAZARDS_CHANGED', \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             RETURN h.hazard_id AS hazard_id",
        )
        .param("dataset", dataset)
        .param("key", key)
        .param("version", version as i64)
        .param("actor_id", actor.actor_id.clone())
        .param("reason", reason.trim())
        .param("audit_id", new_id())
        .param("outbox_id", new_id())
        .param("payload", payload),
    )
    .await?;
    Ok(HazardOutcome::Cleared { hazard_id, overlay_version: version })
}

fn hazard_from_row(row: &Row) -> Result<HazardRecord, OpsRepoError> {
    Ok(HazardRecord {
        hazard_id: row.get("hazard_id")?,
        request_id: row.get("request_id")?,
        dataset: row.get("dataset")?,
        segment_id: row.get("segment_id")?,
        status: parse_enum(&row.get::<String>("status")?, HazardStatus::parse, "hazard status")?,
        valid_from: parse_time(&row.get::<String>("valid_from")?)?,
        valid_until: row.get::<Option<String>>("valid_until")?.map(|t| parse_time(&t)).transpose()?,
        actor_id: row.get("actor_id")?,
        reason: row.get("reason")?,
        source: parse_enum(&row.get::<String>("source")?, HazardSource::parse, "hazard source")?,
        overlay_version: row.get::<i64>("overlay_version")? as u64,
    })
}
