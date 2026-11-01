//! M7 acceptance tests: the console API (reads, review and task-complete
//! commands, submit-and-wait dispatch) and the live event feed. Live Neo4j +
//! Redis (skipped if unreachable).
//!
//! Each test builds its own network in a dataset named `TEST-<uuid>`:
//!
//! ```text
//!   A --ab (60 s)--> B --bd (60 s)--> D      fast
//!   A --ac (300 s)-> C --cd (300 s)-> D      slow detour
//!   depot S_A at A (no beds), drop-off shelter S_D at D
//! ```
//!
//! Triage and matching run in-process (scripted extraction, `run_cycle`),
//! never through the outbox relay: a test relay would publish every
//! unpublished event in the database, not only this test's.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use neo4rs::query;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio_stream::StreamExt;
use uuid::Uuid;

use crisis_graph::api::{console_routes, event_body, intake_routes, token_hash_hex, ApiState, ConsoleApiState, EventHub, OperatorTokens};
use crisis_graph::config::{Config, PipelineParams};
use crisis_graph::db::{ensure_ops_schema, Neo4jClient, OpsRepository, RedisClient, StateManager};
use crisis_graph::graph::GraphSnapshot;
use crisis_graph::matcher::Matcher;
use crisis_graph::models::{
    derive_urgency, AssetType, LocationSource, SosAlert, TriageInferenceMetadata, TriageReport, VictimProfile,
};
use crisis_graph::runtime::Shutdown;
use crisis_graph::triage::{EngineOutput, TriageEngine, TriageWorker};

const DISPATCHER: &str = "dispatcher-token";
const OBSERVER: &str = "observer-token";

/// Scripted extraction: the victims are at `junction`; a number in the text
/// is the headcount, and without one the report needs review.
struct FakeEngine {
    junction: String,
    calls: AtomicUsize,
}

#[async_trait]
impl TriageEngine for FakeEngine {
    async fn triage(&self, alert: &SosAlert) -> Result<EngineOutput, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let headcount = alert.raw_text.split_whitespace().find_map(|w| w.parse::<u32>().ok());
        let profile = VictimProfile { reporter_on_scene: Some(true), ..Default::default() };
        let urgency = derive_urgency(&profile);
        Ok(EngineOutput {
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
                    schema_version: "triage-extraction-v3".into(),
                    latency_ms: 1,
                    tokens_per_second: None,
                    prompt_eval_ms: None,
                    completion_tokens: None,
                },
                victim_profile: Some(profile),
                urgency: Some(urgency),
                location_source: Some(LocationSource::DeviceGps),
            },
            errors: vec![],
        })
    }
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
    ensure_ops_schema(&neo4j).await.expect("ops schema");
    let dataset = format!("TEST-{}", Uuid::new_v4());
    let params = PipelineParams {
        graph_dataset: dataset.clone(),
        extract_stream: format!("{dataset}:extract"),
        events_stream: format!("{dataset}:events"),
        dispatch_wait_secs: 20,
        ..PipelineParams::default()
    };
    let repo = OpsRepository::new(neo4j.clone());
    let spec = format!(
        "dispatcher:DISPATCHER:{},observer:OBSERVER:{}",
        token_hash_hex(DISPATCHER),
        token_hash_hex(OBSERVER)
    );
    let (operators, problems) = OperatorTokens::parse(&spec);
    assert!(problems.is_empty(), "{problems:?}");
    let app = intake_routes(ApiState { repo: repo.clone() }).merge(console_routes(ConsoleApiState {
        repo: repo.clone(),
        state: StateManager::new(neo4j.clone()),
        redis: redis.clone(),
        operators: Arc::new(operators),
        params: params.clone(),
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let ctx = Ctx { neo4j, redis, repo, dataset, params, base, http: reqwest::Client::new() };
    ctx.build().await;
    Some(ctx)
}

impl Ctx {
    fn id(&self, name: &str) -> String {
        format!("{}/{name}", self.dataset)
    }

    async fn run(&self, cypher: &str, params: &[(&str, neo4rs::BoltType)]) {
        let mut q = query(cypher).param("d", self.dataset.clone());
        for (key, value) in params {
            q = q.param(key, value.clone());
        }
        self.neo4j.graph.run(q).await.unwrap();
    }

    async fn count(&self, cypher: &str) -> i64 {
        let mut result = self.neo4j.graph.execute(query(cypher).param("d", self.dataset.clone())).await.unwrap();
        result.next().await.unwrap().unwrap().get::<i64>("n").unwrap()
    }

    async fn build(&self) {
        for (name, lat, lon) in [("A", 10.00, 76.00), ("B", 10.00, 76.01), ("C", 10.01, 76.00), ("D", 10.01, 76.01)] {
            self.run(
                "CREATE (:Junction {id: $d + '/J' + $name, dataset: $d, lat: $lat, lon: $lon, \
                     location: point({latitude: $lat, longitude: $lon})})",
                &[("name", name.into()), ("lat", lat.into()), ("lon", lon.into())],
            )
            .await;
        }
        for (name, from, to, seconds) in [("ab", "A", "B", 60.0), ("bd", "B", "D", 60.0), ("ac", "A", "C", 300.0), ("cd", "C", "D", 300.0)] {
            self.run(
                "MATCH (a:Junction {id: $d + '/J' + $from, dataset: $d}), (b:Junction {id: $d + '/J' + $to, dataset: $d}) \
                 CREATE (a)-[:CONNECTS_TO {segment_id: $d + '/' + $name, dataset: $d, length_m: 1000.0, \
                     travel_time_s: $seconds, road_class: 'primary', road_name: 'Test road ' + $name, oneway: true, \
                     emergency: true, motorcar: true, hgv: true, baseline_status: 'OPEN'}]->(b)",
                &[("name", name.into()), ("from", from.into()), ("to", to.into()), ("seconds", seconds.into())],
            )
            .await;
        }
        self.run(
            "CREATE (:Shelter {id: $d + '/S_A', name: 'test depot', dataset: $d, junction_id: $d + '/JA', capacity: 0, current_occupancy: 0}) \
             CREATE (:Shelter {id: $d + '/S_D', name: 'test drop-off', dataset: $d, junction_id: $d + '/JD', capacity: 50, current_occupancy: 0})",
            &[],
        )
        .await;
        self.run(
            "MATCH (s:Shelter {id: $d + '/S_A'}) \
             CREATE (v:Vehicle {vehicle_id: $d + '/AMB-1', asset_type: 'AMBULANCE', org: 'test', capacity_persons: 4, \
                 status: 'AVAILABLE', version: 0, home_shelter_id: s.id, junction_id: $d + '/JA', \
                 created_at: datetime(), updated_at: datetime()})-[:BASED_AT]->(s)",
            &[],
        )
        .await;
    }

    /// A `REVIEW_REQUIRED` incident with no location and no task.
    async fn review_incident(&self, name: &str) -> String {
        let id = self.id(&format!("I-{name}"));
        self.run(
            "CREATE (:OpsIncident {incident_id: $id, status: 'REVIEW_REQUIRED', version: 0, urgency: 'UNKNOWN', \
                 priority_rank: 1, callback_required: true, review_priority: 'HIGH', review_rank: 2, \
                 review_reasons: ['Triage extraction failed'], created_at: datetime(), updated_at: datetime()})",
            &[("id", id.clone().into())],
        )
        .await;
        id
    }

    async fn matcher(&self) -> Matcher {
        let snapshot = GraphSnapshot::load(self.repo.clone(), &StateManager::new(self.neo4j.clone()), &self.dataset)
            .await
            .expect("snapshot");
        Matcher::new(
            self.repo.clone(),
            self.redis.clone(),
            StateManager::new(self.neo4j.clone()),
            Arc::new(snapshot),
            self.params.clone(),
        )
    }

    /// One matching cycle as leader.
    async fn match_once(&self, matcher: &Matcher) -> usize {
        let mut lease = matcher.lease();
        let token = match lease.try_acquire().await.unwrap() {
            Some(token) => token,
            None => panic!("matcher lease busy"),
        };
        matcher.sync_queue().await.unwrap();
        let assigned = matcher.run_cycle(token).await.unwrap().assigned.len();
        lease.release().await.unwrap();
        assigned
    }

    fn worker(&self) -> TriageWorker {
        let engine = Arc::new(FakeEngine { junction: self.id("JD"), calls: AtomicUsize::new(0) });
        TriageWorker::new(self.repo.clone(), self.redis.clone(), engine, self.params.clone(), 1, format!("{}-w", self.dataset))
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let response = self.http.get(format!("{}{path}", self.base)).send().await.unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn post(&self, path: &str, token: Option<&str>, body: Value) -> (StatusCode, Value, bool) {
        let mut request = self.http.post(format!("{}{path}", self.base)).json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let replayed = response.headers().get("idempotent-replayed").is_some();
        (status, response.json().await.unwrap_or(Value::Null), replayed)
    }

    async fn incident(&self, id: &str) -> Value {
        let (status, body) = self.get(&format!("/api/v1/incidents/{}", urlencoding(id))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn ids(&self, cypher: &str) -> Vec<String> {
        let mut result = self.neo4j.graph.execute(query(cypher).param("d", self.dataset.clone())).await.unwrap();
        let mut ids = Vec::new();
        while let Some(row) = result.next().await.unwrap() {
            ids.push(row.get::<String>("id").unwrap());
        }
        ids
    }

    async fn cleanup(&self) {
        // Triage gives incidents and tasks plain UUIDs, so find this test's
        // records through its dataset, reports and commands.
        let incidents = self
            .ids(
                "MATCH (i:OpsIncident) WHERE i.incident_id STARTS WITH $d \
                    OR EXISTS { MATCH (i)-[:LOCATED_AT]->(:Junction {dataset: $d}) } \
                    OR EXISTS { MATCH (r:Report)-[:REPORTS]->(i) WHERE r.request_id STARTS WITH $d } \
                 RETURN i.incident_id AS id",
            )
            .await;
        let reports = self.ids("MATCH (r:Report) WHERE r.request_id STARTS WITH $d RETURN r.report_id AS id").await;
        let mut ids = incidents.clone();
        ids.extend(reports);
        let tasks = {
            let mut result = self
                .neo4j
                .graph
                .execute(query("MATCH (t:Task) WHERE t.incident_id IN $ids RETURN t.task_id AS id").param("ids", incidents.clone()))
                .await
                .unwrap();
            let mut tasks = Vec::new();
            while let Some(row) = result.next().await.unwrap() {
                tasks.push(row.get::<String>("id").unwrap());
            }
            tasks
        };
        ids.extend(tasks);
        let by_ids = [
            "MATCH (r:Reservation) WHERE r.incident_id IN $ids OR r.task_id IN $ids \
             OPTIONAL MATCH (r)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE r, a",
            "MATCH (a:AuditEvent) WHERE a.aggregate_id IN $ids DETACH DELETE a",
            "MATCH (e:OutboxEvent) WHERE e.aggregate_id IN $ids DELETE e",
            "MATCH (t:Task) WHERE t.task_id IN $ids DETACH DELETE t",
            "MATCH (i:OpsIncident) WHERE i.incident_id IN $ids DETACH DELETE i",
            "MATCH (r:Report) WHERE r.report_id IN $ids DETACH DELETE r",
        ];
        for statement in by_ids {
            self.neo4j.graph.run(query(statement).param("ids", ids.clone())).await.unwrap();
        }
        let statements = [
            "MATCH (c:Command) WHERE c.request_id STARTS WITH $d DELETE c",
            "MATCH (v:Vehicle) WHERE v.vehicle_id STARTS WITH $d \
             OPTIONAL MATCH (v)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE v, a",
            "MATCH (a:AuditEvent) WHERE a.aggregate_id STARTS WITH $d DETACH DELETE a",
            "MATCH (e:OutboxEvent) WHERE e.aggregate_id STARTS WITH $d DELETE e",
            "MATCH (h:Hazard {dataset: $d}) OPTIONAL MATCH (h)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE h, a",
            "MATCH (o:OverlayState {dataset: $d}) DELETE o",
            "MATCH (f:LeaseFence) WHERE f.name = 'matcher:' + $d DELETE f",
            "MATCH (s:Shelter {dataset: $d}) DETACH DELETE s",
            "MATCH (j:Junction {dataset: $d}) DETACH DELETE j",
        ];
        for statement in statements {
            let mut last = None;
            for attempt in 1..=5u64 {
                match self.neo4j.graph.run(query(statement).param("d", self.dataset.clone())).await {
                    Ok(()) => {
                        last = None;
                        break;
                    }
                    Err(e) => {
                        last = Some(e);
                        tokio::time::sleep(Duration::from_millis(50 * attempt)).await;
                    }
                }
            }
            if let Some(e) = last {
                panic!("cleanup failed for {}: {e:?}", self.dataset);
            }
        }
        let mut conn = self.redis.manager.clone();
        for key in [
            format!("alloc:queue:{}", self.dataset),
            format!("lease:matcher:{}", self.dataset),
            format!("lease:matcher:{}:fence", self.dataset),
            self.params.events_stream.clone(),
            self.params.extract_stream.clone(),
        ] {
            let _: Result<(), _> = redis::cmd("DEL").arg(key).query_async(&mut conn).await;
        }
    }
}

/// Incident IDs contain `/`; encode them for the path.
fn urlencoding(id: &str) -> String {
    id.replace('/', "%2F")
}

fn review_body(request: &str, version: u64) -> Value {
    json!({
        "request_id": request, "expected_version": version, "decision": "APPROVE",
        "required_asset": "AMBULANCE", "headcount": 3, "reason": "called back: three people, ground floor",
    })
}

/// Approve: one queued task, `INCIDENT_READY`, replay-safe, version- and
/// role-checked; then the matcher assigns it.
#[tokio::test]
async fn review_approve_creates_one_task_and_is_checked() {
    let Some(ctx) = setup().await else { return };
    let incident = ctx.review_incident("1").await;
    let path = format!("/api/v1/incidents/{}/review", urlencoding(&incident));
    let mut body = review_body(&ctx.id("cmd-1"), 0);
    body["junction_id"] = json!(ctx.id("JD"));

    assert_eq!(ctx.post(&path, None, body.clone()).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(ctx.post(&path, Some("guess"), body.clone()).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(ctx.post(&path, Some(OBSERVER), body.clone()).await.0, StatusCode::FORBIDDEN);

    let (status, approved, replayed) = ctx.post(&path, Some(DISPATCHER), body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert!(!replayed);
    assert_eq!(approved["status"], "TRIAGE_APPROVED");
    let (status, again, replayed) = ctx.post(&path, Some(DISPATCHER), body.clone()).await;
    assert_eq!((status, replayed), (StatusCode::OK, true));
    assert_eq!(again["task_id"], approved["task_id"], "replay returns the original outcome");

    let mut stale = review_body(&ctx.id("cmd-2"), 0);
    stale["junction_id"] = json!(ctx.id("JD"));
    assert_eq!(ctx.post(&path, Some(DISPATCHER), stale).await.0, StatusCode::CONFLICT);

    assert_eq!(
        ctx.count("MATCH (:OpsIncident {incident_id: $d + '/I-1'})-[:HAS_TASK]->(t:Task {status: 'QUEUED'}) RETURN count(t) AS n").await,
        1
    );
    assert_eq!(
        ctx.count("MATCH (e:OutboxEvent {type: 'INCIDENT_READY', aggregate_id: $d + '/I-1'}) RETURN count(e) AS n").await,
        1
    );
    let detail = ctx.incident(&incident).await;
    assert_eq!(detail["display_status"], "QUEUED");
    assert_eq!(detail["location_source"], "DISPATCHER");
    assert_eq!(detail["headcount"], 3);
    assert_eq!(detail["callback_required"], true, "urgency still UNKNOWN");
    assert!(detail["audit"].as_array().unwrap().iter().any(|a| a["action"] == "INCIDENT_REVIEW_APPROVED" && a["actor_id"] == "dispatcher"));

    let matcher = ctx.matcher().await;
    assert_eq!(ctx.match_once(&matcher).await, 1);
    assert_eq!(ctx.incident(&incident).await["display_status"], "ASSIGNED");
    ctx.cleanup().await;
}

/// Reject: no task, leaves the callback list; invalid input is explained.
#[tokio::test]
async fn review_reject_and_invalid_requests() {
    let Some(ctx) = setup().await else { return };
    let incident = ctx.review_incident("2").await;
    let path = format!("/api/v1/incidents/{}/review", urlencoding(&incident));

    let mut invalid = review_body(&ctx.id("cmd-bad"), 0);
    invalid["headcount"] = json!(0);
    invalid["junction_id"] = json!(ctx.id("JD"));
    assert_eq!(ctx.post(&path, Some(DISPATCHER), invalid).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let no_location = review_body(&ctx.id("cmd-noloc"), 0);
    assert_eq!(ctx.post(&path, Some(DISPATCHER), no_location).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let mut far = review_body(&ctx.id("cmd-far"), 0);
    far["latitude"] = json!(12.0);
    far["longitude"] = json!(78.0);
    assert_eq!(ctx.post(&path, Some(DISPATCHER), far).await.0, StatusCode::UNPROCESSABLE_ENTITY);

    let reject = json!({ "request_id": ctx.id("cmd-rej"), "expected_version": 0, "decision": "REJECT",
                         "reason": "duplicate of a call already handled by radio" });
    let (status, outcome, _) = ctx.post(&path, Some(DISPATCHER), reject).await;
    assert_eq!(status, StatusCode::OK, "{outcome}");
    let detail = ctx.incident(&incident).await;
    assert_eq!(detail["display_status"], "REJECTED");
    assert_eq!(detail["tasks"].as_array().unwrap().len(), 0);
    let (_, alerts) = ctx.get("/api/v1/alerts").await;
    assert!(!alerts["callbacks"].as_array().unwrap().iter().any(|c| c["incident_id"] == incident.as_str()));
    let (_, list) = ctx.get("/api/v1/incidents?status=REJECTED&limit=200").await;
    assert!(list["incidents"].as_array().unwrap().iter().any(|i| i["incident_id"] == incident.as_str()));
    ctx.cleanup().await;
}

/// Task complete frees the vehicle at the drop-off, keeps the beds, closes
/// the incident and announces the vehicle; replay-safe and role-checked.
#[tokio::test]
async fn task_complete_frees_the_vehicle_at_the_dropoff() {
    let Some(ctx) = setup().await else { return };
    let incident = ctx.review_incident("3").await;
    let mut body = review_body(&ctx.id("cmd-3"), 0);
    body["junction_id"] = json!(ctx.id("JD"));
    let (_, approved, _) = ctx.post(&format!("/api/v1/incidents/{}/review", urlencoding(&incident)), Some(DISPATCHER), body).await;
    let task_id = approved["task_id"].as_str().unwrap().to_string();
    let matcher = ctx.matcher().await;
    assert_eq!(ctx.match_once(&matcher).await, 1);

    let detail = ctx.incident(&incident).await;
    let task = &detail["tasks"][0];
    assert_eq!(task["status"], "ASSIGNED");
    let version = task["version"].as_u64().unwrap();
    let (_, route) = ctx.get(&format!("/api/v1/incidents/{}/route", urlencoding(&incident))).await;
    let line = &route["features"][0]["geometry"]["coordinates"];
    assert_eq!(line.as_array().unwrap().len(), 3, "A -> B -> D: {route}");
    assert!(route["features"].as_array().unwrap().iter().any(|f| f["properties"]["kind"] == "dropoff"));

    let path = format!("/api/v1/tasks/{task_id}/complete");
    let complete = json!({ "request_id": ctx.id("cmd-done"), "expected_version": version, "note": "delivered" });
    assert_eq!(ctx.post(&path, Some("guess"), complete.clone()).await.0, StatusCode::UNAUTHORIZED);
    let (status, done, _) = ctx.post(&path, Some(DISPATCHER), complete.clone()).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["incident_status"], "COMPLETED");
    assert_eq!(done["vehicle_position_junction"], json!(ctx.id("JD")), "the vehicle is at the drop-off");
    let (status, _, replayed) = ctx.post(&path, Some(DISPATCHER), complete).await;
    assert_eq!((status, replayed), (StatusCode::OK, true));
    let again = json!({ "request_id": ctx.id("cmd-done-2"), "expected_version": version });
    assert_eq!(ctx.post(&path, Some(DISPATCHER), again).await.0, StatusCode::CONFLICT);

    let (_, vehicles) = ctx.get("/api/v1/vehicles").await;
    let vehicle = &vehicles["vehicles"][0];
    assert_eq!((vehicle["status"].as_str(), vehicle["task_id"].as_str()), (Some("AVAILABLE"), None));
    assert_eq!(vehicle["position"]["lat"], 10.01);
    let (_, shelters) = ctx.get("/api/v1/shelters").await;
    let dropoff = shelters["shelters"].as_array().unwrap().iter().find(|s| s["shelter_id"] == ctx.id("S_D").as_str()).cloned().unwrap();
    assert_eq!(dropoff["occupancy"], 3, "the delivered people keep their beds");
    assert_eq!(ctx.incident(&incident).await["display_status"], "COMPLETED");
    assert_eq!(
        ctx.count("MATCH (r:Reservation {resource_kind: 'VEHICLE', status: 'RELEASED'}) WHERE r.incident_id STARTS WITH $d RETURN count(r) AS n").await,
        1
    );
    let available = ctx
        .count("MATCH (e:OutboxEvent {type: 'VEHICLE_AVAILABLE'}) WHERE e.aggregate_id = $d + '/AMB-1' RETURN count(e) AS n")
        .await;
    assert_eq!(available, 1);
    ctx.cleanup().await;
}

/// The read endpoints the console uses, on one assigned incident.
#[tokio::test]
async fn read_endpoints_describe_the_pipeline() {
    let Some(ctx) = setup().await else { return };
    let incident = ctx.review_incident("4").await;
    let mut body = review_body(&ctx.id("cmd-4"), 0);
    body["junction_id"] = json!(ctx.id("JD"));
    ctx.post(&format!("/api/v1/incidents/{}/review", urlencoding(&incident)), Some(DISPATCHER), body).await;

    let (status, queue) = ctx.get("/api/v1/queue").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(queue["total"], 1);
    assert_eq!(queue["tiers"]["URGENT"].as_array().unwrap().len(), 1, "UNKNOWN is matched with URGENT");

    let (_, near) = ctx.get("/api/v1/segments/near?lat=10.0001&lon=76.0001").await;
    assert_eq!(near["junction_id"], json!(ctx.id("JA")));
    let segments: Vec<&str> = near["segments"].as_array().unwrap().iter().map(|s| s["segment_id"].as_str().unwrap()).collect();
    assert_eq!(segments.len(), 2, "ab and ac leave A: {segments:?}");

    let (_, system) = ctx.get("/api/v1/system").await;
    assert_eq!(system["dataset"], json!(ctx.dataset));
    assert!(system.get("matcher_leader").is_some() && system.get("relay_leader").is_some());

    let (status, hazards) = ctx.get("/api/v1/hazards/active").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hazards["geojson"]["features"].as_array().unwrap().len(), 0);

    let (_, list) = ctx.get("/api/v1/incidents?limit=200").await;
    let row = list["incidents"].as_array().unwrap().iter().find(|i| i["incident_id"] == incident.as_str()).cloned().unwrap();
    assert_eq!(row["display_status"], "QUEUED");
    let (status, _) = ctx.get("/api/v1/incidents/no-such-incident").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    ctx.cleanup().await;
}

/// The event feed delivers live events and, on reconnect, replays only what
/// came after the browser's `Last-Event-ID`.
#[tokio::test]
async fn event_feed_streams_and_replays_after_last_event_id() {
    let Some(ctx) = setup().await else { return };
    let stream = ctx.params.events_stream.clone();
    let hub = EventHub::new(ctx.redis.clone(), stream.clone());
    let shutdown = Shutdown::new();
    let reader = ctx.redis.dedicated().await.unwrap();
    tokio::spawn(hub.clone().run(reader, shutdown.clone()));
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut live = Box::pin(hub.subscribe(None).await.unwrap());
    let publish = |n: usize| {
        let redis = ctx.redis.clone();
        let stream = stream.clone();
        async move {
            let payload = json!({ "n": n }).to_string();
            redis
                .xadd_fields(&stream, &[("event_id", &format!("e{n}")), ("type", "TASK_ASSIGNED"), ("aggregate_id", "t"), ("payload", &payload)])
                .await
                .unwrap()
        }
    };
    let first = publish(1).await;
    let second = publish(2).await;
    let got = tokio::time::timeout(Duration::from_secs(5), async {
        let a = live.next().await.unwrap();
        let b = live.next().await.unwrap();
        (a, b)
    })
    .await
    .expect("live events");
    assert_eq!((got.0.id.as_str(), got.1.id.as_str()), (first.as_str(), second.as_str()));
    assert_eq!(event_body(&got.1)["payload"]["n"], 2);

    // A browser that saw only the first event gets the second back, then live.
    let mut resumed = Box::pin(hub.subscribe(Some(first.clone())).await.unwrap());
    let replayed = tokio::time::timeout(Duration::from_secs(5), resumed.next()).await.unwrap().unwrap();
    assert_eq!(replayed.id, second);
    let third = publish(3).await;
    let next = tokio::time::timeout(Duration::from_secs(5), resumed.next()).await.unwrap().unwrap();
    assert_eq!(next.id, third, "no duplicate of the replayed event");
    shutdown.trigger();
    ctx.cleanup().await;
}

/// End to end on the new pipeline: submit-and-wait returns the assigned
/// vehicle and route; after completion the same vehicle serves the next
/// report; an unclear report goes to review, is approved and assigned.
#[tokio::test]
async fn dispatch_runs_end_to_end_on_the_new_pipeline() {
    let Some(ctx) = setup().await else { return };
    let ctx = Arc::new(ctx);
    let worker = Arc::new(ctx.worker());
    let matcher = Arc::new(ctx.matcher().await);

    // Stand-ins for the triage and matcher roles: process whatever is pending.
    let stop = Shutdown::new();
    let roles = {
        let (ctx, worker, matcher, stop) = (ctx.clone(), worker.clone(), matcher.clone(), stop.clone());
        tokio::spawn(async move {
            while !stop.is_triggered() {
                let mut result = ctx
                    .neo4j
                    .graph
                    .execute(query("MATCH (r:Report {extraction_status: 'PENDING'}) WHERE r.request_id STARTS WITH $d RETURN r.report_id AS id").param("d", ctx.dataset.clone()))
                    .await
                    .unwrap();
                let mut pending = Vec::new();
                while let Some(row) = result.next().await.unwrap() {
                    pending.push(row.get::<String>("id").unwrap());
                }
                for report in pending {
                    worker.process_report(&report).await.unwrap();
                }
                ctx.match_once(&matcher).await;
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        })
    };

    let report = |n: u32, text: &str| {
        json!({ "request_id": ctx.id(&format!("req-{n}")), "device_id": ctx.id(&format!("dev-{n}")),
                "raw_text": text, "latitude": 10.01 + n as f64 * 0.0001, "longitude": 76.01, "accuracy_m": 15 })
    };
    let (status, first, _) = ctx.post("/api/v1/dispatch", None, report(1, "2 people on the roof, water rising")).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["status"], "ASSIGNED");
    assert_eq!(first["route"]["features"][0]["geometry"]["type"], "LineString");
    let task = &first["incident"]["tasks"][0];

    // Only one vehicle: the next report waits until the first task completes.
    let complete = json!({ "request_id": ctx.id("done-1"), "expected_version": task["version"] });
    let (status, done, _) = ctx.post(&format!("/api/v1/tasks/{}/complete", task["task_id"].as_str().unwrap()), Some(DISPATCHER), complete).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    let (status, second, _) = ctx.post("/api/v1/dispatch", None, report(2, "3 people stuck near the market")).await;
    assert_eq!((status, second["status"].as_str()), (StatusCode::OK, Some("ASSIGNED")), "{second}");

    // No headcount: review, then the dispatcher approves at a clicked point.
    let task = &second["incident"]["tasks"][0];
    let complete = json!({ "request_id": ctx.id("done-2"), "expected_version": task["version"] });
    ctx.post(&format!("/api/v1/tasks/{}/complete", task["task_id"].as_str().unwrap()), Some(DISPATCHER), complete).await;
    let (status, unclear, _) = ctx.post("/api/v1/dispatch", None, report(3, "please help us, water everywhere")).await;
    assert_eq!((status, unclear["status"].as_str()), (StatusCode::OK, Some("REVIEW_REQUIRED")), "{unclear}");
    let incident = unclear["incident"]["incident_id"].as_str().unwrap().to_string();
    let version = unclear["incident"]["version"].as_u64().unwrap();
    let mut approve = review_body(&ctx.id("cmd-unclear"), version);
    approve["latitude"] = json!(10.0101);
    approve["longitude"] = json!(76.0101);
    let (status, approved, _) = ctx.post(&format!("/api/v1/incidents/{}/review", urlencoding(&incident)), Some(DISPATCHER), approve).await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let mut assigned = false;
    for _ in 0..50 {
        if ctx.incident(&incident).await["display_status"] == "ASSIGNED" {
            assigned = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(assigned, "approved review incident is assigned by the matcher");

    // The report status endpoint now links the report to its incident.
    let (_, status_body) = ctx.get(&format!("/api/v1/reports/{}", unclear["report_id"].as_str().unwrap())).await;
    assert_eq!(status_body["incident_id"], json!(incident));

    stop.trigger();
    let _ = roles.await;
    ctx.cleanup().await;
}
