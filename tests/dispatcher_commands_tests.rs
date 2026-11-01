//! M6a acceptance tests: merge and split of duplicates (T4, rule D-4),
//! vehicle status (including breakdowns), and the simulated driver. Live
//! Neo4j + Redis (skipped if unreachable).
//!
//! Each test builds its own network in a dataset named `TEST-<uuid>`:
//!
//! ```text
//!   A --ab (60 s)--> B --bd (60 s)--> D      fast
//!   A --ac (300 s)-> C --cd (300 s)-> D      slow detour
//!   depot S_A at A (no beds), drop-off shelter S_D at D (50 beds)
//! ```

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use neo4rs::query;
use reqwest::StatusCode;
use serde_json::{json, Value};
use uuid::Uuid;

use crisis_graph::api::{console_routes, token_hash_hex, ConsoleApiState, OperatorTokens};
use crisis_graph::config::{Config, PipelineParams};
use crisis_graph::db::{
    ensure_ops_schema, MergeCommand, Neo4jClient, OpsRepoError, OpsRepository, RedisClient, SplitCommand, StateManager,
    VehicleStatusCommand,
};
use crisis_graph::graph::GraphSnapshot;
use crisis_graph::matcher::Matcher;
use crisis_graph::models::{ActorContext, ActorRole};
use crisis_graph::sim::SimDriver;

struct Ctx {
    neo4j: Neo4jClient,
    redis: RedisClient,
    repo: OpsRepository,
    dataset: String,
    params: PipelineParams,
}

async fn setup(vehicles: usize) -> Option<Ctx> {
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
        sim_time_scale: 0.0001,
        sim_on_scene_secs: 1,
        ..PipelineParams::default()
    };
    let ctx = Ctx { repo: OpsRepository::new(neo4j.clone()), neo4j, redis, dataset, params };
    ctx.build(vehicles).await;
    Some(ctx)
}

fn dispatcher() -> ActorContext {
    ActorContext::new("dispatcher", [ActorRole::Dispatcher])
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

    async fn value<T: serde::de::DeserializeOwned>(&self, cypher: &str, params: &[(&str, neo4rs::BoltType)]) -> T {
        let mut q = query(cypher).param("d", self.dataset.clone());
        for (key, value) in params {
            q = q.param(key, value.clone());
        }
        let mut result = self.neo4j.graph.execute(q).await.unwrap();
        result.next().await.unwrap().unwrap().get::<T>("v").unwrap()
    }

    async fn build(&self, vehicles: usize) {
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
                     travel_time_s: $seconds, road_class: 'primary', oneway: true, emergency: true, \
                     motorcar: true, hgv: true, baseline_status: 'OPEN'}]->(b)",
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
        for n in 1..=vehicles {
            self.run(
                "MATCH (s:Shelter {id: $d + '/S_A'}) \
                 CREATE (v:Vehicle {vehicle_id: $d + '/AMB-' + $n, asset_type: 'AMBULANCE', org: 'test', \
                     capacity_persons: 6, status: 'AVAILABLE', version: 0, home_shelter_id: s.id, \
                     junction_id: $d + '/JA', created_at: datetime(), updated_at: datetime()})-[:BASED_AT]->(s)",
                &[("n", (n as i64).to_string().into())],
            )
            .await;
        }
    }

    /// An approved incident at junction D (with a queued task unless
    /// `with_task` is false) and its primary report.
    async fn incident(&self, name: &str, rank: i64, headcount: i64, with_task: bool) -> String {
        let id = self.id(&format!("I-{name}"));
        self.run(
            "MATCH (j:Junction {id: $d + '/JD', dataset: $d}) \
             CREATE (i:OpsIncident {incident_id: $id, status: 'TRIAGE_APPROVED', version: 0, headcount: $headcount, \
                 priority_rank: $rank, urgency: CASE $rank WHEN 2 THEN 'CRITICAL' WHEN 0 THEN 'STABLE' ELSE 'URGENT' END, \
                 required_asset: 'AMBULANCE', junction_id: j.id, report_count: 1, created_at: datetime(), updated_at: datetime()}) \
             CREATE (i)-[:LOCATED_AT]->(j) \
             CREATE (:Report {report_id: $id + '/R1', request_id: $id + '/R1', device_id: $id, headcount: $headcount, \
                 required_asset: 'AMBULANCE', resolved_junction_id: j.id, urgency: 'URGENT', priority_rank: 1, \
                 raw_text: 'test report', received_at: datetime(), extraction_status: 'EXTRACTED'}) \
                 -[:REPORTS {kind: 'PRIMARY', rule: 'NEW'}]->(i) \
             FOREACH (_ IN CASE WHEN $with_task THEN [1] ELSE [] END | \
                 CREATE (i)-[:HAS_TASK {seq: 1}]->(:Task {task_id: $id + '/T', incident_id: $id, kind: 'ROAD_PICKUP', \
                     status: 'QUEUED', required_asset: 'AMBULANCE', priority_rank: $rank, version: 0, \
                     created_at: datetime(), queued_at: datetime()}))",
            &[
                ("id", id.clone().into()),
                ("rank", rank.into()),
                ("headcount", headcount.into()),
                ("with_task", with_task.into()),
            ],
        )
        .await;
        id
    }

    async fn version(&self, label: &str, key: &str, id: &str) -> u64 {
        self.value::<i64>(&format!("MATCH (n:{label} {{{key}: $id}}) RETURN n.version AS v"), &[("id", id.into())]).await as u64
    }

    async fn status(&self, label: &str, key: &str, id: &str) -> String {
        self.value(&format!("MATCH (n:{label} {{{key}: $id}}) RETURN n.status AS v"), &[("id", id.into())]).await
    }

    async fn matcher(&self) -> Matcher {
        let snapshot = GraphSnapshot::load(self.repo.clone(), &StateManager::new(self.neo4j.clone()), &self.dataset)
            .await
            .expect("snapshot");
        Matcher::new(self.repo.clone(), self.redis.clone(), StateManager::new(self.neo4j.clone()), Arc::new(snapshot), self.params.clone())
    }

    async fn match_once(&self, matcher: &Matcher) -> usize {
        let mut lease = matcher.lease();
        let token = lease.try_acquire().await.unwrap().expect("matcher lease");
        matcher.sync_queue().await.unwrap();
        let assigned = matcher.run_cycle(token).await.unwrap().assigned.len();
        lease.release().await.unwrap();
        assigned
    }

    async fn merge(&self, survivor: &str, absorbed: &str, request: &str) -> Result<Value, OpsRepoError> {
        let command = MergeCommand {
            request_id: self.id(request),
            survivor_id: survivor.into(),
            survivor_version: self.version("OpsIncident", "incident_id", survivor).await,
            absorbed_id: absorbed.into(),
            absorbed_version: self.version("OpsIncident", "incident_id", absorbed).await,
            reason: "same family, two phones".into(),
        };
        self.repo.merge_incidents(&dispatcher(), &command).await.map(|o| o.outcome)
    }

    async fn active_vehicle_reservations(&self, incident: &str) -> i64 {
        self.value(
            "MATCH (r:Reservation {incident_id: $id, resource_kind: 'VEHICLE'}) WHERE r.active_vehicle_key IS NOT NULL \
             RETURN count(r) AS v",
            &[("id", incident.into())],
        )
        .await
    }

    async fn occupancy(&self) -> i64 {
        self.value("MATCH (s:Shelter {id: $d + '/S_D'}) RETURN s.current_occupancy AS v", &[]).await
    }

    async fn outbox(&self, kind: &str, aggregate: &str) -> i64 {
        self.value(
            "MATCH (e:OutboxEvent {type: $kind, aggregate_id: $id}) RETURN count(e) AS v",
            &[("kind", kind.into()), ("id", aggregate.into())],
        )
        .await
    }

    async fn cleanup(&self) {
        let ids: Vec<String> = self
            .value(
                "OPTIONAL MATCH (i:OpsIncident) WHERE i.incident_id STARTS WITH $d \
                    OR EXISTS { MATCH (i)-[:LOCATED_AT]->(:Junction {dataset: $d}) } \
                 OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) \
                 OPTIONAL MATCH (r:Report)-[:REPORTS]->(i) \
                 RETURN [x IN collect(DISTINCT i.incident_id) + collect(DISTINCT t.task_id) + collect(DISTINCT r.report_id) \
                     WHERE x IS NOT NULL] AS v",
                &[],
            )
            .await;
        for statement in [
            "MATCH (r:Reservation) WHERE r.incident_id IN $ids OR r.task_id IN $ids \
             OPTIONAL MATCH (r)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE r, a",
            "MATCH (a:AuditEvent) WHERE a.aggregate_id IN $ids DETACH DELETE a",
            "MATCH (e:OutboxEvent) WHERE e.aggregate_id IN $ids DELETE e",
            "MATCH (n) WHERE (n:Task AND n.task_id IN $ids) OR (n:OpsIncident AND n.incident_id IN $ids) \
                OR (n:Report AND n.report_id IN $ids) DETACH DELETE n",
        ] {
            self.neo4j.graph.run(query(statement).param("ids", ids.clone())).await.unwrap();
        }
        for statement in [
            "MATCH (c:Command) WHERE c.request_id STARTS WITH $d OR c.outcome_json CONTAINS $d DELETE c",
            "MATCH (v:Vehicle) WHERE v.vehicle_id STARTS WITH $d \
             OPTIONAL MATCH (v)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE v, a",
            "MATCH (a:AuditEvent) WHERE a.aggregate_id STARTS WITH $d DETACH DELETE a",
            "MATCH (e:OutboxEvent) WHERE e.aggregate_id STARTS WITH $d OR e.payload_json CONTAINS $d DELETE e",
            "MATCH (o:OverlayState {dataset: $d}) DELETE o",
            "MATCH (f:LeaseFence) WHERE f.name = 'matcher:' + $d DELETE f",
            "MATCH (s:Shelter {dataset: $d}) DETACH DELETE s",
            "MATCH (j:Junction {dataset: $d}) DETACH DELETE j",
        ] {
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
            crisis_graph::sim::heartbeat_key(&self.dataset),
        ] {
            let _: Result<(), _> = redis::cmd("DEL").arg(key).query_async(&mut conn).await;
        }
    }
}

/// Both assigned: the survivor keeps its vehicle, the absorbed vehicle and
/// beds are released, facts combine (largest headcount, raised urgency).
#[tokio::test]
async fn merge_two_assigned_keeps_the_survivors_vehicle_only() {
    let Some(ctx) = setup(2).await else { return };
    let survivor = ctx.incident("s", 1, 2, true).await;
    let absorbed = ctx.incident("a", 2, 4, true).await;
    let matcher = ctx.matcher().await;
    assert_eq!(ctx.match_once(&matcher).await, 2);
    assert_eq!(ctx.occupancy().await, 6);

    let outcome = ctx.merge(&survivor, &absorbed, "m1").await.unwrap();
    assert_eq!(outcome["plan"], "KEEP_SURVIVOR");
    assert_eq!(outcome["released_vehicles"].as_array().unwrap().len(), 1);
    assert_eq!((outcome["headcount"].as_i64(), outcome["urgency"].as_str()), (Some(4), Some("CRITICAL")));
    assert_eq!(ctx.status("OpsIncident", "incident_id", &absorbed).await, "MERGED");
    assert_eq!(ctx.status("Task", "task_id", &format!("{absorbed}/T")).await, "CANCELLED");
    assert_eq!(ctx.active_vehicle_reservations(&survivor).await, 1);
    assert_eq!(ctx.active_vehicle_reservations(&absorbed).await, 0);
    assert_eq!(ctx.occupancy().await, 2, "the absorbed incident's beds are returned");
    let available: i64 = ctx.value("MATCH (v:Vehicle {status: 'AVAILABLE'}) WHERE v.vehicle_id STARTS WITH $d RETURN count(v) AS v", &[]).await;
    assert_eq!(available, 1);
    let linked: i64 = ctx
        .value("MATCH (:OpsIncident {incident_id: $a})-[:MERGED_INTO]->(:OpsIncident {incident_id: $s}) RETURN count(*) AS v",
               &[("a", absorbed.clone().into()), ("s", survivor.clone().into())])
        .await;
    assert_eq!(linked, 1);
    let reports: i64 = ctx.value("MATCH (:Report)-[:REPORTS]->(:OpsIncident {incident_id: $s}) RETURN count(*) AS v", &[("s", survivor.clone().into())]).await;
    assert_eq!(reports, 2, "the absorbed report is now on the survivor too");
    assert_eq!(ctx.outbox("INCIDENT_MERGED", &absorbed).await, 1);
    let survivor_rank: i64 = ctx.value("MATCH (i:OpsIncident {incident_id: $s}) RETURN i.priority_rank AS v", &[("s", survivor.clone().into())]).await;
    assert_eq!(survivor_rank, 2);
    ctx.cleanup().await;
}

/// Only the absorbed incident has a vehicle: the assignment moves to the
/// survivor, whose own queued task is cancelled.
#[tokio::test]
async fn merge_moves_an_assigned_vehicle_to_a_queued_survivor() {
    let Some(ctx) = setup(1).await else { return };
    let absorbed = ctx.incident("a", 1, 2, true).await;
    let matcher = ctx.matcher().await;
    assert_eq!(ctx.match_once(&matcher).await, 1);
    let survivor = ctx.incident("s", 1, 3, true).await;

    let outcome = ctx.merge(&survivor, &absorbed, "m2").await.unwrap();
    assert_eq!(outcome["plan"], "MOVE_ABSORBED");
    assert_eq!(outcome["released_vehicles"].as_array().unwrap().len(), 0, "a sent vehicle is never recalled");
    assert_eq!(ctx.status("Task", "task_id", &format!("{survivor}/T")).await, "CANCELLED");
    let moved: String = ctx
        .value("MATCH (:OpsIncident {incident_id: $s})-[:HAS_TASK]->(t:Task {status: 'ASSIGNED'}) RETURN t.task_id AS v", &[("s", survivor.clone().into())])
        .await;
    assert_eq!(moved, format!("{absorbed}/T"));
    assert_eq!(ctx.active_vehicle_reservations(&survivor).await, 1);
    let incident: Value = ctx
        .value("MATCH (i:OpsIncident {incident_id: $s}) RETURN {status: i.status, headcount: i.headcount} AS v", &[("s", survivor.clone().into())])
        .await;
    assert_eq!(incident, json!({ "status": "ASSIGNED", "headcount": 3 }));
    // Nothing for the matcher to do: one incident, one vehicle.
    assert_eq!(ctx.match_once(&matcher).await, 0);
    ctx.cleanup().await;
}

/// Queued + queued: one task remains; the matcher sends one vehicle.
/// No task + queued: the queued task moves to the survivor.
#[tokio::test]
async fn merge_of_queued_incidents_leaves_one_task() {
    let Some(ctx) = setup(2).await else { return };
    let survivor = ctx.incident("s", 1, 2, true).await;
    let absorbed = ctx.incident("a", 1, 2, true).await;
    assert_eq!(ctx.merge(&survivor, &absorbed, "m3").await.unwrap()["plan"], "KEEP_SURVIVOR");
    let matcher = ctx.matcher().await;
    assert_eq!(ctx.match_once(&matcher).await, 1, "one vehicle for the merged incident");

    let empty = ctx.incident("e", 1, 1, false).await;
    let queued = ctx.incident("q", 1, 1, true).await;
    assert_eq!(ctx.merge(&empty, &queued, "m4").await.unwrap()["plan"], "MOVE_ABSORBED");
    let tasks: i64 = ctx.value("MATCH (:OpsIncident {incident_id: $s})-[:HAS_TASK]->(t:Task {status: 'QUEUED'}) RETURN count(t) AS v", &[("s", empty.clone().into())]).await;
    assert_eq!(tasks, 1);
    ctx.cleanup().await;
}

/// Merges that must not happen, a stale version, a wrong role and a replay.
#[tokio::test]
async fn merge_refusals_and_replay() {
    let Some(ctx) = setup(1).await else { return };
    let one = ctx.incident("1", 1, 2, true).await;
    let two = ctx.incident("2", 1, 2, true).await;
    let review = ctx.id("I-review");
    ctx.run(
        "MATCH (j:Junction {id: $d + '/JD', dataset: $d}) CREATE (i:OpsIncident {incident_id: $id, status: 'REVIEW_REQUIRED', \
             version: 0, priority_rank: 1, urgency: 'UNKNOWN', created_at: datetime()})-[:LOCATED_AT]->(j)",
        &[("id", review.clone().into())],
    )
    .await;

    assert!(matches!(ctx.merge(&one, &one, "r1").await, Err(OpsRepoError::InvalidData(_))));
    assert!(matches!(ctx.merge(&review, &one, "r2").await, Err(OpsRepoError::InvalidData(_))), "merge into the approved incident");
    let stale = MergeCommand {
        request_id: ctx.id("r3"),
        survivor_id: one.clone(),
        survivor_version: 7,
        absorbed_id: two.clone(),
        absorbed_version: 0,
        reason: "x".into(),
    };
    assert!(matches!(ctx.repo.merge_incidents(&dispatcher(), &stale).await, Err(OpsRepoError::NotAvailable(_))));
    let observer = ActorContext::new("obs", [ActorRole::Observer]);
    assert!(matches!(ctx.repo.merge_incidents(&observer, &MergeCommand { survivor_version: 0, ..stale.clone() }).await, Err(OpsRepoError::Forbidden(_))));

    let first = ctx.merge(&one, &two, "r4").await.unwrap();
    let replay = MergeCommand {
        request_id: ctx.id("r4"),
        survivor_id: one.clone(),
        survivor_version: 0,
        absorbed_id: two.clone(),
        absorbed_version: 0,
        reason: "same family".into(),
    };
    let again = ctx.repo.merge_incidents(&dispatcher(), &replay).await.unwrap();
    assert!(again.replayed);
    assert_eq!(again.outcome, first);
    // A merged incident cannot be merged again.
    assert!(matches!(ctx.merge(&review, &two, "r5").await, Err(OpsRepoError::InvalidData(_))));
    ctx.cleanup().await;
}

/// Split: the follow-up becomes its own review incident; the old incident
/// loses the report and recomputes its headcount; the first report stays.
#[tokio::test]
async fn split_moves_a_follow_up_into_a_review_incident() {
    let Some(ctx) = setup(1).await else { return };
    let incident = ctx.incident("1", 1, 5, true).await;
    let follow_up = format!("{incident}/R2");
    ctx.run(
        "MATCH (i:OpsIncident {incident_id: $id}) \
         CREATE (:Report {report_id: $r, request_id: $r, device_id: $id, headcount: 5, required_asset: 'AMBULANCE', \
             resolved_junction_id: $d + '/JD', urgency: 'URGENT', priority_rank: 1, raw_text: 'another family, 5 people', \
             received_at: datetime(), extraction_status: 'EXTRACTED'})-[:REPORTS {kind: 'FOLLOW_UP', rule: 'D-2'}]->(i) \
         SET i.report_count = 2",
        &[("id", incident.clone().into()), ("r", follow_up.clone().into())],
    )
    .await;
    ctx.run("MATCH (r:Report {report_id: $r}) SET r.headcount = 2", &[("r", format!("{incident}/R1").into())]).await;

    let primary = SplitCommand {
        request_id: ctx.id("s0"),
        incident_id: incident.clone(),
        expected_version: 0,
        report_id: format!("{incident}/R1"),
        reason: "not the same family".into(),
    };
    assert!(matches!(ctx.repo.split_report(&dispatcher(), &primary).await, Err(OpsRepoError::InvalidData(_))));

    let split = SplitCommand { request_id: ctx.id("s1"), report_id: follow_up.clone(), ..primary };
    let outcome = ctx.repo.split_report(&dispatcher(), &split).await.unwrap().outcome;
    let new_id = outcome["new_incident_id"].as_str().unwrap().to_string();
    assert_eq!((outcome["headcount"].as_i64(), outcome["remaining_reports"].as_i64()), (Some(2), Some(1)));
    let created: Value = ctx
        .value("MATCH (r:Report)-[l:REPORTS]->(n:OpsIncident {incident_id: $n}) \
                RETURN {status: n.status, headcount: n.headcount, link: l.kind, report: r.report_id, \
                        located: EXISTS { MATCH (n)-[:LOCATED_AT]->() }} AS v",
               &[("n", new_id.clone().into())])
        .await;
    assert_eq!(created, json!({ "status": "REVIEW_REQUIRED", "headcount": 5, "link": "PRIMARY", "report": follow_up, "located": true }));
    let still_linked: i64 = ctx
        .value("MATCH (:Report {report_id: $r})-[:REPORTS]->(:OpsIncident {incident_id: $i}) RETURN count(*) AS v",
               &[("r", follow_up.clone().into()), ("i", incident.clone().into())])
        .await;
    assert_eq!(still_linked, 0);
    assert_eq!(ctx.outbox("INCIDENT_REVIEW_REQUIRED", &new_id).await, 1);
    ctx.cleanup().await;
}

/// Out of service and back; a breakdown re-queues the task and frees its
/// beds, and the matcher sends the other vehicle; an out-of-service vehicle
/// is never matched.
#[tokio::test]
async fn vehicle_breakdown_requeues_and_another_vehicle_is_sent() {
    let Some(ctx) = setup(2).await else { return };
    let first = ctx.id("AMB-1");
    let second = ctx.id("AMB-2");
    let set = |vehicle: String, out: bool, request: &str, version: u64| VehicleStatusCommand {
        request_id: ctx.id(request),
        vehicle_id: vehicle,
        expected_version: version,
        out_of_service: out,
        reason: "test".into(),
    };
    // AMB-2 out of service: AMB-1 must take the task.
    ctx.repo.set_vehicle_status(&dispatcher(), &set(second.clone(), true, "v1", 0)).await.unwrap();
    assert!(matches!(
        ctx.repo.set_vehicle_status(&dispatcher(), &set(second.clone(), true, "v1b", 1)).await,
        Err(OpsRepoError::InvalidData(_))
    ));
    let incident = ctx.incident("1", 1, 2, true).await;
    let matcher = ctx.matcher().await;
    assert_eq!(ctx.match_once(&matcher).await, 1);
    let assigned: String = ctx.value("MATCH (t:Task {task_id: $t}) RETURN t.assigned_vehicle_id AS v", &[("t", format!("{incident}/T").into())]).await;
    assert_eq!(assigned, first);
    assert_eq!(ctx.occupancy().await, 2);

    // AMB-1 breaks down on the way; AMB-2 is back in service.
    let version = ctx.version("Vehicle", "vehicle_id", &first).await;
    let breakdown = ctx.repo.set_vehicle_status(&dispatcher(), &set(first.clone(), true, "v2", version)).await.unwrap().outcome;
    assert_eq!(breakdown["requeued_task"], json!(format!("{incident}/T")));
    assert_eq!(ctx.status("Task", "task_id", &format!("{incident}/T")).await, "QUEUED");
    assert_eq!(ctx.occupancy().await, 0, "the beds wait for the new assignment");
    assert_eq!(ctx.match_once(&matcher).await, 0, "no vehicle in service");
    ctx.repo.set_vehicle_status(&dispatcher(), &set(second.clone(), false, "v3", 1)).await.unwrap();
    assert_eq!(ctx.match_once(&matcher).await, 1);
    let reassigned: String = ctx.value("MATCH (t:Task {task_id: $t}) RETURN t.assigned_vehicle_id AS v", &[("t", format!("{incident}/T").into())]).await;
    assert_eq!(reassigned, second);
    assert_eq!(ctx.status("Vehicle", "vehicle_id", &first).await, "OUT_OF_SERVICE");
    ctx.cleanup().await;
}

/// The simulated driver completes due missions through T5, exactly once
/// even with two drivers, and a new driver picks up existing assignments.
#[tokio::test]
async fn simulated_driver_completes_missions_exactly_once() {
    let Some(ctx) = setup(2).await else { return };
    let one = ctx.incident("1", 1, 2, true).await;
    let two = ctx.incident("2", 1, 2, true).await;
    let matcher = ctx.matcher().await;
    assert_eq!(ctx.match_once(&matcher).await, 2);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let a = SimDriver::new(ctx.repo.clone(), ctx.redis.clone(), ctx.params.clone());
    let b = SimDriver::new(ctx.repo.clone(), ctx.redis.clone(), ctx.params.clone());
    let now = Utc::now();
    let (left, right) = tokio::join!(a.run_once(now), b.run_once(now));
    let completed = left.unwrap().len() + right.unwrap().len();
    assert_eq!(completed, 2, "each task completed once across both drivers");
    for incident in [&one, &two] {
        assert_eq!(ctx.status("Task", "task_id", &format!("{incident}/T")).await, "COMPLETED");
    }
    let completions: i64 = ctx
        .value("MATCH (a:AuditEvent {action: 'TASK_COMPLETED', actor_id: 'sim-driver'}) WHERE a.aggregate_id STARTS WITH $d RETURN count(a) AS v", &[])
        .await;
    assert_eq!(completions, 2);
    let available: i64 = ctx.value("MATCH (v:Vehicle {status: 'AVAILABLE', junction_id: $d + '/JD'}) WHERE v.vehicle_id STARTS WITH $d RETURN count(v) AS v", &[]).await;
    assert_eq!(available, 2, "both vehicles wait at the drop-off");
    assert!(a.run_once(Utc::now()).await.unwrap().is_empty());
    ctx.cleanup().await;
}

/// The routes exist on the console API and check the operator's role.
#[tokio::test]
async fn http_routes_check_tokens_and_roles() {
    let Some(ctx) = setup(1).await else { return };
    let spec = format!("disp:DISPATCHER:{},obs:OBSERVER:{}", token_hash_hex("disp-token"), token_hash_hex("obs-token"));
    let (operators, _) = OperatorTokens::parse(&spec);
    let app = console_routes(ConsoleApiState {
        repo: ctx.repo.clone(),
        state: StateManager::new(ctx.neo4j.clone()),
        redis: ctx.redis.clone(),
        operators: Arc::new(operators),
        params: ctx.params.clone(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = reqwest::Client::new();
    let vehicle = ctx.id("AMB-1").replace('/', "%2F");
    let body = json!({ "request_id": ctx.id("h1"), "status": "OUT_OF_SERVICE", "expected_version": 0, "reason": "flat tyre" });
    let send = |token: Option<&'static str>| {
        let mut request = http.post(format!("{base}/api/v1/vehicles/{vehicle}/status")).json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send()
    };
    assert_eq!(send(None).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(send(Some("obs-token")).await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(send(Some("disp-token")).await.unwrap().status(), StatusCode::OK);
    let one = ctx.incident("1", 1, 2, true).await.replace('/', "%2F");
    let merge = http
        .post(format!("{base}/api/v1/incidents/{one}/merge"))
        .bearer_auth("disp-token")
        .json(&json!({ "request_id": ctx.id("h2"), "absorbed_id": "missing" }))
        .send()
        .await
        .unwrap();
    assert_eq!(merge.status(), StatusCode::UNPROCESSABLE_ENTITY, "versions are required");
    let (_, system) = (0, http.get(format!("{base}/api/v1/system")).send().await.unwrap().json::<Value>().await.unwrap());
    assert_eq!(system["simulated_driver"], false);
    ctx.cleanup().await;
}
