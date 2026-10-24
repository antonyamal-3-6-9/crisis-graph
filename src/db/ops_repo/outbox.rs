use neo4rs::query;

use super::{parse_time, OpsRepoError, OpsRepository};
use crate::models::OutboxRecord;

impl OpsRepository {
    /// Oldest unpublished outbox events first (for the outbox relay).
    pub async fn fetch_unpublished(&self, limit: usize) -> Result<Vec<OutboxRecord>, OpsRepoError> {
        let q = query(
            "MATCH (e:OutboxEvent {published: false}) \
             RETURN e.event_id AS event_id, e.aggregate_id AS aggregate_id, e.type AS event_type, \
                    e.payload_json AS payload_json, toString(e.created_at) AS created_at \
             ORDER BY e.created_at ASC LIMIT $limit",
        )
        .param("limit", limit as i64);

        let mut result = self.neo4j.graph.execute(q).await?;
        let mut events = Vec::new();
        while let Some(row) = result.next().await? {
            events.push(OutboxRecord {
                event_id: row.get("event_id")?,
                aggregate_id: row.get("aggregate_id")?,
                event_type: row.get("event_type")?,
                payload_json: row.get("payload_json")?,
                created_at: parse_time(&row.get::<String>("created_at")?)?,
            });
        }
        Ok(events)
    }

    /// Mark events as published on behalf of the current lease holder.
    ///
    /// Fencing: `(:LeaseFence {name})` stores the highest token that has
    /// written. The write is accepted only if `token` is at least that high,
    /// and records it, so a former leader that resumes after losing its lease
    /// (with an older token) is rejected with `Fenced` instead of writing.
    pub async fn mark_published_fenced(
        &self,
        event_ids: &[String],
        fence_name: &str,
        token: u64,
    ) -> Result<u64, OpsRepoError> {
        let q = query(
            "MERGE (f:LeaseFence {name: $fence}) \
             ON CREATE SET f.token = $token \
             SET f._lock = true \
             REMOVE f._lock \
             WITH f \
             WHERE f.token <= $token \
             SET f.token = $token, f.updated_at = datetime() \
             WITH f \
             OPTIONAL MATCH (e:OutboxEvent) WHERE e.event_id IN $ids AND e.published = false \
             SET e.published = true, e.published_at = datetime(), e.attempts = e.attempts + 1 \
             RETURN f.token AS fence, count(e) AS marked",
        )
        .param("fence", fence_name)
        .param("token", token as i64)
        .param("ids", event_ids.to_vec());
        match self.write_first_row(q).await? {
            Some(row) => Ok(row.get::<i64>("marked")? as u64),
            None => Err(OpsRepoError::Fenced(format!(
                "lease '{fence_name}' token {token} is older than the current holder's"
            ))),
        }
    }

    /// Delete published events older than `retention_days`. Returns how many
    /// were removed. Unpublished events are never deleted.
    pub async fn purge_published(&self, retention_days: u64) -> Result<u64, OpsRepoError> {
        let q = query(
            "MATCH (e:OutboxEvent {published: true}) \
             WHERE e.published_at < datetime() - duration({days: $days}) \
             WITH e LIMIT 10000 \
             DETACH DELETE e \
             RETURN count(*) AS purged",
        )
        .param("days", retention_days as i64);
        Ok(match self.write_first_row(q).await? {
            Some(row) => row.get::<i64>("purged")? as u64,
            None => 0,
        })
    }

    /// Number of unpublished events and the age in seconds of the oldest.
    pub async fn outbox_backlog(&self) -> Result<(u64, f64), OpsRepoError> {
        let q = query(
            "MATCH (e:OutboxEvent {published: false}) \
             RETURN count(e) AS pending, \
                    coalesce(duration.inSeconds(min(e.created_at), datetime()).seconds, 0) AS oldest_secs",
        );
        let mut result = self.neo4j.graph.execute(q).await?;
        Ok(match result.next().await? {
            Some(row) => (
                row.get::<i64>("pending")? as u64,
                row.get::<i64>("oldest_secs")? as f64,
            ),
            None => (0, 0.0),
        })
    }

    /// Mark events as published. Already-published events are left alone, so
    /// a relay that crashed and repeats this call does no harm.
    pub async fn mark_published(&self, event_ids: &[String]) -> Result<u64, OpsRepoError> {
        if event_ids.is_empty() {
            return Ok(0);
        }
        let q = query(
            "UNWIND $ids AS id \
             MATCH (e:OutboxEvent {event_id: id, published: false}) \
             SET e.published = true, e.published_at = datetime(), e.attempts = e.attempts + 1 \
             RETURN count(e) AS marked",
        )
        .param("ids", event_ids.to_vec());
        Ok(match self.write_first_row(q).await? {
            Some(row) => row.get::<i64>("marked")? as u64,
            None => 0,
        })
    }
}
