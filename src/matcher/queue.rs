//! Priority queue `alloc:queue:{dataset}`: a Redis sorted set of queued
//! `task_id`s, scored by [`queue_score`] (tier + aging). Neo4j stays the
//! record: the queue is a copy, synced from the `QUEUED` tasks on becoming
//! leader, on events that change them and periodically, so anything lost in
//! Redis is rebuilt.

use redis::AsyncCommands;

use super::assign::queue_score;
use crate::db::RedisClient;
use crate::models::QueuedTask;

type QueueResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
pub struct AllocQueue {
    redis: RedisClient,
    key: String,
    aging_secs: u64,
}

impl AllocQueue {
    pub fn new(redis: RedisClient, dataset: &str, aging_secs: u64) -> Self {
        Self { redis, key: format!("alloc:queue:{dataset}"), aging_secs }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// Make the queue hold exactly `tasks` (the dataset's `QUEUED` tasks),
    /// atomically: members that are no longer queued are removed, the rest
    /// are added or rescored. Readers never see a half-built queue.
    pub async fn sync(&self, tasks: &[QueuedTask]) -> QueueResult<()> {
        let mut conn = self.redis.manager.clone();
        let mut pipe = redis::pipe();
        pipe.atomic().del(&self.key).ignore();
        if !tasks.is_empty() {
            let members: Vec<(f64, &str)> = tasks
                .iter()
                .map(|t| (queue_score(t.priority_rank, t.queued_at, self.aging_secs), t.task_id.as_str()))
                .collect();
            pipe.zadd_multiple(&self.key, &members).ignore();
        }
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    /// The `limit` task ids served first.
    pub async fn first(&self, limit: usize) -> QueueResult<Vec<String>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut conn = self.redis.manager.clone();
        Ok(conn.zrange(&self.key, 0, limit as isize - 1).await?)
    }

    pub async fn remove(&self, task_id: &str) -> QueueResult<()> {
        let mut conn = self.redis.manager.clone();
        conn.zrem::<_, _, ()>(&self.key, task_id).await?;
        Ok(())
    }

    pub async fn len(&self) -> QueueResult<u64> {
        let mut conn = self.redis.manager.clone();
        Ok(conn.zcard(&self.key).await?)
    }

    pub async fn clear(&self) -> QueueResult<()> {
        let mut conn = self.redis.manager.clone();
        conn.del::<_, ()>(&self.key).await?;
        Ok(())
    }
}
