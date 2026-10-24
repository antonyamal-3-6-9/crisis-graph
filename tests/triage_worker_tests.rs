//! M3a acceptance tests: triage workers with minimal deduplication, against
//! live Neo4j and Redis (skipped if unreachable). A scripted engine replaces
//! the LLM so the tests need no inference server; an optional end-to-end test
//! uses the real engine when llama.cpp/vLLM answers on :8000.
//!
//! Rule D-3 flags any open incident nearby, so each test reports from its own
//! location (~1 km from the others) with its own junction ID.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use neo4rs::query;
use uuid::Uuid;

use crisis_graph::config::{Config, PipelineParams};
use crisis_graph::db::{
    ensure_ops_schema, Neo4jClient, OpsRepository, RedisClient, StateManager, StreamMessage,
};
use crisis_graph::ingestion::TriageExtractor;
use crisis_graph::models::{AssetType, NewReport, SosAlert, TriageInferenceMetadata, TriageReport};
use crisis_graph::pipeline::PipelineStages;
use crisis_graph::runtime::{BreakerState, Shutdown};
use crisis_graph::triage::{
    record_from_output, EngineOutput, StageOneEngine, TriageEngine, TriageOutcome, TriageWorker,
    TRIAGE_GROUP,
};

/// Scripted stand-in for the LLM: headcount is the first number in the text;
/// every report resolves to the test's junction and asks for an ambulance.
struct FakeEngine {
    calls: AtomicUsize,
    fail: AtomicBool,
    junction: String,
}

impl FakeEngine {
    fn new(junction: &str) -> Arc<Self> {
        Arc::new(Self { calls: AtomicUsize::new(0), fail: AtomicBool::new(false), junction: junction.into() })
    }

    fn output(&self, text: &str) -> EngineOutput {
        let headcount = text.split_whitespace().find_map(|w| w.parse::<u32>().ok());
        EngineOutput {
            triage: TriageReport {
                victim_location_raw: Some("Test location".into()),
                resolved_junction_id: Some(self.junction.clone()),
                headcount,
                required_asset: Some(AssetType::Ambulance),
                hazards: vec![],
                confidence_score: 0.9,
                needs_human_review: headcount.is_none(),
                uncertainty_reasons: if headcount.is_none() { vec!["Headcount was not provided".into()] } else { vec![] },
                inference: TriageInferenceMetadata {
                    backend: "fake".into(),
                    model: "scripted".into(),
                    adapter_id: None,
                    adapter_sha256: None,
                    prompt_version: "test".into(),
                    schema_version: "triage-extraction-v2".into(),
                    latency_ms: 1,
                    tokens_per_second: None,
                    prompt_eval_ms: None,
                    completion_tokens: None,
                },
            },
            errors: vec![],
        }
    }
}

#[async_trait]
impl TriageEngine for FakeEngine {
    async fn triage(&self, alert: &SosAlert) -> Result<EngineOutput, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err("Triage inference endpoint unavailable (scripted)".into());
        }
        Ok(self.output(&alert.raw_text))
    }
}

struct Ctx {
    neo4j: Neo4jClient,
    redis: RedisClient,
    repo: OpsRepository,
    prefix: String,
    params: PipelineParams,
    lat: f64,
    lon: f64,
}

async fn setup(slot: u32) -> Option<Ctx> {
    let config = Config::from_env();
    let neo4j = Neo4jClient::connect(&config).await.ok()?;
    if neo4j.ping().await.is_err() {
        eprintln!("Neo4j unavailable; skipping");
        return None;
    }
    let redis = match RedisClient::connect(&config).await {
        Ok(redis) if redis.ping().await.is_ok() => redis,
        _ => {
            eprintln!("Redis unavailable; skipping");
            return None;
        }
    };
    ensure_ops_schema(&neo4j).await.expect("ops schema");
    let prefix = format!("TEST-{}", Uuid::new_v4());
    let params = PipelineParams {
        extract_stream: format!("{prefix}:extract"),
        events_stream: format!("{prefix}:events"),
        ..PipelineParams::default()
    };
    Some(Ctx {
        repo: OpsRepository::new(neo4j.clone()),
        neo4j,
        redis,
        params,
        // ~1.1 km apart per slot, well outside the 100 m D-3 radius
        lat: 9.5 + slot as f64 * 0.01,
        lon: 76.0,
        prefix,
    })
}

impl Ctx {
    fn junction(&self) -> String {
        format!("{}-J", self.prefix)
    }

    fn worker(&self, engine: Arc<dyn TriageEngine>) -> TriageWorker {
        TriageWorker::new(self.repo.clone(), self.redis.clone(), engine, self.params.clone(), 2, format!("{}-w", self.prefix))
    }

    async fn report(&self, n: usize, device: &str, text: &str) -> String {
        let (report, _) = self
            .repo
            .create_report(&NewReport {
                request_id: format!("{}-REQ-{n}", self.prefix),
                device_id: format!("{}-{device}", self.prefix),
                channel: "test".into(),
                raw_text: text.into(),
                latitude: Some(self.lat),
                longitude: Some(self.lon),
                accuracy_m: Some(10.0),
            })
            .await
            .unwrap();
        report.report_id
    }

    async fn count(&self, cypher: &str) -> i64 {
        let mut result = self.neo4j.graph.execute(query(cypher).param("p", self.prefix.clone())).await.unwrap();
        result.next().await.unwrap().unwrap().get::<i64>("n").unwrap()
    }

    async fn incidents(&self) -> i64 {
        self.count(
            "MATCH (r:Report)-[:REPORTS]->(i:OpsIncident) WHERE r.request_id STARTS WITH $p \
             RETURN count(DISTINCT i) AS n",
        )
        .await
    }

    async fn cleanup(&self) {
        let statements = [
            "MATCH (r:Report) WHERE r.request_id STARTS WITH $p \
             OPTIONAL MATCH (r)-[:REPORTS]->(i:OpsIncident) \
             OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) \
             OPTIONAL MATCH (i)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) \
             WITH collect(DISTINCT r.report_id) + collect(DISTINCT i.incident_id) + collect(DISTINCT t.task_id) AS ids, \
                  collect(DISTINCT a) AS audits, collect(DISTINCT t) AS tasks, collect(DISTINCT i) AS incidents \
             OPTIONAL MATCH (e:OutboxEvent) WHERE e.aggregate_id IN ids \
             WITH audits, tasks, incidents, collect(e) AS events \
             FOREACH (n IN audits + tasks + incidents + events | DETACH DELETE n)",
            "MATCH (r:Report) WHERE r.request_id STARTS WITH $p DETACH DELETE r",
        ];
        for statement in statements {
            run_cleanup(&self.neo4j, statement, &self.prefix).await;
        }
        let mut conn = self.redis.manager.clone();
        let keys: Vec<String> = redis::cmd("KEYS").arg(format!("{}*", self.prefix)).query_async(&mut conn).await.unwrap_or_default();
        for key in keys {
            let _: Result<(), _> = redis::cmd("DEL").arg(key).query_async(&mut conn).await;
        }
    }
}

/// Run a cleanup statement, retrying transient failures (e.g. a deadlock with
/// a concurrently running test). Panics if it still fails, so leaked test
/// data is reported instead of silently left behind.
async fn run_cleanup(neo4j: &Neo4jClient, statement: &str, prefix: &str) {
    let mut last_error = None;
    for attempt in 1..=5u64 {
        match neo4j.graph.run(query(statement).param("p", prefix)).await {
            Ok(()) => return,
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(std::time::Duration::from_millis(50 * attempt)).await;
            }
        }
    }
    panic!("test cleanup failed for prefix {prefix}: {last_error:?}");
}

/// Acceptance 1: after a crash following extraction, redelivery reuses the
/// saved extraction instead of calling the model again.
#[tokio::test]
async fn saved_extraction_is_reused_after_a_crash() {
    let Some(ctx) = setup(1).await else { return };
    let engine = FakeEngine::new(&ctx.junction());
    let worker = ctx.worker(engine.clone());
    let report_id = ctx.report(1, "D1", "4 people trapped").await;

    // The first worker extracted and saved, then crashed before T1.
    ctx.repo.save_extraction(&report_id, &record_from_output(&engine.output("4 people trapped"))).await.unwrap();

    let outcome = worker.process_report(&report_id).await.unwrap();
    let calls = engine.calls.load(Ordering::SeqCst);
    let incidents = ctx.incidents().await;
    ctx.cleanup().await;

    assert_eq!(calls, 0, "the model must not be called again");
    assert!(matches!(outcome, TriageOutcome::Created { ready: true, .. }), "{outcome:?}");
    assert_eq!(incidents, 1);
}

/// Acceptance 2: duplicate deliveries (sequential or concurrent) never
/// create a second incident.
#[tokio::test]
async fn duplicate_delivery_creates_one_incident() {
    let Some(ctx) = setup(2).await else { return };
    let engine = FakeEngine::new(&ctx.junction());
    let worker = ctx.worker(engine.clone());

    let report_a = ctx.report(1, "D1", "3 people stranded").await;
    let first = worker.process_report(&report_a).await.unwrap();
    let again = worker.process_report(&report_a).await.unwrap();

    let report_b = ctx.report(2, "D2", "2 people stranded").await;
    let (x, y) = tokio::join!(worker.process_report(&report_b), worker.process_report(&report_b));
    let incidents = ctx.incidents().await;
    let tasks = ctx
        .count(
            "MATCH (r:Report)-[:REPORTS]->(:OpsIncident)-[:HAS_TASK]->(t:Task) \
             WHERE r.request_id STARTS WITH $p RETURN count(DISTINCT t) AS n",
        )
        .await;
    ctx.cleanup().await;

    let TriageOutcome::Created { incident_id, .. } = first else { panic!("{first:?}") };
    assert_eq!(again, TriageOutcome::AlreadyLinked { incident_id });
    let created = [x.unwrap(), y.unwrap()]
        .iter()
        .filter(|o| matches!(o, TriageOutcome::Created { .. }))
        .count();
    assert_eq!(created, 1, "concurrent delivery of one report must create one incident");
    assert_eq!(incidents, 2);
    assert_eq!(tasks, 2, "one task per incident");
}

/// Acceptance 3: a follow-up from the same device within 30 min attaches to
/// the open incident and keeps the largest headcount.
#[tokio::test]
async fn same_device_follow_up_is_attached() {
    let Some(ctx) = setup(3).await else { return };
    let worker = ctx.worker(FakeEngine::new(&ctx.junction()));

    let first = ctx.report(1, "PHONE", "3 people on the roof").await;
    let second = ctx.report(2, "PHONE", "now 5 people water rising").await;
    let a = worker.process_report(&first).await.unwrap();
    let b = worker.process_report(&second).await.unwrap();
    let headcount = ctx
        .count(
            "MATCH (r:Report)-[:REPORTS]->(i:OpsIncident) WHERE r.request_id STARTS WITH $p \
             RETURN max(i.headcount) AS n",
        )
        .await;
    let incidents = ctx.incidents().await;
    ctx.cleanup().await;

    let TriageOutcome::Created { incident_id, .. } = a else { panic!("{a:?}") };
    assert!(
        matches!(&b, TriageOutcome::Attached { incident_id: id, headcount_increased: true, .. } if *id == incident_id),
        "{b:?}"
    );
    assert_eq!(incidents, 1);
    assert_eq!(headcount, 5, "largest reported headcount, not the sum");
}

/// Acceptance 4: neighbours (different devices, same place, same moment)
/// get separate incidents that are flagged as possible duplicates.
#[tokio::test]
async fn simultaneous_neighbours_are_flagged_not_merged() {
    let Some(ctx) = setup(4).await else { return };
    let worker = ctx.worker(FakeEngine::new(&ctx.junction()));

    let first = ctx.report(1, "NEIGHBOUR-A", "4 people next door trapped").await;
    let second = ctx.report(2, "NEIGHBOUR-B", "family of 4 trapped next door").await;
    let (a, b) = tokio::join!(worker.process_report(&first), worker.process_report(&second));
    let (a, b) = (a.unwrap(), b.unwrap());
    let incidents = ctx.incidents().await;
    let links = ctx
        .count(
            "MATCH (r:Report)-[:REPORTS]->(i:OpsIncident)-[d:POSSIBLE_DUPLICATE]-(o:OpsIncident)<-[:REPORTS]-(r2:Report) \
             WHERE r.request_id STARTS WITH $p AND r2.request_id STARTS WITH $p \
             RETURN count(DISTINCT d) AS n",
        )
        .await;
    ctx.cleanup().await;

    assert!(matches!(a, TriageOutcome::Created { .. }) && matches!(b, TriageOutcome::Created { .. }));
    assert_eq!(incidents, 2, "different devices are never merged automatically");
    assert_eq!(links, 1, "exactly one possible-duplicate link between them");
}

/// Acceptance 5: repeated model failures open the breaker; reports then go
/// to manual triage without calling the model, and a later success closes it.
#[tokio::test]
async fn breaker_sends_reports_to_manual_triage_and_recovers() {
    let Some(mut ctx) = setup(5).await else { return };
    ctx.params.breaker_half_open_secs = 1;
    let engine = FakeEngine::new(&ctx.junction());
    engine.fail.store(true, Ordering::SeqCst);
    let worker = ctx.worker(engine.clone());

    let mut outcomes = Vec::new();
    for n in 0..6 {
        let report = ctx.report(n, &format!("D{n}"), "2 people need help").await;
        outcomes.push(worker.process_report(&report).await.unwrap());
    }
    let calls_while_failing = engine.calls.load(Ordering::SeqCst);
    let state_while_failing = worker.breaker_state();
    let skipped_reason = ctx
        .count(
            "MATCH (r:Report {request_id: $p + '-REQ-5'}) \
             RETURN size([x IN r.review_reasons WHERE x CONTAINS 'circuit open']) AS n",
        )
        .await;

    engine.fail.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let report = ctx.report(9, "D9", "3 people need help").await;
    let recovered = worker.process_report(&report).await.unwrap();
    let state_after = worker.breaker_state();
    ctx.cleanup().await;

    assert_eq!(calls_while_failing, 5, "the 6th report must not call the model");
    assert_eq!(state_while_failing, BreakerState::Open);
    assert!(outcomes.iter().all(|o| matches!(o, TriageOutcome::Created { ready: false, .. })),
        "every report still becomes an incident, for manual triage: {outcomes:?}");
    assert_eq!(skipped_reason, 1);
    assert!(matches!(recovered, TriageOutcome::Created { ready: true, .. }), "{recovered:?}");
    assert_eq!(state_after, BreakerState::Closed);
}

/// Acceptance 6: poison messages go to the dead-letter stream.
#[tokio::test]
async fn poison_messages_are_dead_lettered() {
    let Some(ctx) = setup(6).await else { return };
    let worker = ctx.worker(FakeEngine::new(&ctx.junction()));

    let missing = StreamMessage {
        id: "1-0".into(),
        fields: [("type", "REPORT_RECEIVED"), ("aggregate_id", "no-such-report"), ("event_id", "e1")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    };
    let a = worker.handle_message(&missing).await.unwrap();

    let report = ctx.report(1, "D1", "2 people").await;
    ctx.neo4j
        .graph
        .run(query("MATCH (r:Report {report_id: $id}) SET r.triage_attempts = 10").param("id", report.clone()))
        .await
        .unwrap();
    let exhausted = StreamMessage {
        id: "2-0".into(),
        fields: [("type", "REPORT_RECEIVED"), ("aggregate_id", report.as_str()), ("event_id", "e2")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    };
    let b = worker.handle_message(&exhausted).await.unwrap();

    let mut conn = ctx.redis.manager.clone();
    let dead: usize = redis::cmd("XLEN").arg(worker.dead_letter_stream()).query_async(&mut conn).await.unwrap();
    ctx.cleanup().await;

    assert!(matches!(a, TriageOutcome::DeadLettered { .. }), "{a:?}");
    assert!(matches!(b, TriageOutcome::DeadLettered { .. }), "{b:?}");
    assert_eq!(dead, 2);
}

/// The worker loop consumes the stream, takes over a crashed consumer's
/// message after the idle timeout, and acknowledges what it processed.
#[tokio::test]
async fn worker_loop_takes_over_crashed_consumers_messages() {
    let Some(mut ctx) = setup(7).await else { return };
    ctx.params.stream_claim_idle_ms = 500;
    let worker = ctx.worker(FakeEngine::new(&ctx.junction()));
    let stream = ctx.params.extract_stream.clone();
    ctx.redis.ensure_group_from_start(&stream, TRIAGE_GROUP).await.unwrap();

    let crashed_report = ctx.report(1, "D1", "2 people").await;
    let live_report = ctx.report(2, "D2", "3 people").await;
    for report in [&crashed_report, &live_report] {
        ctx.redis
            .xadd_fields(&stream, &[("event_id", report), ("type", "REPORT_RECEIVED"), ("aggregate_id", report)])
            .await
            .unwrap();
    }
    // Another consumer claims the first message and dies without acking.
    let claimed = ctx.redis.read_group(&stream, TRIAGE_GROUP, "crashed-consumer", 1, 100).await.unwrap();
    assert_eq!(claimed.len(), 1);

    let shutdown = Shutdown::new();
    let task = tokio::spawn(worker.run(shutdown.clone()));
    let started = Instant::now();
    let mut linked = 0;
    while started.elapsed() < Duration::from_secs(10) {
        linked = ctx
            .count(
                "MATCH (r:Report)-[:REPORTS]->(:OpsIncident) WHERE r.request_id STARTS WITH $p \
                 RETURN count(r) AS n",
            )
            .await;
        if linked == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    shutdown.trigger();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;

    let mut conn = ctx.redis.manager.clone();
    let pending: (i64, Option<String>, Option<String>, Option<Vec<(String, String)>>) = redis::cmd("XPENDING")
        .arg(&stream)
        .arg(TRIAGE_GROUP)
        .query_async(&mut conn)
        .await
        .unwrap();
    ctx.cleanup().await;

    assert_eq!(linked, 2, "both reports processed, including the crashed consumer's");
    assert_eq!(pending.0, 0, "every processed message is acknowledged");
}

/// Optional: the real engine (stage 1 + llama.cpp/vLLM). Skipped when no
/// inference server answers on the configured endpoint.
#[tokio::test]
async fn real_engine_end_to_end_when_inference_is_available() {
    let config = Config::from_env();
    let reachable = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::TcpStream::connect(
            config.inference_base_url.trim_start_matches("http://").split('/').next().unwrap_or("localhost:8000"),
        ),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false);
    if !reachable {
        eprintln!("Inference endpoint not reachable; skipping real-engine test");
        return;
    }
    let Some(ctx) = setup(8).await else { return };
    let stages = PipelineStages::new(
        TriageExtractor::new(&config),
        StateManager::new(ctx.neo4j.clone()),
        ctx.redis.clone(),
    );
    let worker = ctx.worker(Arc::new(StageOneEngine::new(stages)));
    let report = ctx.report(1, "D1", "3 injured people at Aluva Railway Station need an ambulance").await;
    let outcome = worker.process_report(&report).await;
    ctx.cleanup().await;

    let outcome = outcome.unwrap();
    assert!(matches!(outcome, TriageOutcome::Created { .. }), "{outcome:?}");
}
