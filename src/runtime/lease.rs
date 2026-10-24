//! Leader lease in Redis with a fencing token.
//!
//! Only one process at a time may run a leader-only job (outbox relay,
//! matcher). The lease is a Redis key holding the owner's ID with an expiry;
//! the owner renews it well before it expires. Every successful acquisition
//! also takes the next value of a counter: the **fencing token**. Leader-only
//! writes carry the token, and the database rejects a token older than one it
//! has already seen, so a leader that was paused (GC, network) and lost its
//! lease cannot write after a newer leader has taken over.

use redis::Script;

use crate::db::RedisClient;

type LeaseResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Extend the lease only if we still own it.
const RENEW_SCRIPT: &str = r#"
if redis.call('get', KEYS[1]) == ARGV[1] then
    return redis.call('pexpire', KEYS[1], ARGV[2])
else
    return 0
end
"#;

/// Delete the lease only if we still own it.
const RELEASE_SCRIPT: &str = r#"
if redis.call('get', KEYS[1]) == ARGV[1] then
    return redis.call('del', KEYS[1])
else
    return 0
end
"#;

pub struct Lease {
    redis: RedisClient,
    name: String,
    owner_id: String,
    ttl_ms: u64,
    token: Option<u64>,
}

impl Lease {
    pub fn new(redis: RedisClient, name: impl Into<String>, ttl_ms: u64) -> Self {
        Self {
            redis,
            name: name.into(),
            owner_id: uuid::Uuid::new_v4().to_string(),
            ttl_ms,
            token: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The fencing token of the current tenure, if this process is leader.
    pub fn token(&self) -> Option<u64> {
        self.token
    }

    fn key(&self) -> String {
        format!("lease:{}", self.name)
    }

    fn fence_key(&self) -> String {
        format!("lease:{}:fence", self.name)
    }

    /// Try to become leader. Returns the new fencing token on success, or
    /// `None` if another process holds the lease.
    pub async fn try_acquire(&mut self) -> LeaseResult<Option<u64>> {
        let mut conn = self.redis.manager.clone();
        let acquired: Option<String> = redis::cmd("SET")
            .arg(self.key())
            .arg(&self.owner_id)
            .arg("NX")
            .arg("PX")
            .arg(self.ttl_ms)
            .query_async(&mut conn)
            .await?;
        if acquired.is_none() {
            self.token = None;
            return Ok(None);
        }
        let token: u64 = redis::cmd("INCR")
            .arg(self.fence_key())
            .query_async(&mut conn)
            .await?;
        self.token = Some(token);
        Ok(Some(token))
    }

    /// Extend the lease. Returns false (and drops leadership) if it was lost,
    /// e.g. because it expired and another process acquired it.
    pub async fn renew(&mut self) -> LeaseResult<bool> {
        let mut conn = self.redis.manager.clone();
        let renewed: i64 = Script::new(RENEW_SCRIPT)
            .key(self.key())
            .arg(&self.owner_id)
            .arg(self.ttl_ms)
            .invoke_async(&mut conn)
            .await?;
        if renewed != 1 {
            self.token = None;
        }
        Ok(renewed == 1)
    }

    /// Give up the lease (on shutdown) so a standby can take over at once.
    pub async fn release(&mut self) -> LeaseResult<()> {
        let mut conn = self.redis.manager.clone();
        let _: i64 = Script::new(RELEASE_SCRIPT)
            .key(self.key())
            .arg(&self.owner_id)
            .invoke_async(&mut conn)
            .await?;
        self.token = None;
        Ok(())
    }
}
