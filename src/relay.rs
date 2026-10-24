//! Outbox relay: publishes committed outbox events to Redis Streams.
//!
//! The database transaction that changes state also writes an
//! `:OutboxEvent`; this relay publishes those events after commit and marks
//! them published. Only the lease holder publishes. Delivery is at least
//! once: a crash between publishing and marking publishes the event again,
//! with the same `event_id`, and consumers discard the duplicate.

use std::time::{Duration, Instant};

use tracing::{info, warn};

use crate::config::PipelineParams;
use crate::db::{OpsRepoError, OpsRepository, RedisClient};
use crate::models::OutboxRecord;
use crate::runtime::{Lease, Shutdown};

/// Lease and fence name shared by every relay process.
pub const RELAY_LEASE: &str = "outbox-relay";

const PURGE_INTERVAL: Duration = Duration::from_secs(3600);
const BACKLOG_INTERVAL: Duration = Duration::from_secs(5);

type RelayResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
pub struct OutboxRelay {
    repo: OpsRepository,
    redis: RedisClient,
    params: PipelineParams,
    lease_name: String,
}

/// Result of one relay pass.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RelayPass {
    pub published: usize,
    pub marked: u64,
}

impl OutboxRelay {
    pub fn new(repo: OpsRepository, redis: RedisClient, params: PipelineParams) -> Self {
        Self::with_lease_name(repo, redis, params, RELAY_LEASE)
    }

    /// Use a different lease/fence name (tests run isolated relays).
    pub fn with_lease_name(
        repo: OpsRepository,
        redis: RedisClient,
        params: PipelineParams,
        lease_name: impl Into<String>,
    ) -> Self {
        Self {
            repo,
            redis,
            params,
            lease_name: lease_name.into(),
        }
    }

    pub fn lease(&self) -> Lease {
        Lease::new(self.redis.clone(), self.lease_name.clone(), self.params.lease_ttl_ms)
    }

    /// The stream an event type is published to.
    pub fn stream_for(&self, event_type: &str) -> &str {
        match event_type {
            "REPORT_RECEIVED" => &self.params.extract_stream,
            _ => &self.params.events_stream,
        }
    }

    /// Publish events to their streams, in order. Does not mark them.
    pub async fn publish(&self, events: &[OutboxRecord]) -> RelayResult<Vec<String>> {
        let mut published = Vec::with_capacity(events.len());
        for event in events {
            self.redis
                .xadd_fields(
                    self.stream_for(&event.event_type),
                    &[
                        ("event_id", &event.event_id),
                        ("type", &event.event_type),
                        ("aggregate_id", &event.aggregate_id),
                        ("payload", &event.payload_json),
                        ("created_at", &event.created_at.to_rfc3339()),
                    ],
                )
                .await?;
            published.push(event.event_id.clone());
        }
        Ok(published)
    }

    /// One pass as leader with fencing token `token`: fetch the oldest
    /// unpublished events, publish them, then mark them published. Returns
    /// `OpsRepoError::Fenced` (boxed) if a newer leader exists.
    pub async fn run_once(&self, token: u64) -> RelayResult<RelayPass> {
        let events = self.repo.fetch_unpublished(self.params.outbox_batch_size).await?;
        if events.is_empty() {
            return Ok(RelayPass::default());
        }
        let published = self.publish(&events).await?;
        let marked = self
            .repo
            .mark_published_fenced(&published, &self.lease_name, token)
            .await?;
        metrics::counter!("crisisgraph_relay_published_total").increment(published.len() as u64);
        Ok(RelayPass {
            published: published.len(),
            marked,
        })
    }

    /// Run until shutdown: acquire or renew the lease, and while leader
    /// publish batches every poll interval. Releases the lease on shutdown.
    pub async fn run(self, shutdown: Shutdown) {
        let mut lease = self.lease();
        let poll = Duration::from_millis(self.params.outbox_poll_ms);
        let renew_every = Duration::from_millis(self.params.lease_renew_ms);
        let mut last_renew = Instant::now();
        let mut last_purge = Instant::now();
        let mut last_backlog = Instant::now() - BACKLOG_INTERVAL;

        loop {
            if shutdown.is_triggered() {
                break;
            }

            match lease.token() {
                None => match lease.try_acquire().await {
                    Ok(Some(token)) => {
                        info!(lease = lease.name(), token, "Outbox relay became leader");
                        last_renew = Instant::now();
                    }
                    Ok(None) => {}
                    Err(error) => warn!(%error, "Lease acquire failed"),
                },
                Some(_) if last_renew.elapsed() >= renew_every => match lease.renew().await {
                    Ok(true) => last_renew = Instant::now(),
                    Ok(false) => {
                        warn!(lease = lease.name(), "Outbox relay lost its lease");
                        metrics::counter!("crisisgraph_lease_lost_total", "lease" => self.lease_name.clone())
                            .increment(1);
                    }
                    Err(error) => warn!(%error, "Lease renew failed"),
                },
                Some(_) => {}
            }

            let is_leader = lease.token().is_some();
            metrics::gauge!("crisisgraph_relay_is_leader").set(if is_leader { 1.0 } else { 0.0 });

            if let Some(token) = lease.token() {
                match self.run_once(token).await {
                    Ok(_) => {}
                    Err(error) => {
                        if matches!(error.downcast_ref::<OpsRepoError>(), Some(OpsRepoError::Fenced(_))) {
                            warn!(lease = lease.name(), token, "Fenced by a newer leader; stepping down");
                            let _ = lease.release().await;
                        } else {
                            warn!(%error, "Relay pass failed; will retry");
                        }
                    }
                }

                if last_purge.elapsed() >= PURGE_INTERVAL {
                    match self.repo.purge_published(self.params.outbox_retention_days).await {
                        Ok(purged) if purged > 0 => info!(purged, "Purged old published outbox events"),
                        Ok(_) => {}
                        Err(error) => warn!(%error, "Outbox purge failed"),
                    }
                    last_purge = Instant::now();
                }
            }

            if last_backlog.elapsed() >= BACKLOG_INTERVAL {
                if let Ok((pending, oldest_secs)) = self.repo.outbox_backlog().await {
                    metrics::gauge!("crisisgraph_outbox_unpublished").set(pending as f64);
                    metrics::gauge!("crisisgraph_outbox_oldest_unpublished_seconds").set(oldest_secs);
                }
                last_backlog = Instant::now();
            }

            tokio::select! {
                _ = tokio::time::sleep(poll) => {}
                _ = shutdown.wait() => break,
            }
        }

        if lease.token().is_some() {
            let _ = lease.release().await;
            info!(lease = lease.name(), "Outbox relay released its lease");
        }
        metrics::gauge!("crisisgraph_relay_is_leader").set(0.0);
    }
}
