//! M8 acceptance tests: trace context and metrics follow one report from
//! intake to assignment, the simulated extraction engine drives reports
//! through the real pipeline, and the surge cleanup leaves nothing behind.
//! Live Neo4j + Redis (skipped if unreachable).
//!
//! Each test builds its own network in a dataset named `TEST-<uuid>`:
//!
//! ```text
//!   A --ab (60 s)--> B --bd (60 s)--> D
//!   depot S_A at A (no beds), drop-off shelter S_D at D (50 beds)
//! ```
//!
//! The relay step is `Relay::publish` on this test's own outbox event (a
//! test relay pass would publish every unpublished event in the database).

use std::sync::{Arc, OnceLock};

use neo4rs::query;
use opentelemetry::trace::TracerProvider as _;
use serde_json::{json, Value};
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

use crisis_graph::api::{intake_routes, ApiState};
use crisis_graph::config::{Config, PipelineParams};
use crisis_graph::db::{ensure_ops_schema, Neo4jClient, OpsRepository, RedisClient, StateManager, StreamMessage};
use crisis_graph::graph::GraphSnapshot;
use crisis_graph::matcher::Matcher;
use crisis_graph::models::{
    ActorContext, ActorRole, AssetType, HazardCommand, HazardSource, HazardStatus, MedicalCondition, OutboxRecord,
    VictimProfile, WaterLevel, WaterTrend,
};
use crisis_graph::relay::OutboxRelay as Relay;
use crisis_graph::runtime::{telemetry, Role};
use crisis_graph::sim::surge::cleanup_run;
use crisis_graph::triage::simulated::{SimTruth, SimulatedEngine};
use crisis_graph::triage::{TriageOutcome, TriageWorker, TRIAGE_GROUP};

/// One process-wide metrics recorder and tracing subscriber (with trace
/// context, no export) for every test in this binary.
fn metrics() -> &'static metrics_exporter_prometheus::PrometheusHandle {
    static HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        let provider = telemetry::tracer_provider("observability-tests", None).unwrap();
        let subscriber =
            tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(provider.tracer("t")));
        tracing::subscriber::set_global_default(subscriber).unwrap();
        std::mem::forget(provider);
        crisis_graph::runtime::metrics::install(Role::Matcher).unwrap()
    })
}

struct Ctx {
    neo4j: Neo4jClient,
    redis: RedisClient,
    repo: OpsRepository,
    dataset: String,
    params: PipelineParams,
    base: String,
    http: reqwest::Client,
}

async fn setup() -> Option<Ctx> {
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
    metrics();
    ensure_ops_schema(&neo4j).await.expect("ops schema");
    let dataset = format!("TEST-{}", Uuid::new_v4());
    let params = PipelineParams {
        graph_dataset: dataset.clone(),
        extract_stream: format!("{dataset}:extract"),
        events_stream: format!("{dataset}:events"),
        ..PipelineParams::default()
    };
    let repo = OpsRepository::new(neo4j.clone());
    let app = intake_routes(ApiState { repo: repo.clone() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let ctx = Ctx { neo4j, redis, repo, dataset, params, base, http: reqwest::Client::new() };
    ctx.build().await;
    Some(ctx)
}

impl Ctx {
    async fn run(&self, cypher: &str) {
        self.neo4j.graph.run(query(cypher).param("d", self.dataset.clone())).await.unwrap();
    }

    async fn rows(&self, cypher: &str) -> Vec<neo4rs::Row> {
        let mut result = self.neo4j.graph.execute(query(cypher).param("d", self.dataset.clone())).await.unwrap();
        let mut rows = Vec::new();
        while let Some(row) = result.next().await.unwrap() {
            rows.push(row);
        }
        rows
    }

    async fn count(&self, cypher: &str) -> i64 {
        self.rows(cypher).await[0].get::<i64>("n").unwrap()
    }

    async fn build(&self) {
        self.run(
            "UNWIND [['A', 10.00, 76.00], ['B', 10.00, 76.01], ['D', 10.01, 76.01]] AS j \
             CREATE (:Junction {id: $d + '/J' + j[0], dataset: $d, lat: j[1], lon: j[2], \
                 location: point({latitude: j[1], longitude: j[2]})})",
        )
        .await;
        self.run(
            "UNWIND [['ab', 'A', 'B'], ['bd', 'B', 'D']] AS s \
             MATCH (a:Junction {id: $d + '/J' + s[1], dataset: $d}), (b:Junction {id: $d + '/J' + s[2], dataset: $d}) \
             CREATE (a)-[:CONNECTS_TO {segment_id: $d + '/' + s[0], dataset: $d, length_m: 1000.0, \
                 travel_time_s: 60.0, road_class: 'primary', road_name: 'Test road', oneway: true, \
                 emergency: true, motorcar: true, hgv: true, baseline_status: 'OPEN'}]->(b)",
        )
        .await;
        self.run(
            "CREATE (a:Shelter {id: $d + '/S_A', name: 'test depot', dataset: $d, junction_id: $d + '/JA', capacity: 0, current_occupancy: 0}) \
             CREATE (:Shelter {id: $d + '/S_D', name: 'test drop-off', dataset: $d, junction_id: $d + '/JD', capacity: 50, current_occupancy: 0}) \
             CREATE (:Vehicle {vehicle_id: $d + '/AMB-1', asset_type: 'AMBULANCE', org: 'test', capacity_persons: 4, \
                 status: 'AVAILABLE', version: 0, home_shelter_id: a.id, junction_id: $d + '/JA', \
                 created_at: datetime(), updated_at: datetime()})-[:BASED_AT]->(a)",
        )
        .await;
    }

    fn run_id(&self) -> String {
        self.dataset.clone()
    }

    /// Ground truth for a critical report at D.
    fn truth(&self, people: u32) -> SimTruth {
        SimTruth {
            headcount: Some(people),
            asset: Some(AssetType::Ambulance),
            junction: Some(format!("{}/JD", self.dataset)),
            profile: VictimProfile {
                reporter_on_scene: Some(true),
                medical_conditions: vec![MedicalCondition::Unconscious],
                water_level: Some(WaterLevel::Chest),
                water_trend: Some(WaterTrend::Rising),
                ..Default::default()
            },
        }
    }

    /// Post one surge report through the intake API; returns its report ID.
    async fn post_report(&self, n: usize, device: &str, people: u32) -> String {
        let body = json!({
            "request_id": format!("SURGE-{}-{n:06}", self.run_id()),
            "device_id": format!("{}-{device}", self.dataset),
            "raw_text": format!("Father unconscious, water rising, {people} of us {}", self.truth(people).tag()),
            "latitude": 10.01, "longitude": 76.01, "accuracy_m": 20.0,
        });
        let response = self.http.post(format!("{}/api/v1/reports", self.base)).json(&body).send().await.unwrap();
        assert_eq!(response.status(), 202);
        let body: Value = response.json().await.unwrap();
        body["report_id"].as_str().expect("report_id").to_string()
    }

    /// The relay step for this report only: publish its outbox event.
    async fn relay(&self, report_id: &str) {
        let rows = self
            .rows(&format!(
                "MATCH (e:OutboxEvent {{aggregate_id: '{report_id}', type: 'REPORT_RECEIVED'}}) \
                 RETURN e.event_id AS id, e.payload_json AS payload, toString(e.created_at) AS created, e.traceparent AS tp"
            ))
            .await;
        let row = &rows[0];
        let record = OutboxRecord {
            event_id: row.get("id").unwrap(),
            aggregate_id: report_id.to_string(),
            event_type: "REPORT_RECEIVED".into(),
            payload_json: row.get("payload").unwrap(),
            created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<String>("created").unwrap())
                .map(|t| t.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
            traceparent: row.get("tp").unwrap(),
        };
        Relay::new(self.repo.clone(), self.redis.clone(), self.params.clone()).publish(&[record]).await.unwrap();
    }

    fn worker(&self) -> TriageWorker {
        TriageWorker::new(
            self.repo.clone(),
            self.redis.clone(),
            Arc::new(SimulatedEngine::new(5)),
            self.params.clone(),
            1,
            format!("{}-w", self.dataset),
        )
    }

    async fn read_messages(&self) -> Vec<StreamMessage> {
        self.redis.ensure_group_from_start(&self.params.extract_stream, TRIAGE_GROUP).await.unwrap();
        self.redis.read_group(&self.params.extract_stream, TRIAGE_GROUP, "test", 100, 100).await.unwrap()
    }

    async fn match_once(&self) -> usize {
        let snapshot = GraphSnapshot::load(self.repo.clone(), &StateManager::new(self.neo4j.clone()), &self.dataset)
            .await
            .expect("snapshot");
        let matcher = Matcher::new(
            self.repo.clone(),
            self.redis.clone(),
            StateManager::new(self.neo4j.clone()),
            Arc::new(snapshot),
            self.params.clone(),
        );
        let mut lease = matcher.lease();
        let token = lease.try_acquire().await.unwrap().expect("matcher lease");
        matcher.sync_queue().await.unwrap();
        let assigned = matcher.run_cycle(token).await.unwrap().assigned.len();
        lease.release().await.unwrap();
        assigned
    }

    /// Remove the test's network after the run's own cleanup.
    async fn teardown(&self) {
        cleanup_run(&self.neo4j, &self.repo, &self.dataset, &self.run_id()).await.unwrap();
        for statement in [
            "MATCH (c:Command) WHERE c.request_id STARTS WITH $d DELETE c",
            "MATCH (v:Vehicle) WHERE v.vehicle_id STARTS WITH $d \
             OPTIONAL MATCH (v)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE v, a",
            "MATCH (a:AuditEvent) WHERE a.aggregate_id STARTS WITH $d DETACH DELETE a",
            "MATCH (e:OutboxEvent) WHERE e.aggregate_id STARTS WITH $d DELETE e",
            "MATCH (h:Hazard {dataset: $d}) OPTIONAL MATCH (h)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE h, a",
            "MATCH (e:OutboxEvent) WHERE e.type = 'HAZARDS_CHANGED' AND e.payload_json CONTAINS $d DELETE e",
            "MATCH (o:OverlayState {dataset: $d}) DELETE o",
            "MATCH (f:LeaseFence) WHERE f.name = 'matcher:' + $d DELETE f",
            "MATCH (s:Shelter {dataset: $d}) DETACH DELETE s",
            "MATCH (j:Junction {dataset: $d}) DETACH DELETE j",
        ] {
            self.run(statement).await;
        }
        let mut conn = self.redis.manager.clone();
        let _: Result<(), _> = redis::cmd("DEL")
            .arg(&self.params.extract_stream)
            .arg(&self.params.events_stream)
            .arg(format!("{}:dead", self.params.extract_stream))
            .arg(format!("alloc:queue:{}", self.dataset))
            .query_async(&mut conn)
            .await;
    }
}

fn is_traceparent(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    parts.len() == 4 && parts[0] == "00" && parts[1].len() == 32 && parts[2].len() == 16
}

fn trace_id(value: &str) -> &str {
    value.split('-').nth(1).unwrap_or_default()
}

#[tokio::test]
async fn one_report_is_one_trace_from_intake_to_assignment() {
    let Some(ctx) = setup().await else { return };
    let report_id = ctx.post_report(1, "phone", 3).await;

    // Intake stored the trace on the report and its outbox event.
    let rows = ctx
        .rows(&format!(
            "MATCH (r:Report {{report_id: '{report_id}'}}) \
             MATCH (e:OutboxEvent {{aggregate_id: r.report_id, type: 'REPORT_RECEIVED'}}) \
             RETURN r.traceparent AS report, e.traceparent AS event"
        ))
        .await;
    let report_tp: String = rows[0].get("report").expect("report traceparent");
    assert!(is_traceparent(&report_tp), "{report_tp}");
    assert_eq!(rows[0].get::<String>("event").unwrap(), report_tp);

    // The relay copies it into the stream message.
    ctx.relay(&report_id).await;
    let messages = ctx.read_messages().await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].fields.get("traceparent"), Some(&report_tp));

    // Simulated extraction drives the real triage: a critical incident
    // with a queued task, both on the report's trace.
    let outcome = ctx.worker().handle_message(&messages[0]).await.unwrap();
    let TriageOutcome::Created { incident_id, ready: true, .. } = outcome else { panic!("unexpected {outcome:?}") };
    let rows = ctx
        .rows(&format!(
            "MATCH (i:OpsIncident {{incident_id: '{incident_id}'}})-[:HAS_TASK]->(t:Task) \
             MATCH (r:Report {{report_id: '{report_id}'}}) \
             RETURN i.traceparent AS incident, t.traceparent AS task, i.urgency AS urgency, i.headcount AS headcount, \
                    r.extraction_json AS extraction"
        ))
        .await;
    assert_eq!(trace_id(&rows[0].get::<String>("incident").unwrap()), trace_id(&report_tp));
    assert_eq!(trace_id(&rows[0].get::<String>("task").unwrap()), trace_id(&report_tp));
    assert_eq!(rows[0].get::<String>("urgency").unwrap(), "CRITICAL");
    assert_eq!(rows[0].get::<i64>("headcount").unwrap(), 3);
    let extraction: Value = serde_json::from_str(&rows[0].get::<String>("extraction").unwrap()).unwrap();
    assert_eq!(extraction["triage"]["inference"]["backend"], "simulated", "labelled as simulated");

    // The matcher assigns the ambulance and records the M8 histograms.
    assert_eq!(ctx.match_once().await, 1);
    let rendered = metrics().render();
    for name in [
        "crisisgraph_time_to_assignment_seconds_bucket",
        "crisisgraph_queue_wait_seconds_bucket",
        "crisisgraph_matcher_cycle_seconds",
    ] {
        assert!(rendered.contains(name), "{name} missing from /metrics");
    }
    assert!(rendered.contains("tier=\"CRITICAL\""), "time to assignment is labelled by tier");

    ctx.teardown().await;
}

#[tokio::test]
async fn surge_cleanup_leaves_nothing_behind() {
    let Some(ctx) = setup().await else { return };
    // 20 reports: 16 callers and 4 repeat calls from earlier devices (D-2).
    let mut reports = Vec::new();
    for n in 0..20 {
        let device = if n >= 16 { format!("dev-{}", n - 16) } else { format!("dev-{n}") };
        reports.push(ctx.post_report(n, &device, 2).await);
    }
    for report in &reports {
        ctx.relay(report).await;
    }
    let worker = ctx.worker();
    let messages = ctx.read_messages().await;
    assert_eq!(messages.len(), 20);
    for message in &messages {
        worker.handle_message(message).await.unwrap();
    }
    assert_eq!(ctx.match_once().await, 1, "one ambulance, one assignment");
    // The run floods a road, as `surge run --hazards-at` does.
    let actor = ActorContext::new("surge-tool", [ActorRole::HazardVerifier]);
    ctx.repo
        .apply_hazard(
            &actor,
            &HazardCommand {
                request_id: format!("SURGE-{}-HZ-0", ctx.run_id()),
                dataset: ctx.dataset.clone(),
                segment_id: format!("{}/ab", ctx.dataset),
                status: HazardStatus::Flooded,
                valid_until: None,
                reason: "surge test".into(),
                source: HazardSource::Operator,
            },
        )
        .await
        .unwrap();
    let beds_before = ctx.count("MATCH (s:Shelter {id: $d + '/S_D'}) RETURN s.current_occupancy AS n").await;
    assert!(beds_before > 0, "the assignment holds beds at the drop-off");

    let done = cleanup_run(&ctx.neo4j, &ctx.repo, &ctx.dataset, &ctx.run_id()).await.unwrap();
    assert_eq!(done.reports, 20);
    assert_eq!(done.incidents, 16, "repeat calls attached to their device's incident");
    assert_eq!(done.hazards_cleared, vec![format!("{}/ab", ctx.dataset)]);

    let prefix = format!("SURGE-{}-", ctx.run_id());
    let ids: Vec<String> = reports.clone();
    let left = ctx
        .rows(&format!(
            "OPTIONAL MATCH (r:Report) WHERE r.request_id STARTS WITH '{prefix}' WITH count(r) AS reports \
             OPTIONAL MATCH (i:OpsIncident) WHERE EXISTS {{ MATCH (i)-[:LOCATED_AT]->(:Junction {{dataset: $d}}) }} \
             WITH reports, count(i) AS incidents \
             OPTIONAL MATCH (t:Task) WHERE t.target_junction_id STARTS WITH $d OR t.incident_junction_id STARTS WITH $d \
             WITH reports, incidents, count(t) AS tasks \
             OPTIONAL MATCH (res:Reservation) WHERE res.vehicle_id STARTS WITH $d OR res.shelter_id STARTS WITH $d \
             WITH reports, incidents, tasks, count(res) AS reservations \
             OPTIONAL MATCH (h:Hazard {{dataset: $d}}) WHERE h.active_key IS NOT NULL \
             RETURN reports, incidents, tasks, reservations, count(h) AS hazards"
        ))
        .await;
    for field in ["reports", "incidents", "tasks", "reservations", "hazards"] {
        assert_eq!(left[0].get::<i64>(field).unwrap(), 0, "{field} left behind");
    }
    let outbox = ctx
        .neo4j
        .graph
        .execute(query("MATCH (e:OutboxEvent) WHERE e.aggregate_id IN $ids RETURN count(e) AS n").param("ids", ids))
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap()
        .get::<i64>("n")
        .unwrap();
    assert_eq!(outbox, 0, "outbox events of the run removed");
    assert_eq!(ctx.count("MATCH (s:Shelter {id: $d + '/S_D'}) RETURN s.current_occupancy AS n").await, 0, "beds returned");
    assert_eq!(
        ctx.count(
            "MATCH (v:Vehicle {vehicle_id: $d + '/AMB-1'}) \
             RETURN CASE WHEN v.status = 'AVAILABLE' AND v.junction_id = $d + '/JA' THEN 0 ELSE 1 END AS n"
        )
        .await,
        0,
        "vehicle available at its depot"
    );

    ctx.teardown().await;
}
