//! Stream metrics (M8, pipeline doc §4.11): length, pending count and age
//! of the oldest pending message per stream, and the dead-letter count.
//! Sampled every few seconds by the roles that consume or feed a stream.

use std::time::Duration;

use redis::Value;
use tracing::debug;

use crate::db::RedisClient;
use crate::runtime::Shutdown;

const SAMPLE_EVERY: Duration = Duration::from_secs(5);

/// A stream to sample, and its consumer group if pending messages matter.
#[derive(Debug, Clone)]
pub struct SampledStream {
    pub stream: String,
    pub group: Option<String>,
}

/// Sample `streams` (and the dead-letter stream, if any) until shutdown.
pub async fn sample_streams(redis: RedisClient, streams: Vec<SampledStream>, dead_letters: Option<String>, shutdown: Shutdown) {
    while !shutdown.is_triggered() {
        for sampled in &streams {
            if let Err(error) = sample(&redis, sampled).await {
                debug!(%error, stream = %sampled.stream, "Stream sample failed");
            }
        }
        if let Some(dead) = &dead_letters {
            let mut conn = redis.manager.clone();
            if let Ok(length) = redis::cmd("XLEN").arg(dead).query_async::<i64>(&mut conn).await {
                metrics::gauge!("crisisgraph_dead_letters", "stream" => dead.clone()).set(length as f64);
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(SAMPLE_EVERY) => {}
            _ = shutdown.wait() => break,
        }
    }
}

async fn sample(redis: &RedisClient, sampled: &SampledStream) -> redis::RedisResult<()> {
    let mut conn = redis.manager.clone();
    let length: i64 = redis::cmd("XLEN").arg(&sampled.stream).query_async(&mut conn).await?;
    metrics::gauge!("crisisgraph_stream_length", "stream" => sampled.stream.clone()).set(length as f64);
    let Some(group) = &sampled.group else { return Ok(()) };
    // XPENDING summary: [count, smallest id, largest id, consumers].
    let summary: Value = redis::cmd("XPENDING").arg(&sampled.stream).arg(group).query_async(&mut conn).await?;
    let (pending, oldest_id) = parse_pending_summary(&summary);
    metrics::gauge!("crisisgraph_stream_pending", "stream" => sampled.stream.clone(), "group" => group.clone())
        .set(pending as f64);
    let age = oldest_id.and_then(|id| id_age_seconds(&id)).unwrap_or(0.0);
    metrics::gauge!("crisisgraph_stream_oldest_pending_seconds", "stream" => sampled.stream.clone(), "group" => group.clone())
        .set(age);
    Ok(())
}

fn parse_pending_summary(value: &Value) -> (i64, Option<String>) {
    let Value::Array(parts) = value else { return (0, None) };
    let count = match parts.first() {
        Some(Value::Int(n)) => *n,
        _ => 0,
    };
    let oldest = match parts.get(1) {
        Some(Value::BulkString(bytes)) => String::from_utf8(bytes.clone()).ok(),
        Some(Value::SimpleString(text)) => Some(text.clone()),
        _ => None,
    };
    (count, oldest)
}

/// Age of a stream ID (`<ms>-<seq>`) in seconds.
fn id_age_seconds(id: &str) -> Option<f64> {
    let ms: i64 = id.split('-').next()?.parse().ok()?;
    Some(((chrono::Utc::now().timestamp_millis() - ms).max(0)) as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_summary_is_parsed() {
        let summary = Value::Array(vec![
            Value::Int(3),
            Value::BulkString(b"1700000000000-0".to_vec()),
            Value::BulkString(b"1700000000500-0".to_vec()),
            Value::Nil,
        ]);
        assert_eq!(parse_pending_summary(&summary), (3, Some("1700000000000-0".into())));
        assert_eq!(parse_pending_summary(&Value::Array(vec![Value::Int(0), Value::Nil, Value::Nil, Value::Nil])), (0, None));
        assert!(id_age_seconds("1700000000000-0").unwrap() > 0.0);
    }
}
