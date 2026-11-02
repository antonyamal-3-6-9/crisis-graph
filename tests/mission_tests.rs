//! M6b acceptance tests: boat-then-road missions. Live Neo4j + Redis
//! (skipped if unreachable).
//!
//! Each test builds its own network in a dataset named `TEST-<uuid>`:
//!
//! ```text
//!   A --ab (60 s)--> B ==bd (flooded)==> D --df (60 s)--> F ==fe (flooded)==> E
//!   A --ac (300 s)-> C ==cd (flooded)==> D
//!   B --ba (60 s)--> A
//!   depot and drop-off shelter S_A at A (50 beds): ambulances, a truck, a boat
//! ```
//!
//! Victims at D are cut off by water: the boat goes by road to the staging
//! point B and crosses `bd`. Victims at E need two separate water crossings
//! (a fragmented cut). Victims at B are reachable by road.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use neo4rs::query;
use serde_json::Value;
use uuid::Uuid;

use crisis_graph::config::{Config, PipelineParams};
use crisis_graph::db::{
    ensure_ops_schema, CancelMissionCommand, CompleteTaskCommand, Neo4jClient, OpsRepoError, OpsRepository,
    RedisClient, StateManager, TriggerCommand,
};
use crisis_graph::graph::GraphSnapshot;
use crisis_graph::matcher::{CycleReport, Matcher};
use crisis_graph::models::{ActorContext, ActorRole, HazardCommand, HazardSource, HazardStatus};
use crisis_graph::sim::SimDriver;

struct Ctx {
    neo4j: Neo4jClient,
    redis: RedisClient,
    repo: OpsRepository,
    dataset: String,
    params: PipelineParams,
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
        sim_time_scale: 0.0001,
        sim_on_scene_secs: 1,
        boat_speed_kmh: 8.0,
        ..PipelineParams::default()
    };
    let ctx = Ctx { repo: OpsRepository::new(neo4j.clone()), neo4j, redis, dataset, params };
    ctx.build().await;
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

    async fn build(&self) {
        let junctions = [
            ("A", 10.00, 76.00), ("B", 10.00, 76.01), ("C", 10.01, 76.00), ("D", 10.01, 76.01),
            ("F", 10.02, 76.01), ("E", 10.03, 76.01),
        ];
        for (name, lat, lon) in junctions {
            self.run(
                "CREATE (:Junction {id: $d + '/J' + $name, dataset: $d, lat: $lat, lon: $lon, \
                     location: point({latitude: $lat, longitude: $lon})})",
                &[("name", name.into()), ("lat", lat.into()), ("lon", lon.into())],
            )
            .await;
        }
        let segments = [
            ("ab", "A", "B", 60.0), ("bd", "B", "D", 60.0), ("ac", "A", "C", 300.0), ("cd", "C", "D", 300.0),
            ("ba", "B", "A", 60.0), ("df", "D", "F", 60.0), ("fe", "F", "E", 60.0),
        ];
        for (name, from, to, seconds) in segments {
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
            "CREATE (:Shelter {id: $d + '/S_A', name: 'test depot and drop-off', dataset: $d, \
                 junction_id: $d + '/JA', capacity: 50, current_occupancy: 0})",
            &[],
        )
        .await;
        for (name, asset, seats) in [("AMB-1", "AMBULANCE", 4), ("AMB-2", "AMBULANCE", 4), ("TRK-1", "EVAC_TRUCK", 20), ("BOAT-1", "RESCUE_BOAT", 8)] {
            self.run(
                "MATCH (s:Shelter {id: $d + '/S_A'}) \
                 CREATE (v:Vehicle {vehicle_id: $d + '/' + $name, asset_type: $asset, org: 'test', \
                     capacity_persons: $seats, status: 'AVAILABLE', version: 0, home_shelter_id: s.id, \
                     junction_id: $d + '/JA', created_at: datetime(), updated_at: datetime()})-[:BASED_AT]->(s)",
                &[("name", name.into()), ("asset", asset.into()), ("seats", (seats as i64).into())],
            )
            .await;
        }
        for segment in ["bd", "cd", "fe"] {
            self.repo
                .apply_hazard(
                    &ActorContext::new("verifier", [ActorRole::HazardVerifier]),
                    &HazardCommand {
                        request_id: self.id(&format!("hz-{segment}")),
                        dataset: self.dataset.clone(),
                        segment_id: self.id(segment),
                        status: HazardStatus::Flooded,
                        valid_until: Some(Utc::now() + chrono::Duration::hours(2)),
                        reason: "river over the road".into(),
                        source: HazardSource::Operator,
                    },
                )
                .await
                .unwrap();
        }
    }

    /// An approved incident at `junction` with a queued road pickup.
    async fn incident(&self, name: &str, junction: &str, headcount: i64, asset: &str) -> String {
        let id = self.id(&format!("I-{name}"));
        self.run(
            "MATCH (j:Junction {id: $d + '/J' + $junction, dataset: $d}) \
             CREATE (i:OpsIncident {incident_id: $id, status: 'TRIAGE_APPROVED', version: 0, headcount: $headcount, \
                 priority_rank: 1, urgency: 'URGENT', required_asset: $asset, junction_id: j.id, report_count: 1, \
                 created_at: datetime(), updated_at: datetime()}) \
             CREATE (i)-[:LOCATED_AT]->(j) \
             CREATE (i)-[:HAS_TASK {seq: 1}]->(:Task {task_id: $id + '/T', incident_id: $id, kind: 'ROAD_PICKUP', \
                 status: 'QUEUED', required_asset: $asset, urgency: 'URGENT', priority_rank: 1, version: 0, \
                 created_at: datetime(), queued_at: datetime()})",
            &[("id", id.clone().into()), ("junction", junction.into()), ("headcount", headcount.into()), ("asset", asset.into())],
        )
        .await;
        id
    }

    async fn matcher(&self) -> Matcher {
        let snapshot = GraphSnapshot::load(self.repo.clone(), &StateManager::new(self.neo4j.clone()), &self.dataset)
            .await
            .expect("snapshot");
        Matcher::new(self.repo.clone(), self.redis.clone(), StateManager::new(self.neo4j.clone()), Arc::new(snapshot), self.params.clone())
    }

    async fn cycle(&self, matcher: &Matcher) -> CycleReport {
        let mut lease = matcher.lease();
        let token = lease.try_acquire().await.unwrap().expect("matcher lease");
        matcher.sync_queue().await.unwrap();
        let report = matcher.run_cycle(token).await.unwrap();
        lease.release().await.unwrap();
        report
    }

    /// `[task_id, kind, status, required_asset, version]` per task, oldest first.
    async fn tasks(&self, incident: &str) -> Vec<Vec<Value>> {
        self.value(
            "MATCH (:OpsIncident {incident_id: $id})-[r:HAS_TASK]->(t:Task) WITH t ORDER BY r.seq \
             RETURN collect([t.task_id, t.kind, t.status, t.required_asset, t.version]) AS v",
            &[("id", incident.into())],
        )
        .await
    }

    async fn task_of(&self, incident: &str, kind: &str) -> (String, String, u64) {
        let task = self
            .tasks(incident)
            .await
            .into_iter()
            .find(|t| t[1] == kind && t[2] != "CANCELLED")
            .unwrap_or_else(|| panic!("no live {kind} task"));
        (task[0].as_str().unwrap().into(), task[2].as_str().unwrap().into(), task[4].as_u64().unwrap())
    }

    async fn complete(&self, task: &(String, String, u64), request: &str) -> Result<Value, OpsRepoError> {
        let command = CompleteTaskCommand { request_id: self.id(request), task_id: task.0.clone(), expected_version: task.2, note: None };
        self.repo.complete_task(&dispatcher(), &command).await.map(|o| o.outcome)
    }

    async fn cleanup(&self) {
        let ids: Vec<String> = self
            .value(
                "OPTIONAL MATCH (i:OpsIncident) WHERE i.incident_id STARTS WITH $d \
                 OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) \
                 RETURN [x IN collect(DISTINCT i.incident_id) + collect(DISTINCT t.task_id) WHERE x IS NOT NULL] AS v",
                &[],
            )
            .await;
        for statement in [
            "MATCH (r:Reservation) WHERE r.incident_id IN $ids OR r.task_id IN $ids \
             OPTIONAL MATCH (r)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE r, a",
            "MATCH (a:AuditEvent) WHERE a.aggregate_id IN $ids DETACH DELETE a",
            "MATCH (e:OutboxEvent) WHERE e.aggregate_id IN $ids DELETE e",
            "MATCH (n) WHERE (n:Task AND n.task_id IN $ids) OR (n:OpsIncident AND n.incident_id IN $ids) DETACH DELETE n",
        ] {
            self.neo4j.graph.run(query(statement).param("ids", ids.clone())).await.unwrap();
        }
        for statement in [
            "MATCH (c:Command) WHERE c.request_id STARTS WITH $d OR c.outcome_json CONTAINS $d DELETE c",
            "MATCH (v:Vehicle) WHERE v.vehicle_id STARTS WITH $d \
             OPTIONAL MATCH (v)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE v, a",
            "MATCH (a:AuditEvent) WHERE a.aggregate_id STARTS WITH $d DETACH DELETE a",
            "MATCH (e:OutboxEvent) WHERE e.aggregate_id STARTS WITH $d OR e.payload_json CONTAINS $d DELETE e",
            "MATCH (h:Hazard {dataset: $d}) OPTIONAL MATCH (h)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE h, a",
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
        ] {
            let _: Result<(), _> = redis::cmd("DEL").arg(key).query_async(&mut conn).await;
        }
    }
}

/// A cut-off pickup converts exactly once into a queued boat task and a
/// waiting road task of the right type; reachable and declined incidents
/// never convert.
#[tokio::test]
async fn cut_off_task_converts_once_with_the_right_road_vehicle() {
    let Some(ctx) = setup().await else { return };
    let cut_off = ctx.incident("cut", "D", 3, "AMBULANCE").await;
    let crowd = ctx.incident("crowd", "D", 12, "RESCUE_BOAT").await;
    let reachable = ctx.incident("dry", "B", 2, "AMBULANCE").await;
    let declined = ctx.incident("declined", "D", 2, "AMBULANCE").await;
    ctx.run("MATCH (i:OpsIncident {incident_id: $id}) SET i.boat_plan_declined = true", &[("id", declined.clone().into())]).await;
    let matcher = ctx.matcher().await;

    let report = ctx.cycle(&matcher).await;
    assert_eq!(report.converted, 2, "{report:?}");
    let tasks = ctx.tasks(&cut_off).await;
    let kinds: Vec<(String, String, String)> = tasks
        .iter()
        .map(|t| (t[1].as_str().unwrap().into(), t[2].as_str().unwrap().into(), t[3].as_str().unwrap().into()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("ROAD_PICKUP".into(), "CANCELLED".into(), "AMBULANCE".into()),
            ("WATER_EXTRACTION".into(), "QUEUED".into(), "RESCUE_BOAT".into()),
            ("ROAD_EVACUATION".into(), "PENDING_TRIGGER".into(), "AMBULANCE".into()),
        ]
    );
    let staging: String = ctx.value("MATCH (i:OpsIncident {incident_id: $id}) RETURN i.staging_junction_id AS v", &[("id", cut_off.clone().into())]).await;
    assert_eq!(staging, ctx.id("JB"));
    let road_for_crowd = ctx.task_of(&crowd, "ROAD_EVACUATION").await;
    let crowd_asset: String = ctx.value("MATCH (t:Task {task_id: $t}) RETURN t.required_asset AS v", &[("t", road_for_crowd.0.into())]).await;
    assert_eq!(crowd_asset, "EVAC_TRUCK", "12 people, no medical need, only a boat requested");
    let alerts: i64 = ctx.value("MATCH (e:OutboxEvent {type: 'MISSION_CONVERTED'}) WHERE e.aggregate_id IN [$a, $b] RETURN count(e) AS v",
        &[("a", cut_off.clone().into()), ("b", crowd.clone().into())]).await;
    assert_eq!(alerts, 2);
    assert_eq!(ctx.tasks(&declined).await.len(), 1, "a declined plan is never converted");
    assert_eq!(ctx.task_of(&reachable, "ROAD_PICKUP").await.1, "ASSIGNED");

    let again = ctx.cycle(&matcher).await;
    assert_eq!(again.converted, 0);
    assert_eq!(ctx.tasks(&cut_off).await.len(), 3, "converted once");
    // The 12 people do not fit the boat (8 seats): escalated, not assigned.
    let crowd_boat = ctx.task_of(&crowd, "WATER_EXTRACTION").await;
    let escalations: Vec<String> = ctx.value("MATCH (t:Task {task_id: $t}) RETURN coalesce(t.escalations, []) AS v", &[("t", crowd_boat.0.into())]).await;
    assert_eq!(escalations, vec!["NO_ELIGIBLE_VEHICLE".to_string()]);
    ctx.cleanup().await;
}

/// Two separate water crossings: no surface plan, escalated once.
#[tokio::test]
async fn fragmented_cut_escalates_no_surface_route() {
    let Some(ctx) = setup().await else { return };
    let island = ctx.incident("island", "E", 2, "AMBULANCE").await;
    let matcher = ctx.matcher().await;
    assert_eq!(ctx.cycle(&matcher).await.converted, 0);
    ctx.cycle(&matcher).await;
    let escalations: Vec<String> = ctx.value("MATCH (t:Task {task_id: $t}) RETURN coalesce(t.escalations, []) AS v", &[("t", format!("{island}/T").into())]).await;
    assert_eq!(escalations, vec!["NO_SURFACE_ROUTE".to_string()]);
    ctx.cleanup().await;
}

/// The boat is sent by road to the staging point (no beds, water leg
/// unverified); "victim aboard" queues the road leg, which goes to the
/// staging point with drop-off beds; both legs complete the incident.
#[tokio::test]
async fn boat_then_road_runs_through_trigger_and_completion() {
    let Some(ctx) = setup().await else { return };
    let incident = ctx.incident("1", "D", 3, "AMBULANCE").await;
    let matcher = ctx.matcher().await;
    assert_eq!(ctx.cycle(&matcher).await.converted, 1);
    let report = ctx.cycle(&matcher).await;
    assert_eq!(report.assigned.len(), 1, "{report:?}");
    assert_eq!(report.dropoff_unresolved, 0, "a boat never needs beds");
    let boat = ctx.task_of(&incident, "WATER_EXTRACTION").await;
    assert_eq!(boat.1, "ASSIGNED");
    let eta: f64 = ctx.value("MATCH (t:Task {task_id: $t}) RETURN t.eta_s AS v", &[("t", boat.0.clone().into())]).await;
    assert!((eta - (60.0 + 450.0)).abs() < 1.0, "road A->B 60 s + 1 km of water at 8 km/h: {eta}");
    let payload: String = ctx.value("MATCH (e:OutboxEvent {type: 'TASK_ASSIGNED', aggregate_id: $t}) RETURN e.payload_json AS v", &[("t", boat.0.clone().into())]).await;
    assert!(payload.contains("UNVERIFIED_ESTIMATE"), "{payload}");
    let beds: i64 = ctx.value("MATCH (s:Shelter {id: $d + '/S_A'}) RETURN s.current_occupancy AS v", &[]).await;
    assert_eq!(beds, 0);

    // Victim aboard, on the boat task.
    let aboard = TriggerCommand { request_id: ctx.id("aboard"), task_id: boat.0.clone(), expected_version: boat.2, reason: "VICTIM_ABOARD".into() };
    let outcome = ctx.repo.trigger_road_leg(&dispatcher(), &aboard).await.unwrap();
    assert_eq!(outcome.outcome["triggered_tasks"].as_array().unwrap().len(), 1);
    assert!(ctx.repo.trigger_road_leg(&dispatcher(), &aboard).await.unwrap().replayed);
    let late = TriggerCommand { request_id: ctx.id("aboard-2"), ..aboard.clone() };
    assert!(matches!(ctx.repo.trigger_road_leg(&dispatcher(), &late).await, Err(OpsRepoError::NotAvailable(_))));

    let report = ctx.cycle(&matcher).await;
    assert_eq!(report.assigned.len(), 1, "{report:?}");
    let road = ctx.task_of(&incident, "ROAD_EVACUATION").await;
    assert_eq!(road.1, "ASSIGNED");
    let route: String = ctx.value("MATCH (t:Task {task_id: $t}) RETURN t.route_json AS v", &[("t", road.0.clone().into())]).await;
    assert!(route.contains(&ctx.id("ab")), "the ambulance drives to the staging point B: {route}");
    let beds: i64 = ctx.value("MATCH (s:Shelter {id: $d + '/S_A'}) RETURN s.current_occupancy AS v", &[]).await;
    assert_eq!(beds, 3, "the road leg reserves the drop-off beds");

    let done = ctx.complete(&boat, "boat-done").await.unwrap();
    assert_eq!(done["vehicle_position_junction"], ctx.id("JB"), "the boat waits at the staging point");
    assert_eq!(done["incident_status"], "ASSIGNED", "the road leg is still out");
    let done = ctx.complete(&road, "road-done").await.unwrap();
    assert_eq!(done["incident_status"], "COMPLETED");
    ctx.cleanup().await;
}

/// Completing the boat without "victim aboard" queues the road leg; the
/// road task itself can also be sent early.
#[tokio::test]
async fn completing_the_boat_queues_a_pending_road_leg() {
    let Some(ctx) = setup().await else { return };
    let incident = ctx.incident("1", "D", 3, "AMBULANCE").await;
    let matcher = ctx.matcher().await;
    ctx.cycle(&matcher).await;
    ctx.cycle(&matcher).await;

    let boat = ctx.task_of(&incident, "WATER_EXTRACTION").await;
    assert_eq!(boat.1, "ASSIGNED");
    let done = ctx.complete(&boat, "done").await.unwrap();
    assert_eq!(done["triggered_tasks"].as_array().unwrap().len(), 1);
    assert_eq!(ctx.task_of(&incident, "ROAD_EVACUATION").await.1, "QUEUED");

    // A second cut-off incident: its road leg is sent early by a dispatcher.
    let early = ctx.incident("2", "D", 2, "AMBULANCE").await;
    ctx.cycle(&matcher).await;

    let road = ctx.task_of(&early, "ROAD_EVACUATION").await;
    assert_eq!(road.1, "PENDING_TRIGGER");
    let now = TriggerCommand { request_id: ctx.id("early"), task_id: road.0.clone(), expected_version: road.2, reason: "EARLY".into() };
    ctx.repo.trigger_road_leg(&dispatcher(), &now).await.unwrap();
    assert_eq!(ctx.task_of(&early, "ROAD_EVACUATION").await.1, "QUEUED");
    let observer = ActorContext::new("obs", [ActorRole::Observer]);
    let refused = TriggerCommand { request_id: ctx.id("obs"), ..now };
    assert!(matches!(ctx.repo.trigger_road_leg(&observer, &refused).await, Err(OpsRepoError::Forbidden(_))));
    ctx.cleanup().await;
}

/// Cancelling the boat plan releases the boat, restores a road pickup and
/// stops further conversion.
#[tokio::test]
async fn cancelling_the_boat_plan_restores_a_road_pickup() {
    let Some(ctx) = setup().await else { return };
    let incident = ctx.incident("1", "D", 3, "AMBULANCE").await;
    let matcher = ctx.matcher().await;
    ctx.cycle(&matcher).await;
    ctx.cycle(&matcher).await;
    assert_eq!(ctx.task_of(&incident, "WATER_EXTRACTION").await.1, "ASSIGNED");

    let version: i64 = ctx.value("MATCH (i:OpsIncident {incident_id: $id}) RETURN i.version AS v", &[("id", incident.clone().into())]).await;
    let cancel = CancelMissionCommand { request_id: ctx.id("cancel"), incident_id: incident.clone(), expected_version: version as u64, reason: "water too fast for the boat".into() };
    let outcome = ctx.repo.cancel_mission(&dispatcher(), &cancel).await.unwrap().outcome;
    assert_eq!(outcome["released_vehicles"].as_array().unwrap().len(), 1);
    let boat_status: String = ctx.value("MATCH (v:Vehicle {vehicle_id: $d + '/BOAT-1'}) RETURN v.status AS v", &[]).await;
    assert_eq!(boat_status, "AVAILABLE");
    assert_eq!(ctx.task_of(&incident, "ROAD_PICKUP").await.1, "QUEUED");
    assert_eq!(ctx.cycle(&matcher).await.converted, 0, "never converted again");
    ctx.cleanup().await;
}

/// The simulated driver runs the mission: victim aboard, boat back, road
/// leg sent and completed.
#[tokio::test]
async fn simulated_driver_runs_a_boat_then_road_mission() {
    let Some(ctx) = setup().await else { return };
    let incident = ctx.incident("1", "D", 3, "AMBULANCE").await;
    let matcher = ctx.matcher().await;
    ctx.cycle(&matcher).await;
    ctx.cycle(&matcher).await;
    let driver = SimDriver::new(ctx.repo.clone(), ctx.redis.clone(), ctx.params.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(driver.run_once(Utc::now()).await.unwrap().len(), 1, "boat leg done");
    let trigger: String = ctx.value("MATCH (t:Task {kind: 'ROAD_EVACUATION'}) WHERE t.incident_id = $id RETURN t.trigger AS v", &[("id", incident.clone().into())]).await;
    assert_eq!(trigger, "VICTIM_ABOARD", "the crew reported the victims aboard before landing");
    assert_eq!(ctx.cycle(&matcher).await.assigned.len(), 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(driver.run_once(Utc::now()).await.unwrap().len(), 1, "road leg done");
    let status: String = ctx.value("MATCH (i:OpsIncident {incident_id: $id}) RETURN i.status AS v", &[("id", incident.clone().into())]).await;
    assert_eq!(status, "COMPLETED");
    ctx.cleanup().await;
}
