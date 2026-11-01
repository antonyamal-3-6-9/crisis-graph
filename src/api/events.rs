//! Live operational events for the console (M7): `GET /api/v1/events`
//! (server-sent events).
//!
//! One reader per api process follows the events stream (`ops:events`, fed
//! by the outbox relay) on its own Redis connection and fans each message
//! out to every connected browser. A browser that reconnects sends
//! `Last-Event-ID` (the stream ID of the last event it saw) and first gets
//! what it missed, replayed from the stream, then the live feed.
//!
//! Delivery is at least once. Each event carries its `event_id`; the console
//! drops duplicates and refetches the affected record, ignoring any answer
//! older than the version it already shows.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, Stream, StreamExt};
use tracing::{info, warn};

use crate::db::{RedisClient, StreamMessage};
use crate::runtime::Shutdown;

/// Events a reconnecting browser can get back in one replay.
const MAX_REPLAY: usize = 500;
/// Live events buffered per browser before the slowest falls behind.
const CHANNEL_CAPACITY: usize = 1024;

pub struct EventHub {
    tx: broadcast::Sender<StreamMessage>,
    redis: RedisClient,
    stream: String,
}

impl EventHub {
    pub fn new(redis: RedisClient, stream: impl Into<String>) -> Arc<Self> {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Arc::new(Self { tx, redis, stream: stream.into() })
    }

    pub fn stream(&self) -> &str {
        &self.stream
    }

    /// Follow the stream until shutdown, on `reader` (a dedicated
    /// connection), starting after the newest message present now.
    pub async fn run(self: Arc<Self>, reader: RedisClient, shutdown: Shutdown) {
        let mut last_id = reader.last_stream_id(&self.stream).await.unwrap_or_else(|_| "$".to_string());
        info!(stream = %self.stream, "Event feed started");
        while !shutdown.is_triggered() {
            let read = tokio::select! {
                read = reader.read_after(&self.stream, &last_id, 200, 1000) => read,
                _ = shutdown.wait() => break,
            };
            match read {
                Ok(messages) => {
                    for message in messages {
                        last_id = message.id.clone();
                        // No browser connected is not an error.
                        let _ = self.tx.send(message);
                    }
                }
                Err(error) => {
                    warn!(%error, "Event feed read failed; backing off");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
        info!(stream = %self.stream, "Event feed stopped");
    }

    /// The SSE stream for one browser: replay after `last_event_id`, then live.
    pub async fn subscribe(
        &self,
        last_event_id: Option<String>,
    ) -> Result<impl Stream<Item = StreamMessage>, Box<dyn std::error::Error + Send + Sync>> {
        // Subscribe before replaying, so nothing falls between the two.
        let live = BroadcastStream::new(self.tx.subscribe());
        let replay = match last_event_id.as_deref().filter(|id| parse_id(id).is_some()) {
            Some(id) => self.redis.range_after(&self.stream, id, MAX_REPLAY).await?,
            None => Vec::new(),
        };
        let floor = replay
            .last()
            .map(|m| m.id.clone())
            .or(last_event_id)
            .and_then(|id| parse_id(&id));
        let live = live.filter_map(move |item| match item {
            Ok(message) if floor.is_none_or(|f| parse_id(&message.id).is_some_and(|id| id > f)) => Some(message),
            // Already replayed, or this browser fell behind (it refetches).
            _ => None,
        });
        Ok(tokio_stream::iter(replay).chain(live))
    }
}

/// `"<ms>-<seq>"` as a comparable pair.
fn parse_id(id: &str) -> Option<(u64, u64)> {
    let (ms, seq) = id.split_once('-')?;
    Some((ms.parse().ok()?, seq.parse().ok()?))
}

/// The browser-facing form of a stream message.
pub fn event_body(message: &StreamMessage) -> Value {
    let payload = message
        .field("payload")
        .map(|p| serde_json::from_str::<Value>(p).unwrap_or_else(|_| json!(p)))
        .unwrap_or(Value::Null);
    json!({
        "stream_id": message.id,
        "event_id": message.field("event_id"),
        "type": message.field("type"),
        "aggregate_id": message.field("aggregate_id"),
        "created_at": message.field("created_at"),
        "payload": payload,
    })
}

pub fn event_routes(hub: Arc<EventHub>) -> Router {
    Router::new().route("/api/v1/events", get(events)).with_state(hub)
}

async fn events(
    State(hub): State<Arc<EventHub>>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, axum::http::StatusCode> {
    let last = headers.get("last-event-id").and_then(|v| v.to_str().ok()).map(str::to_string);
    let stream = hub.subscribe(last).await.map_err(|error| {
        warn!(%error, "Event replay failed");
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    })?;
    let stream = stream.map(|message| {
        let kind = message.field("type").unwrap_or("message").to_string();
        Ok(Event::default().id(message.id.clone()).event(kind).data(event_body(&message).to_string()))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::parse_id;

    #[test]
    fn stream_ids_compare_numerically() {
        assert!(parse_id("1700000000000-10") > parse_id("1700000000000-9"));
        assert!(parse_id("1700000000001-0") > parse_id("1700000000000-99"));
        assert_eq!(parse_id("bogus"), None);
    }
}
