use redis::aio::ConnectionManager;
use redis::streams::StreamReadReply;
use redis::FromRedisValue;
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

    /// Append a payload to a Redis Stream (XADD)
    pub async fn xadd(&self, stream_key: &str, payload: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut conn = self.manager.clone();
        let id: String = redis::cmd("XADD")
            .arg(stream_key)
            .arg("*")
            .arg("payload")
            .arg(payload)
            .query_async(&mut conn)
            .await?;
        Ok(id)
    }

    /// Ensure that a consumer group exists for a stream (XGROUP CREATE ... MKSTREAM)
    pub async fn ensure_consumer_group(&self, stream_key: &str, group_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut conn = self.manager.clone();
        let res: Result<(), redis::RedisError> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream_key)
            .arg(group_name)
            .arg("$")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;

        match res {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("BUSYGROUP") => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Read incoming messages for a consumer group (XREADGROUP)
    pub async fn read_group_messages(
        &self,
        stream_key: &str,
        group_name: &str,
        consumer_name: &str,
        count: usize,
        block_ms: usize,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error + Send + Sync>> {
        let mut conn = self.manager.clone();
        let reply: Option<StreamReadReply> = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(group_name)
            .arg(consumer_name)
            .arg("BLOCK")
            .arg(block_ms)
            .arg("COUNT")
            .arg(count)
            .arg("STREAMS")
            .arg(stream_key)
            .arg(">")
            .query_async(&mut conn)
            .await?;

        let mut messages = Vec::new();
        if let Some(reply) = reply {
            for key in reply.keys {
                for id_entry in key.ids {
                    if let Some(val) = id_entry.map.get("payload") {
                        if let Ok(payload_str) = String::from_redis_value(val) {
                            messages.push((id_entry.id, payload_str));
                        }
                    }
                }
            }
        }

        Ok(messages)
    }

    /// Acknowledge a processed stream message (XACK)
    pub async fn xack(&self, stream_key: &str, group_name: &str, message_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut conn = self.manager.clone();
        let _: () = redis::cmd("XACK")
            .arg(stream_key)
            .arg(group_name)
            .arg(message_id)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}
