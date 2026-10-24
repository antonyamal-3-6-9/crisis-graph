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
