use neo4rs::query;
use serde_json::json;

use super::{new_id, OpsRepoError, OpsRepository, SYSTEM_ACTOR};
use crate::models::{active_task_key, Reservation, ResourceKind};

impl OpsRepository {
    /// Reserve `beds` drop-off beds at a shelter for a task.
    ///
    /// Beds are counts shared with the legacy pipeline (`capacity` /
    /// `current_occupancy`). Counts cannot be protected by a uniqueness
    /// constraint, so the statement takes the shelter's write lock *before*
    /// reading the counter (`SET s._lock` / `REMOVE s._lock`); concurrent
    /// reservations then see each other's committed updates and capacity can
    /// never be exceeded. One active bed reservation per task is enforced by
    /// `active_task_key`; a repeated call returns the existing reservation.
    pub async fn reserve_beds(
        &self,
        shelter_id: &str,
        beds: u32,
        task_id: &str,
        incident_id: &str,
    ) -> Result<Reservation, OpsRepoError> {
        if beds == 0 {
            return Err(OpsRepoError::InvalidData("bed count must be positive".into()));
        }
        if let Some(existing) = self.active_reservation(task_id, ResourceKind::Beds).await? {
            return Ok(existing);
        }

        let reservation_id = new_id();
        let q = reserve_beds_query(shelter_id, beds, task_id, incident_id, &reservation_id);

        match self.run_returning_id(q).await {
            Ok(Some(_)) => self
                .get_reservation(&reservation_id)
                .await?
                .ok_or(OpsRepoError::NotFound(reservation_id)),
            Ok(None) => Err(OpsRepoError::NotAvailable(format!(
                "shelter {shelter_id} has fewer than {beds} free beds (or does not exist)"
            ))),
            Err(error) if error.is_conflict() => self
                .active_reservation(task_id, ResourceKind::Beds)
                .await?
                .ok_or(error),
            Err(error) => Err(error),
        }
    }
}

/// The bed reservation statement (shared lock, capacity check, counter,
/// reservation, audit, outbox). Returns a row only if the beds were free.
/// Also run inside the T2 commit transaction (`commit_assignment`).
pub(super) fn reserve_beds_query(
    shelter_id: &str,
    beds: u32,
    task_id: &str,
    incident_id: &str,
    reservation_id: &str,
) -> neo4rs::Query {
    query(
        "MATCH (s:Shelter {id: $shelter_id}) \
         SET s._lock = true \
         REMOVE s._lock \
         WITH s \
         WHERE s.capacity - s.current_occupancy >= $beds \
         SET s.current_occupancy = s.current_occupancy + $beds, \
             s.version = coalesce(s.version, 0) + 1, s.last_updated = datetime() \
         CREATE (r:Reservation { \
             reservation_id: $reservation_id, task_id: $task_id, incident_id: $incident_id, \
             resource_kind: 'BEDS', shelter_id: $shelter_id, beds: $beds, status: 'COMMITTED', \
             created_at: datetime(), active_task_key: $active_task_key }) \
         CREATE (r)-[:AT_SHELTER {beds: $beds}]->(s) \
         CREATE (r)-[:HAS_AUDIT_EVENT]->(:AuditEvent { \
             event_id: $audit_id, aggregate_type: 'Shelter', aggregate_id: $shelter_id, \
             action: 'BEDS_RESERVED', actor_id: $actor_id, \
             reason: toString($beds) + ' beds for task ' + $task_id, occurred_at: datetime(), \
             details_json: $payload }) \
         CREATE (:OutboxEvent { \
             event_id: $outbox_id, aggregate_id: $task_id, type: 'BEDS_RESERVED', \
             payload_json: $payload, created_at: datetime(), published: false, attempts: 0 }) \
         RETURN r.reservation_id AS reservation_id",
    )
    .param("shelter_id", shelter_id)
    .param("beds", beds as i64)
    .param("reservation_id", reservation_id)
    .param("task_id", task_id)
    .param("incident_id", incident_id)
    .param("active_task_key", active_task_key(task_id, ResourceKind::Beds))
    .param("audit_id", new_id())
    .param("outbox_id", new_id())
    .param("actor_id", SYSTEM_ACTOR)
    .param(
        "payload",
        json!({
            "task_id": task_id,
            "incident_id": incident_id,
            "shelter_id": shelter_id,
            "beds": beds,
            "reservation_id": reservation_id,
        })
        .to_string(),
    )
}
