use chrono::{DateTime, Utc};
use neo4rs::{query, Row};
use serde_json::json;

use super::{new_id, parse_enum, OpsRepoError, OpsRepository, SYSTEM_ACTOR};
use crate::models::{active_task_key, Reservation, ReservationStatus, ResourceKind};

/// Result of a hold attempt. `created` is false when the task already held
/// this vehicle (a repeated or redelivered attempt).
#[derive(Debug, Clone, PartialEq)]
pub struct HoldOutcome {
    pub reservation: Reservation,
    pub created: bool,
}

const RESERVATION_FIELDS: &str = "r.reservation_id AS reservation_id, r.task_id AS task_id, \
     r.incident_id AS incident_id, r.resource_kind AS resource_kind, \
     r.vehicle_id AS vehicle_id, r.shelter_id AS shelter_id, \
     coalesce(r.beds, 0) AS beds, r.status AS status";

impl OpsRepository {
    /// Hold `vehicle_id` for `task_id` until `held_until` (transaction T2,
    /// first step). In one statement: vehicle `AVAILABLE → HELD` (guarded by
    /// status and version), reservation with active keys, audit and outbox.
    ///
    /// Idempotent: if the task already holds this vehicle, the existing
    /// reservation is returned with `created = false`. If the vehicle or the
    /// task is already held by someone else, the database's uniqueness
    /// constraints reject the write and `Conflict` is returned.
    pub async fn hold_vehicle(
        &self,
        task_id: &str,
        incident_id: &str,
        vehicle_id: &str,
        expected_vehicle_version: u64,
        held_until: DateTime<Utc>,
    ) -> Result<HoldOutcome, OpsRepoError> {
        if let Some(existing) = self.active_reservation(task_id, ResourceKind::Vehicle).await? {
            return idempotent_or_conflict(existing, vehicle_id);
        }

        let reservation_id = new_id();
        // Take the vehicle's write lock *before* reading its status, so
        // competing holds queue on the vehicle and the losers see it HELD and
        // stop. Checking first and locking later lets competing statements
        // take the vehicle and unique-index locks in different orders, which
        // Neo4j resolves by aborting them as deadlocks under contention.
        let q = query(
            "MATCH (v:Vehicle {vehicle_id: $vehicle_id}) \
             SET v._lock = true \
             REMOVE v._lock \
             WITH v \
             WHERE v.status = 'AVAILABLE' AND v.version = $expected_version \
             SET v.status = 'HELD', v.version = v.version + 1, v.updated_at = datetime() \
             CREATE (r:Reservation { \
                 reservation_id: $reservation_id, task_id: $task_id, incident_id: $incident_id, \
                 resource_kind: 'VEHICLE', vehicle_id: $vehicle_id, status: 'HELD', \
                 held_until: datetime($held_until), created_at: datetime(), \
                 active_vehicle_key: $vehicle_id, active_task_key: $active_task_key }) \
             CREATE (r)-[:OF_VEHICLE]->(v) \
             CREATE (v)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Vehicle', aggregate_id: $vehicle_id, \
                 action: 'VEHICLE_HELD', actor_id: $actor_id, from_state: 'AVAILABLE', to_state: 'HELD', \
                 previous_version: $expected_version, new_version: v.version, \
                 reason: 'held for task ' + $task_id, occurred_at: datetime(), details_json: $payload }) \
             CREATE (:OutboxEvent { \
                 event_id: $outbox_id, aggregate_id: $task_id, type: 'VEHICLE_HELD', \
                 payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
             WITH r \
             OPTIONAL MATCH (t:Task {task_id: $task_id}) \
             FOREACH (_ IN CASE WHEN t IS NULL THEN [] ELSE [1] END | \
                 CREATE (t)-[:HAS_RESERVATION]->(r)) \
             RETURN r.reservation_id AS reservation_id",
        )
        .param("vehicle_id", vehicle_id)
        .param("expected_version", expected_vehicle_version as i64)
        .param("reservation_id", reservation_id.clone())
        .param("task_id", task_id)
        .param("incident_id", incident_id)
        .param("held_until", held_until.to_rfc3339())
        .param("active_task_key", active_task_key(task_id, ResourceKind::Vehicle))
        .param("audit_id", new_id())
        .param("outbox_id", new_id())
        .param("actor_id", SYSTEM_ACTOR)
        .param(
            "payload",
            json!({
                "task_id": task_id,
                "incident_id": incident_id,
                "vehicle_id": vehicle_id,
                "reservation_id": reservation_id,
                "held_until": held_until.to_rfc3339(),
            })
            .to_string(),
        );

        match self.run_returning_id(q).await {
            Ok(Some(_)) => {
                let reservation = self
                    .get_reservation(&reservation_id)
                    .await?
                    .ok_or_else(|| OpsRepoError::NotFound(reservation_id.clone()))?;
                Ok(HoldOutcome { reservation, created: true })
            }
            Ok(None) => Err(OpsRepoError::NotAvailable(format!(
                "vehicle {vehicle_id} is not AVAILABLE at version {expected_vehicle_version}"
            ))),
            Err(error) if error.is_conflict() => {
                // Lost a race. If it was this same task holding this same
                // vehicle (concurrent redelivery), the outcome is idempotent.
                match self.active_reservation(task_id, ResourceKind::Vehicle).await? {
                    Some(existing) => idempotent_or_conflict(existing, vehicle_id),
                    None => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Confirm a held vehicle after route verification (T2, commit step):
    /// reservation `HELD → COMMITTED`, vehicle `HELD → ASSIGNED`.
    pub async fn commit_vehicle_hold(&self, reservation_id: &str) -> Result<bool, OpsRepoError> {
        let q = query(
            "MATCH (r:Reservation {reservation_id: $reservation_id, status: 'HELD'})-[:OF_VEHICLE]->(v:Vehicle) \
             SET r.status = 'COMMITTED', r.committed_at = datetime(), r.held_until = null, \
                 v.status = 'ASSIGNED', v.version = v.version + 1, v.updated_at = datetime() \
             CREATE (v)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Vehicle', aggregate_id: v.vehicle_id, \
                 action: 'VEHICLE_ASSIGNED', actor_id: $actor_id, from_state: 'HELD', to_state: 'ASSIGNED', \
                 previous_version: v.version - 1, new_version: v.version, \
                 reason: 'assignment committed for task ' + r.task_id, occurred_at: datetime(), \
                 details_json: '{}' }) \
             CREATE (:OutboxEvent { \
                 event_id: $outbox_id, aggregate_id: r.task_id, type: 'TASK_ASSIGNED', \
                 payload_json: '{\"reservation_id\":\"' + r.reservation_id + '\",\"vehicle_id\":\"' + v.vehicle_id + '\"}', \
                 created_at: datetime(), published: false, attempts: 0 }) \
             RETURN r.reservation_id AS reservation_id",
        )
        .param("reservation_id", reservation_id)
        .param("audit_id", new_id())
        .param("outbox_id", new_id())
        .param("actor_id", SYSTEM_ACTOR);
        Ok(self.run_returning_id(q).await?.is_some())
    }

    /// Release an active reservation (T3/T5): status `RELEASED`, active keys
    /// removed, vehicle back to `AVAILABLE`. Returns false if it was not
    /// active (already released or expired), so repeating it is harmless.
    pub async fn release_reservation(&self, reservation_id: &str, reason: &str) -> Result<bool, OpsRepoError> {
        let q = query(
            "MATCH (r:Reservation {reservation_id: $reservation_id}) \
             WHERE r.status IN ['HELD', 'COMMITTED'] \
             OPTIONAL MATCH (r)-[:OF_VEHICLE]->(v:Vehicle) \
             OPTIONAL MATCH (r)-[:AT_SHELTER]->(s:Shelter) \
             SET s._lock = true \
             REMOVE s._lock \
             SET r.status = 'RELEASED', r.released_at = datetime(), r.release_reason = $reason \
             REMOVE r.active_vehicle_key, r.active_task_key \
             FOREACH (_ IN CASE WHEN v IS NULL THEN [] ELSE [1] END | \
                 SET v.status = 'AVAILABLE', v.version = v.version + 1, v.updated_at = datetime()) \
             FOREACH (_ IN CASE WHEN s IS NULL THEN [] ELSE [1] END | \
                 SET s.current_occupancy = CASE WHEN s.current_occupancy >= r.beds \
                         THEN s.current_occupancy - r.beds ELSE 0 END, \
                     s.version = coalesce(s.version, 0) + 1, s.last_updated = datetime()) \
             CREATE (r)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: $audit_id, aggregate_type: 'Reservation', aggregate_id: r.reservation_id, \
                 action: 'RESERVATION_RELEASED', actor_id: $actor_id, to_state: 'RELEASED', \
                 reason: $reason, occurred_at: datetime(), details_json: '{}' }) \
             CREATE (:OutboxEvent { \
                 event_id: $outbox_id, aggregate_id: r.task_id, type: 'RESERVATION_RELEASED', \
                 payload_json: '{\"reservation_id\":\"' + r.reservation_id + '\"}', \
                 created_at: datetime(), published: false, attempts: 0 }) \
             RETURN r.reservation_id AS reservation_id",
        )
        .param("reservation_id", reservation_id)
        .param("reason", reason)
        .param("audit_id", new_id())
        .param("outbox_id", new_id())
        .param("actor_id", SYSTEM_ACTOR);
        Ok(self.run_returning_id(q).await?.is_some())
    }

    /// Expire vehicle holds whose `held_until` has passed (T3). Returns how
    /// many were expired. Committed reservations are never expired.
    pub async fn expire_due_holds(&self, limit: usize) -> Result<u64, OpsRepoError> {
        let q = query(
            "MATCH (r:Reservation {status: 'HELD'}) \
             WHERE r.held_until < datetime() \
             WITH r LIMIT $limit \
             OPTIONAL MATCH (r)-[:OF_VEHICLE]->(v:Vehicle) \
             SET r.status = 'EXPIRED', r.released_at = datetime(), r.release_reason = 'hold expired' \
             REMOVE r.active_vehicle_key, r.active_task_key \
             FOREACH (_ IN CASE WHEN v IS NULL THEN [] ELSE [1] END | \
                 SET v.status = 'AVAILABLE', v.version = v.version + 1, v.updated_at = datetime()) \
             CREATE (r)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
                 event_id: randomUUID(), aggregate_type: 'Reservation', aggregate_id: r.reservation_id, \
                 action: 'RESERVATION_EXPIRED', actor_id: $actor_id, to_state: 'EXPIRED', \
                 reason: 'hold expired', occurred_at: datetime(), details_json: '{}' }) \
             CREATE (:OutboxEvent { \
                 event_id: randomUUID(), aggregate_id: r.task_id, type: 'RESERVATION_EXPIRED', \
                 payload_json: '{\"reservation_id\":\"' + r.reservation_id + '\"}', \
                 created_at: datetime(), published: false, attempts: 0 }) \
             RETURN count(r) AS expired",
        )
        .param("limit", limit as i64)
        .param("actor_id", SYSTEM_ACTOR);
        Ok(match self.write_first_row(q).await? {
            Some(row) => row.get::<i64>("expired")? as u64,
            None => 0,
        })
    }

    pub async fn get_reservation(&self, reservation_id: &str) -> Result<Option<Reservation>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (r:Reservation {{reservation_id: $reservation_id}}) RETURN {RESERVATION_FIELDS}"
        ))
        .param("reservation_id", reservation_id);
        self.one_reservation(q).await
    }

    /// The task's active reservation of the given kind, if any.
    pub async fn active_reservation(
        &self,
        task_id: &str,
        kind: ResourceKind,
    ) -> Result<Option<Reservation>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (r:Reservation {{active_task_key: $key}}) RETURN {RESERVATION_FIELDS}"
        ))
        .param("key", active_task_key(task_id, kind));
        self.one_reservation(q).await
    }

    async fn one_reservation(&self, q: neo4rs::Query) -> Result<Option<Reservation>, OpsRepoError> {
        let mut result = self.neo4j.graph.execute(q).await?;
        match result.next().await? {
            Some(row) => Ok(Some(reservation_from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub(super) async fn run_returning_id(&self, q: neo4rs::Query) -> Result<Option<String>, OpsRepoError> {
        match self.write_first_row(q).await? {
            Some(row) => Ok(Some(row.get::<String>("reservation_id")?)),
            None => Ok(None),
        }
    }
}

fn idempotent_or_conflict(existing: Reservation, vehicle_id: &str) -> Result<HoldOutcome, OpsRepoError> {
    if existing.vehicle_id.as_deref() == Some(vehicle_id) {
        Ok(HoldOutcome { reservation: existing, created: false })
    } else {
        Err(OpsRepoError::Conflict(format!(
            "task {} already holds vehicle {:?}",
            existing.task_id, existing.vehicle_id
        )))
    }
}

pub(super) fn reservation_from_row(row: &Row) -> Result<Reservation, OpsRepoError> {
    Ok(Reservation {
        reservation_id: row.get("reservation_id")?,
        task_id: row.get("task_id")?,
        incident_id: row.get("incident_id")?,
        resource_kind: parse_enum(&row.get::<String>("resource_kind")?, ResourceKind::parse, "resource kind")?,
        vehicle_id: row.get("vehicle_id")?,
        shelter_id: row.get("shelter_id")?,
        beds: row.get::<i64>("beds")? as u32,
        status: parse_enum(&row.get::<String>("status")?, ReservationStatus::parse, "reservation status")?,
    })
}
