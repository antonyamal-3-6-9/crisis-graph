use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{info, warn};

use super::{failed_record, record_from_output, TriageEngine};
use crate::config::PipelineParams;
use crate::db::{OpsRepoError, OpsRepository, RedisClient, StreamMessage};
use crate::models::{ExtractionRecord, Report, ReportExtractionStatus, SosAlert, TriageCommit};
use crate::runtime::{CircuitBreaker, Shutdown};

/// Consumer group shared by every triage worker.
pub const TRIAGE_GROUP: &str = "triage-workers";

const READ_BLOCK_MS: usize = 1000;
const CLAIM_BATCH: usize = 10;

/// What processing a message did. Every outcome except a processing error
/// means the message can be acknowledged.
#[derive(Debug, Clone, PartialEq)]
pub enum TriageOutcome {
    Created { incident_id: String, ready: bool, flagged: Vec<String> },
    Attached { incident_id: String, headcount_increased: bool, flagged: Vec<String> },
    AlreadyLinked { incident_id: String },
    DeadLettered { reason: String },
    Ignored,
}

impl TriageOutcome {
    fn label(&self) -> &'static str {
        match self {
            Self::Created { ready: true, .. } => "created",
            Self::Created { ready: false, .. } => "review",
            Self::Attached { .. } => "attached",
            Self::AlreadyLinked { .. } => "duplicate_delivery",
            Self::DeadLettered { .. } => "dead_letter",
            Self::Ignored => "ignored",
        }
    }
}

#[derive(Clone)]
pub struct TriageWorker {
    repo: OpsRepository,
    redis: RedisClient,
    engine: Arc<dyn TriageEngine>,
    breaker: Arc<Mutex<CircuitBreaker>>,
    permits: Arc<Semaphore>,
    params: PipelineParams,
    concurrency: usize,
    consumer: String,
}

impl TriageWorker {
    pub fn new(
        repo: OpsRepository,
        redis: RedisClient,
        engine: Arc<dyn TriageEngine>,
        params: PipelineParams,
        concurrency: usize,
        consumer: impl Into<String>,
    ) -> Self {
        let breaker = CircuitBreaker::new(
            params.breaker_consecutive_failures,
            params.breaker_error_rate,
            Duration::from_secs(params.breaker_window_secs),
            Duration::from_secs(params.breaker_half_open_secs),
        );
        let concurrency = concurrency.max(1);
        Self {
            repo,
            redis,
            engine,
            breaker: Arc::new(Mutex::new(breaker)),
            permits: Arc::new(Semaphore::new(concurrency)),
            params,
            concurrency,
            consumer: consumer.into(),
        }
    }

    pub fn dead_letter_stream(&self) -> String {
        format!("{}:dead", self.params.extract_stream)
    }

    pub fn breaker_state(&self) -> crate::runtime::BreakerState {
        self.breaker.lock().expect("breaker lock").state()
    }

    /// Process one stream message. `Ok` means it may be acknowledged
    /// (including dead-lettered and ignored messages); `Err` means a
    /// processing error, so it is left pending and retried later.
    pub async fn handle_message(&self, message: &StreamMessage) -> Result<TriageOutcome, OpsRepoError> {
        if message.field("type") != Some("REPORT_RECEIVED") {
            return Ok(TriageOutcome::Ignored);
        }
        let Some(report_id) = message.field("aggregate_id") else {
            return self.dead_letter(message, "message has no aggregate_id").await;
        };
        match self.process_report(report_id).await {
            Err(OpsRepoError::NotFound(what)) => {
                self.dead_letter(message, &format!("not found: {what}")).await
            }
            // Too many failed attempts: preserve the message for a human.
            Ok(TriageOutcome::DeadLettered { reason }) => self.dead_letter(message, &reason).await,
            other => other,
        }
    }

    /// Process a report end to end: checkpointed extraction, T1, then D-3.
    /// Safe to repeat: every step recognises work already done.
    pub async fn process_report(&self, report_id: &str) -> Result<TriageOutcome, OpsRepoError> {
        let state = self
            .repo
            .begin_triage(report_id)
            .await?
            .ok_or_else(|| OpsRepoError::NotFound(format!("report {report_id}")))?;

        if let Some(incident_id) = state.linked_incident_id {
            // Finish any flagging a crash may have interrupted.
            self.flag(&incident_id).await?;
            return Ok(TriageOutcome::AlreadyLinked { incident_id });
        }
        if state.attempts > self.params.stream_max_retries {
            return Ok(TriageOutcome::DeadLettered {
                reason: format!("report {report_id} failed {} times", state.attempts - 1),
            });
        }

        if state.report.extraction_status == ReportExtractionStatus::Pending {
            let record = self.extract(&state.report).await;
            self.repo.save_extraction(report_id, &record).await?;
        }

        let commit = self
            .repo
            .commit_triage(report_id, self.params.dedup_same_device_window_secs)
            .await?;
        let flagged = self.flag(commit.incident_id()).await?;
        Ok(match commit {
            TriageCommit::Created { incident_id, task_id } => TriageOutcome::Created {
                incident_id,
                ready: task_id.is_some(),
                flagged,
            },
            TriageCommit::Attached { incident_id, headcount_increased } => TriageOutcome::Attached {
                incident_id,
                headcount_increased,
                flagged,
            },
            TriageCommit::AlreadyLinked { incident_id } => TriageOutcome::AlreadyLinked { incident_id },
        })
    }

    async fn flag(&self, incident_id: &str) -> Result<Vec<String>, OpsRepoError> {
        self.repo
            .flag_possible_duplicates(
                incident_id,
                self.params.dedup_nearby_radius_m,
                self.params.dedup_nearby_window_secs,
            )
            .await
    }

    /// Run the model behind the circuit breaker and the concurrency limit.
    async fn extract(&self, report: &Report) -> ExtractionRecord {
        let allowed = self.breaker.lock().expect("breaker lock").allow(Instant::now());
        self.publish_breaker_state();
        if !allowed {
            return failed_record("Inference unavailable (circuit open); manual triage required");
        }

        let _permit = self.permits.acquire().await.expect("semaphore is never closed");
        let started = Instant::now();
        let result = self.engine.triage(&alert_for(report)).await;
        metrics::histogram!("crisisgraph_triage_extraction_seconds").record(started.elapsed().as_secs_f64());

        let record = {
            let mut breaker = self.breaker.lock().expect("breaker lock");
            match &result {
                Ok(_) => breaker.record_success(Instant::now()),
                Err(_) => breaker.record_failure(Instant::now()),
            }
            match result {
                Ok(output) => record_from_output(&output),
                Err(error) => failed_record(&format!("Triage extraction failed: {error}")),
            }
        };
        self.publish_breaker_state();
        record
    }

    fn publish_breaker_state(&self) {
        let state = self.breaker_state();
        metrics::gauge!("crisisgraph_triage_breaker_state").set(state.as_gauge());
    }

    async fn dead_letter(&self, message: &StreamMessage, reason: &str) -> Result<TriageOutcome, OpsRepoError> {
        let mut fields: Vec<(&str, &str)> =
            message.fields.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        fields.push(("original_id", &message.id));
        fields.push(("error", reason));
        self.redis
            .xadd_fields(&self.dead_letter_stream(), &fields)
            .await
            .map_err(|e| OpsRepoError::Database(e.to_string()))?;
        warn!(message_id = %message.id, reason, "Moved message to dead-letter stream");
        Ok(TriageOutcome::DeadLettered { reason: reason.to_string() })
    }

    /// Handle a message and acknowledge it unless processing failed.
    async fn handle_and_ack(&self, message: StreamMessage) {
        match self.handle_message(&message).await {
            Ok(outcome) => {
                metrics::counter!("crisisgraph_triage_processed_total", "outcome" => outcome.label())
                    .increment(1);
                if let Err(error) = self
                    .redis
                    .xack(&self.params.extract_stream, TRIAGE_GROUP, &message.id)
                    .await
                {
                    warn!(%error, message_id = %message.id, "Failed to acknowledge message");
                }
            }
            Err(error) => {
                metrics::counter!("crisisgraph_triage_processed_total", "outcome" => "error").increment(1);
                warn!(%error, message_id = %message.id, "Triage failed; message will be retried");
            }
        }
    }

    /// Consume the extraction stream until shutdown. Also takes over messages
    /// left unacknowledged by crashed workers after the idle timeout.
    pub async fn run(self, shutdown: Shutdown) {
        let stream = self.params.extract_stream.clone();
        if let Err(error) = self.redis.ensure_group_from_start(&stream, TRIAGE_GROUP).await {
            warn!(%error, "Could not ensure consumer group; continuing");
        }
        info!(consumer = %self.consumer, stream = %stream, "Triage worker started");

        let claim_every = Duration::from_millis((self.params.stream_claim_idle_ms / 2).max(1000));
        let mut last_claim = Instant::now() - claim_every;

        while !shutdown.is_triggered() {
            let mut batch = Vec::new();
            if last_claim.elapsed() >= claim_every {
                match self
                    .redis
                    .autoclaim(&stream, TRIAGE_GROUP, &self.consumer, self.params.stream_claim_idle_ms, CLAIM_BATCH)
                    .await
                {
                    Ok(claimed) => {
                        if !claimed.is_empty() {
                            info!(count = claimed.len(), "Took over stale messages");
                        }
                        batch.extend(claimed);
                    }
                    Err(error) => warn!(%error, "XAUTOCLAIM failed"),
                }
                last_claim = Instant::now();
            }

            let read = tokio::select! {
                read = self.redis.read_group(&stream, TRIAGE_GROUP, &self.consumer, self.concurrency, READ_BLOCK_MS) => read,
                _ = shutdown.wait() => break,
            };
            match read {
                Ok(messages) => batch.extend(messages),
                Err(error) => {
                    warn!(%error, "Stream read failed; backing off");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }

            let mut tasks = JoinSet::new();
            for message in batch {
                let worker = self.clone();
                tasks.spawn(async move { worker.handle_and_ack(message).await });
            }
            while tasks.join_next().await.is_some() {}
        }
        info!(consumer = %self.consumer, "Triage worker stopped");
    }
}

fn alert_for(report: &Report) -> SosAlert {
    let mut alert = SosAlert::new(report.report_id.clone(), report.raw_text.clone());
    alert.timestamp = report.received_at;
    alert.source_channel = Some(report.channel.clone());
    alert.latitude = report.latitude;
    alert.longitude = report.longitude;
    alert.accuracy_m = report.accuracy_m;
    alert
}
