//! M4 acceptance tests: hazard records and command T7, the shared graph
//! snapshot, and the hazard API. Live Neo4j + Redis (skipped if unreachable).
//!
//! Each test builds its own tiny road network in a dataset named
//! `TEST-<uuid>`, so the Aluva baseline is never touched:
//!
//! ```text
//!   A --ab (60 s)--> B --bd (60 s)--> D      fast
//!   A --ac (300 s)-> C --cd (300 s)-> D      slow detour
//! ```

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use neo4rs::query;
use serde_json::{json, Value};
use uuid::Uuid;

use crisis_graph::api::{hazard_routes, token_hash_hex, HazardApiState, OperatorTokens};
use crisis_graph::config::Config;
use crisis_graph::db::{ensure_ops_schema, Neo4jClient, OpsRepoError, OpsRepository, RedisClient, StateManager, TraversalProfile};
use crisis_graph::graph::GraphSnapshot;
use crisis_graph::models::{
    ActorContext, ActorRole, AssetType, HazardCommand, HazardOutcome, HazardSource, HazardStatus, TaskKind, TaskStatus,
};
use crisis_graph::runtime::Shutdown;
use crisis_graph::solver::RoutingQuery;

struct Ctx {
    neo4j: Neo4jClient,
    redis: RedisClient,
    repo: OpsRepository,
    dataset: String,
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
    let ctx = Ctx { repo: OpsRepository::new(neo4j.clone()), neo4j, redis, dataset: format!("TEST-{}", Uuid::new_v4()) };
    ctx.build_network().await;
    Some(ctx)
}

impl Ctx {
    fn segment(&self, name: &str) -> String {
        format!("{}/{name}", self.dataset)
    }

    fn junction(&self, name: &str) -> String {
        format!("{}/J{name}", self.dataset)
    }

    async fn build_network(&self) {
        let junctions = [("A", 10.00, 76.00), ("B", 10.00, 76.01), ("C", 10.01, 76.00), ("D", 10.01, 76.01)];
        for (name, lat, lon) in junctions {
            self.neo4j
                .graph
                .run(
                    query("CREATE (:Junction {id: $id, dataset: $dataset, lat: $lat, lon: $lon})")
                        .param("id", self.junction(name))
                        .param("dataset", self.dataset.clone())
                        .param("lat", lat)
                        .param("lon", lon),
                )
                .await
                .unwrap();
        }
        let segments = [("ab", "A", "B", 60.0), ("bd", "B", "D", 60.0), ("ac", "A", "C", 300.0), ("cd", "C", "D", 300.0)];
        for (name, from, to, seconds) in segments {
            self.neo4j
                .graph
                .run(
                    query(
                        "MATCH (a:Junction {id: $from, dataset: $dataset}), (b:Junction {id: $to, dataset: $dataset}) \
                         CREATE (a)-[:CONNECTS_TO {segment_id: $segment_id, dataset: $dataset, length_m: 1000.0, \
                             travel_time_s: $seconds, road_class: 'primary', oneway: true, emergency: true, \
                             motorcar: true, hgv: true, baseline_status: 'OPEN'}]->(b)",
                    )
                    .param("from", self.junction(from))
                    .param("to", self.junction(to))
                    .param("dataset", self.dataset.clone())
                    .param("segment_id", self.segment(name))
                    .param("seconds", seconds),
                )
                .await
                .unwrap();
        }
    }

    fn command(&self, request: &str, segment: &str, status: HazardStatus) -> HazardCommand {
        HazardCommand {
            request_id: format!("{}-{request}", self.dataset),
            dataset: self.dataset.clone(),
            segment_id: self.segment(segment),
            status,
            valid_until: Some(Utc::now() + chrono::Duration::hours(6)),
            reason: "test hazard".into(),
            source: HazardSource::Operator,
        }
    }

    async fn count(&self, cypher: &str) -> i64 {
        let mut result = self.neo4j.graph.execute(query(cypher).param("d", self.dataset.clone())).await.unwrap();
        result.next().await.unwrap().unwrap().get::<i64>("n").unwrap()
    }

    async fn version(&self) -> u64 {
        self.repo.overlay_version(&self.dataset).await.unwrap()
    }

    async fn cleanup(&self) {
        let statements = [
            "MATCH (h:Hazard {dataset: $d}) OPTIONAL MATCH (h)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE h, a",
            "MATCH (e:OutboxEvent {aggregate_id: $d}) DELETE e",
            "MATCH (o:OverlayState {dataset: $d}) DELETE o",
            "MATCH (t:Task) WHERE t.task_id STARTS WITH $d DETACH DELETE t",
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
        let _: Result<(), _> = redis::cmd("DEL").arg(format!("{}:events", self.dataset)).query_async(&mut conn).await;
    }
}

fn verifier() -> ActorContext {
    ActorContext::new("test-verifier", [ActorRole::HazardVerifier])
}

/// T7 writes the record, version, audit and outbox together, and never
/// writes on the baseline road segment.
#[tokio::test]
async fn hazard_command_is_atomic_and_never_touches_the_road() {
    let Some(ctx) = setup().await else { return };
    let road_keys = "MATCH ()-[r:CONNECTS_TO {dataset: $d, segment_id: $d + '/bd'}]->() \
                     RETURN size(keys(r)) AS n";
    let keys_before = ctx.count(road_keys).await;

    let outcome = ctx.repo.apply_hazard(&verifier(), &ctx.command("1", "bd", HazardStatus::Flooded)).await.unwrap();
    let keys_after = ctx.count(road_keys).await;
    let active = ctx.count("MATCH (h:Hazard {dataset: $d}) WHERE h.active_key IS NOT NULL RETURN count(h) AS n").await;
    let audits = ctx
        .count("MATCH (:Hazard {dataset: $d})-[:HAS_AUDIT_EVENT]->(a:AuditEvent {action: 'HAZARD_APPLIED'}) RETURN count(a) AS n")
        .await;
    let events = ctx.count("MATCH (e:OutboxEvent {aggregate_id: $d, type: 'HAZARDS_CHANGED'}) RETURN count(e) AS n").await;
    let version = ctx.version().await;
    ctx.cleanup().await;

    let HazardOutcome::Applied { hazard, overlay_version, replaced } = outcome else { panic!("{outcome:?}") };
    assert_eq!((overlay_version, version, hazard.overlay_version), (1, 1, 1));
    assert_eq!(replaced, None);
    assert_eq!((active, audits, events), (1, 1, 1));
    assert_eq!(keys_before, keys_after, "the baseline road segment is unchanged");
}

/// A retried command (same request_id) changes nothing.
#[tokio::test]
async fn repeated_request_id_is_replayed() {
    let Some(ctx) = setup().await else { return };
    let command = ctx.command("1", "bd", HazardStatus::Flooded);
    let first = ctx.repo.apply_hazard(&verifier(), &command).await.unwrap();
    let second = ctx.repo.apply_hazard(&verifier(), &command).await.unwrap();
    let version = ctx.version().await;
    let events = ctx.count("MATCH (e:OutboxEvent {aggregate_id: $d}) RETURN count(e) AS n").await;
    ctx.cleanup().await;

    let (HazardOutcome::Applied { hazard: a, .. }, HazardOutcome::Replayed { hazard: b }) = (&first, &second) else {
        panic!("{first:?} / {second:?}");
    };
    assert_eq!(a.hazard_id, b.hazard_id);
    assert_eq!((version, events), (1, 1));
}

/// A new hazard supersedes the active one; clearing ends it; clearing again
/// changes nothing.
#[tokio::test]
async fn supersede_then_clear_keeps_history() {
    let Some(ctx) = setup().await else { return };
    let first = ctx.repo.apply_hazard(&verifier(), &ctx.command("1", "bd", HazardStatus::Flooded)).await.unwrap();
    let second = ctx.repo.apply_hazard(&verifier(), &ctx.command("2", "bd", HazardStatus::Blocked)).await.unwrap();
    let active = ctx.repo.active_hazards(&ctx.dataset).await.unwrap();
    let cleared = ctx.repo.clear_hazard(&verifier(), &ctx.dataset, &ctx.segment("bd"), "water receded").await.unwrap();
    let again = ctx.repo.clear_hazard(&verifier(), &ctx.dataset, &ctx.segment("bd"), "water receded").await.unwrap();
    let version = ctx.version().await;
    let history = ctx.count("MATCH (h:Hazard {dataset: $d}) RETURN count(h) AS n").await;
    let still_active = ctx.repo.active_hazards(&ctx.dataset).await.unwrap();
    ctx.cleanup().await;

    let HazardOutcome::Applied { hazard: first, .. } = first else { panic!() };
    let HazardOutcome::Applied { replaced, .. } = second else { panic!() };
    assert_eq!(replaced, Some(first.hazard_id));
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].status, HazardStatus::Blocked);
    assert!(matches!(cleared, HazardOutcome::Cleared { overlay_version: 3, .. }), "{cleared:?}");
    assert_eq!(again, HazardOutcome::NothingToClear);
    assert_eq!(version, 3, "a no-op clear does not bump the version");
    assert_eq!(history, 2, "ended records are kept");
    assert!(still_active.is_empty());
}

/// Concurrent changes to one segment: every one applies in turn, versions
/// strictly increase with none lost, and exactly one hazard stays active.
#[tokio::test]
async fn concurrent_changes_serialise_on_the_overlay_version() {
    let Some(ctx) = setup().await else { return };
    let ctx = Arc::new(ctx);
    let mut tasks = Vec::new();
    for n in 0..10 {
        let ctx = ctx.clone();
        tasks.push(tokio::spawn(async move {
            ctx.repo.apply_hazard(&verifier(), &ctx.command(&n.to_string(), "bd", HazardStatus::Flooded)).await
        }));
    }
    let mut versions = Vec::new();
    for task in tasks {
        match task.await.unwrap().unwrap() {
            HazardOutcome::Applied { overlay_version, .. } => versions.push(overlay_version),
            other => panic!("{other:?}"),
        }
    }
    let active = ctx.repo.active_hazards(&ctx.dataset).await.unwrap().len();
    let version = ctx.version().await;
    ctx.cleanup().await;

    versions.sort_unstable();
    assert_eq!(versions, (1..=10).collect::<Vec<u64>>());
    assert_eq!((active, version), (1, 10));
}

/// Roles are enforced and unknown segments are rejected, with nothing written.
#[tokio::test]
async fn forbidden_actor_and_unknown_segment_change_nothing() {
    let Some(ctx) = setup().await else { return };
    let observer = ActorContext::new("test-observer", [ActorRole::Observer]);
    let dispatcher = ActorContext::new("test-dispatcher", [ActorRole::Dispatcher]);
    let forbidden = ctx.repo.apply_hazard(&observer, &ctx.command("1", "bd", HazardStatus::Flooded)).await;
    let dispatcher_result = ctx.repo.apply_hazard(&dispatcher, &ctx.command("2", "bd", HazardStatus::Flooded)).await;
    let unknown = ctx.repo.apply_hazard(&verifier(), &ctx.command("3", "zz", HazardStatus::Flooded)).await;
    let version = ctx.version().await;
    let records = ctx.count("MATCH (h:Hazard {dataset: $d}) RETURN count(h) AS n").await;
    ctx.cleanup().await;

    assert!(matches!(forbidden, Err(OpsRepoError::Forbidden(_))), "{forbidden:?}");
    assert!(matches!(dispatcher_result, Err(OpsRepoError::Forbidden(_))), "dispatchers cannot change hazards");
    assert!(matches!(unknown, Err(OpsRepoError::NotFound(_))), "{unknown:?}");
    assert_eq!((version, records), (0, 0));
}

/// M4 acceptance: a hazard change reaches routing through the events stream
/// without reloading the baseline, and a route computed on the old overlay
/// fails live verification. The task records the overlay version it used.
#[tokio::test]
async fn hazard_reaches_routing_without_reloading_and_old_route_fails_verification() {
    let Some(ctx) = setup().await else { return };
    let state = StateManager::new(ctx.neo4j.clone());
    let snapshot = Arc::new(GraphSnapshot::load(ctx.repo.clone(), &state, &ctx.dataset).await.unwrap());
    let baseline_before = snapshot.baseline();
    let query = RoutingQuery::new(ctx.junction("A"), ctx.junction("D")).with_asset_type(AssetType::Ambulance);
    let old = snapshot.route(&query, Utc::now()).unwrap();

    // The refresher only reacts to events here (fallback refresh: 1 hour).
    let stream = format!("{}:events", ctx.dataset);
    let shutdown = Shutdown::new();
    let refresher = tokio::spawn(snapshot.clone().run_refresher(
        ctx.redis.clone(),
        stream.clone(),
        Duration::from_secs(3600),
        shutdown.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(300)).await;

    ctx.repo.apply_hazard(&verifier(), &ctx.command("1", "bd", HazardStatus::Flooded)).await.unwrap();
    // Stand in for the outbox relay: publish the committed event.
    ctx.redis
        .xadd_fields(&stream, &[("type", "HAZARDS_CHANGED"), ("aggregate_id", ctx.dataset.as_str())])
        .await
        .unwrap();
    let mut swapped = false;
    for _ in 0..50 {
        if snapshot.overlay().version == 1 {
            swapped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let new = snapshot.route(&query, Utc::now()).unwrap();
    shutdown.trigger();
    let _ = refresher.await;

    let flagged = state
        .verify_path_segments(&old.route.segment_path, &TraversalProfile::Vehicle(AssetType::Ambulance))
        .await
        .unwrap();
    let fresh_flagged = state
        .verify_path_segments(&new.route.segment_path, &TraversalProfile::Vehicle(AssetType::Ambulance))
        .await
        .unwrap();

    let task_id = format!("{}-TASK", ctx.dataset);
    ctx.repo
        .create_task(&task_id, "no-incident", TaskKind::RoadPickup, TaskStatus::Queued, AssetType::Ambulance)
        .await
        .unwrap();
    let route_json = serde_json::to_string(&new.route).unwrap();
    let task = ctx.repo.record_task_route(&task_id, 0, new.overlay_version, &route_json).await.unwrap();
    ctx.cleanup().await;

    assert!(swapped, "HAZARDS_CHANGED swapped the overlay");
    assert!(Arc::ptr_eq(&baseline_before, &snapshot.baseline()), "baseline not reloaded");
    assert_eq!(old.route.segment_path, vec![ctx.segment("ab"), ctx.segment("bd")]);
    assert_eq!(old.overlay_version, 0);
    assert_eq!(new.route.segment_path, vec![ctx.segment("ac"), ctx.segment("cd")]);
    assert_eq!(new.overlay_version, 1);
    assert_eq!(flagged, vec![ctx.segment("bd")], "old route fails live verification");
    assert!(fresh_flagged.is_empty());
    assert_eq!(task.overlay_version, Some(1));
}

/// An expiring hazard stops applying at `valid_until`, with no new version.
#[tokio::test]
async fn expired_hazard_stops_applying_without_a_new_version() {
    let Some(ctx) = setup().await else { return };
    let state = StateManager::new(ctx.neo4j.clone());
    let mut command = ctx.command("1", "bd", HazardStatus::Flooded);
    command.valid_until = Some(Utc::now() + chrono::Duration::seconds(2));
    ctx.repo.apply_hazard(&verifier(), &command).await.unwrap();
    let snapshot = GraphSnapshot::load(ctx.repo.clone(), &state, &ctx.dataset).await.unwrap();
    let query = RoutingQuery::new(ctx.junction("A"), ctx.junction("D")).with_asset_type(AssetType::Ambulance);
    let during = snapshot.route(&query, Utc::now()).unwrap();
    let path = vec![ctx.segment("ab"), ctx.segment("bd")];
    let flagged_during = state.verify_path_segments(&path, &TraversalProfile::Vehicle(AssetType::Ambulance)).await.unwrap();

    tokio::time::sleep(Duration::from_millis(2500)).await;
    let after = snapshot.route(&query, Utc::now()).unwrap();
    let flagged_after = state.verify_path_segments(&path, &TraversalProfile::Vehicle(AssetType::Ambulance)).await.unwrap();
    let version = ctx.version().await;
    ctx.cleanup().await;

    assert_eq!(during.route.segment_path, vec![ctx.segment("ac"), ctx.segment("cd")]);
    assert_eq!(flagged_during, vec![ctx.segment("bd")]);
    assert_eq!(after.route.segment_path, path);
    assert!(flagged_after.is_empty());
    assert_eq!((after.overlay_version, version), (1, 1));
}

/// The HTTP command: 401 without a valid token, 403 for a role without
/// permission, 201 / replay / 404 / clear for a hazard verifier.
#[tokio::test]
async fn hazard_api_enforces_operator_tokens_and_roles() {
    let Some(ctx) = setup().await else { return };
    let spec = format!(
        "asha:HAZARD_VERIFIER:{},obs:OBSERVER:{}",
        token_hash_hex("verifier-token"),
        token_hash_hex("observer-token")
    );
    let (operators, problems) = OperatorTokens::parse(&spec);
    assert!(problems.is_empty());
    let app = hazard_routes(HazardApiState {
        repo: ctx.repo.clone(),
        operators: Arc::new(operators),
        default_dataset: ctx.dataset.clone(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let body = json!({
        "request_id": format!("{}-HTTP-1", ctx.dataset),
        "segment_id": ctx.segment("bd"),
        "status": "FLOODED",
        "duration_hours": 6,
        "reason": "field team: road under water"
    });
    let post = |token: Option<&str>, path: &str, body: &Value| {
        let mut request = client.post(format!("{base}{path}")).json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send()
    };

    let none = post(None, "/api/v1/hazards", &body).await.unwrap().status();
    let wrong = post(Some("guess"), "/api/v1/hazards", &body).await.unwrap().status();
    let observer = post(Some("observer-token"), "/api/v1/hazards", &body).await.unwrap().status();
    let created = post(Some("verifier-token"), "/api/v1/hazards", &body).await.unwrap();
    let created_status = created.status();
    let created_body: Value = created.json().await.unwrap();
    let replay = post(Some("verifier-token"), "/api/v1/hazards", &body).await.unwrap();
    let replay_status = replay.status();
    let replay_header = replay.headers().get("idempotent-replayed").cloned();
    let mut unknown_body = body.clone();
    unknown_body["request_id"] = json!(format!("{}-HTTP-2", ctx.dataset));
    unknown_body["segment_id"] = json!("way/0/seg/0/fwd");
    let unknown = post(Some("verifier-token"), "/api/v1/hazards", &unknown_body).await.unwrap().status();
    let listed = client.get(format!("{base}/api/v1/hazards")).bearer_auth("observer-token").send().await.unwrap();
    let listed_status = listed.status();
    let listed_body: Value = listed.json().await.unwrap();
    let clear_body = json!({ "segment_id": ctx.segment("bd"), "reason": "water receded" });
    let cleared = post(Some("verifier-token"), "/api/v1/hazards/clear", &clear_body).await.unwrap();
    let cleared_status = cleared.status();
    let cleared_body: Value = cleared.json().await.unwrap();
    ctx.cleanup().await;

    assert_eq!((none.as_u16(), wrong.as_u16(), observer.as_u16()), (401, 401, 403));
    assert_eq!(created_status.as_u16(), 201, "{created_body}");
    assert_eq!(created_body["hazard"]["actor_id"], "asha");
    assert_eq!(created_body["overlay_version"], 1);
    assert_eq!(replay_status.as_u16(), 200);
    assert_eq!(replay_header.as_ref().and_then(|h| h.to_str().ok()), Some("true"));
    assert_eq!(unknown.as_u16(), 404);
    assert_eq!(listed_status.as_u16(), 200);
    assert_eq!(listed_body["hazards"].as_array().unwrap().len(), 1);
    assert_eq!(listed_body["hazards"][0]["in_force"], true);
    assert_eq!(cleared_status.as_u16(), 200);
    assert_eq!(cleared_body["cleared"], true);
}

/// `migrate_ops`: old-style hazard properties on a road become records (only
/// those still in force) and are removed from the road; a second run is a
/// no-op.
#[tokio::test]
async fn road_property_hazards_migrate_to_records_once() {
    let Some(ctx) = setup().await else { return };
    ctx.neo4j
        .graph
        .run(
            query(
                "MATCH ()-[r:CONNECTS_TO {dataset: $d}]->() WHERE r.segment_id = $d + '/bd' \
                 SET r.operational_status = 'FLOODED', r.valid_until = datetime() + duration('PT6H')",
            )
            .param("d", ctx.dataset.clone()),
        )
        .await
        .unwrap();
    ctx.neo4j
        .graph
        .run(
            query(
                "MATCH ()-[r:CONNECTS_TO {dataset: $d}]->() WHERE r.segment_id = $d + '/cd' \
                 SET r.operational_status = 'BLOCKED', r.valid_until = datetime() - duration('PT1H')",
            )
            .param("d", ctx.dataset.clone()),
        )
        .await
        .unwrap();

    ctx.repo.migrate_segment_hazards().await.unwrap();
    let active: HashSet<String> =
        ctx.repo.active_hazards(&ctx.dataset).await.unwrap().into_iter().map(|h| h.segment_id).collect();
    let leftover = ctx
        .count("MATCH ()-[r:CONNECTS_TO {dataset: $d}]->() WHERE r.operational_status IS NOT NULL RETURN count(r) AS n")
        .await;
    let version = ctx.version().await;
    ctx.repo.migrate_segment_hazards().await.unwrap();
    let version_after_second = ctx.version().await;
    let records = ctx.count("MATCH (h:Hazard {dataset: $d}) RETURN count(h) AS n").await;
    ctx.cleanup().await;

    assert_eq!(active, HashSet::from([ctx.segment("bd")]), "only the hazard still in force is migrated");
    assert_eq!(leftover, 0, "road properties removed");
    assert_eq!((version, version_after_second, records), (1, 1, 1));
}
