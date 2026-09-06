use redis::aio::ConnectionManager;
use crate::config::Config;

#[derive(Clone)]
pub struct RedisClient {
    pub manager: ConnectionManager,
}

impl RedisClient {
    pub async fn connect(config: &Config) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let client = redis::Client::open(config.redis_url())?;
        let manager = ConnectionManager::new(client).await?;
        Ok(Self { manager })
    }

    /// Attempts to acquire a distributed lock on a resource (e.g. shelter/asset)
    pub async fn acquire_lock(&self, resource_id: &str, lock_token: &str, ttl_secs: u64) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut conn = self.manager.clone();
        let key = format!("lock:crisisgraph:{}", resource_id);
        
        let acquired: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg(lock_token)
            .arg("NX")
            .arg("EX")
            .arg(ttl_secs)
            .query_async(&mut conn)
            .await?;

        Ok(acquired.is_some())
    }

    /// Releases a distributed lock if the token matches
    pub async fn release_lock(&self, resource_id: &str, lock_token: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut conn = self.manager.clone();
        let key = format!("lock:crisisgraph:{}", resource_id);
        
        // Lua script to atomically verify lock ownership before deletion
        let script = r#"
            if redis.call("get", KEYS[1]) == ARGV[1] then
                return redis.call("del", KEYS[1])
            else
                return 0
            end
        "#;
        
        let result: i32 = redis::Script::new(script)
            .key(&key)
            .arg(lock_token)
            .invoke_async(&mut conn)
            .await?;

        Ok(result == 1)
    }

    /// Healthcheck ping
    pub async fn ping(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut conn = self.manager.clone();
        let pong: String = redis::cmd("PING").query_async(&mut conn).await?;
        if pong == "PONG" {
            Ok(())
        } else {
            Err("Unexpected Redis PING response".into())
        }
    }
}
