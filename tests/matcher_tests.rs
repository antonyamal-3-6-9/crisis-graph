//! M5 acceptance tests: the matcher and allocation (T2/T3). Live Neo4j +
//! Redis (skipped if unreachable).
//!
//! Each test builds its own network, depot, drop-off shelter, vehicles and
//! tasks in a dataset named `TEST-<uuid>`, so real tasks and the real fleet
//! are never touched:
//!
//! ```text
//!   A --ab (60 s)--> B --bd (60 s)--> D      fast
//!   A --ac (300 s)-> C --cd (300 s)-> D      slow detour
//!   depot S_A at A (no beds), drop-off shelter S_D at D
//! ```

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use neo4rs::query;
use uuid::Uuid;

use crisis_graph::config::{Config, PipelineParams};
use crisis_graph::db::{ensure_ops_schema, Neo4jClient, OpsRepoError, OpsRepository, RedisClient, StateManager};
use crisis_graph::graph::GraphSnapshot;
use crisis_graph::matcher::Matcher;
use crisis_graph::models::{ActorContext, ActorRole, HazardCommand, HazardSource, HazardStatus};
use crisis_graph::runtime::Shutdown;

struct Ctx {
    neo4j: Neo4jClient,
    redis: RedisClient,
    repo: OpsRepository,
    dataset: String,
}

async fn setup(dropoff_beds: i64) -> Option<Ctx> {
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
    ctx.build(dropoff_beds).await;
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

    async fn string(&self, cypher: &str) -> Option<String> {
        let mut result = self.neo4j.graph.execute(query(cypher).param("d", self.dataset.clone())).await.unwrap();
        result.next().await.unwrap().and_then(|row| row.get::<Option<String>>("v").unwrap())
    }

    async fn build(&self, dropoff_beds: i64) {
        for (name, lat, lon) in [("A", 10.00, 76.00), ("B", 10.00, 76.01), ("C", 10.01, 76.00), ("D", 10.01, 76.01)] {
            self.run(
                "CREATE (:Junction {id: $d + '/J' + $name, dataset: $d, lat: $lat, lon: $lon})",
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
             CREATE (:Shelter {id: $d + '/S_D', name: 'test drop-off', dataset: $d, junction_id: $d + '/JD', capacity: $beds, current_occupancy: 0})",
            &[("beds", dropoff_beds.into())],
        )
        .await;
    }

    /// A vehicle based at depot S_A, positioned at `junction`.
    async fn vehicle(&self, name: &str, asset: &str, capacity: i64, junction: &str) -> String {
        let id = self.id(name);
        self.run(
            "MATCH (s:Shelter {id: $d + '/S_A'}) \
             CREATE (v:Vehicle {vehicle_id: $id, asset_type: $asset, org: 'test', capacity_persons: $capacity, \
                 status: 'AVAILABLE', version: 0, home_shelter_id: s.id, junction_id: $d + '/J' + $junction, \
                 created_at: datetime(), updated_at: datetime()})-[:BASED_AT]->(s)",
            &[("id", id.clone().into()), ("asset", asset.into()), ("capacity", capacity.into()), ("junction", junction.into())],
        )
        .await;
        id
    }

    /// An approved incident at `junction` with one QUEUED task that has
    /// waited `waited_s`. Returns the task id.
    async fn task(&self, name: &str, rank: i64, asset: &str, headcount: i64, junction: &str, waited_s: i64) -> String {
        let task_id = self.id(&format!("T-{name}"));
        self.run(
            "MATCH (j:Junction {id: $d + '/J' + $junction, dataset: $d}) \
             CREATE (i:OpsIncident {incident_id: $d + '/I-' + $name, status: 'TRIAGE_APPROVED', version: 0, \
                 headcount: $headcount, priority_rank: $rank, junction_id: j.id, created_at: datetime()}) \
             CREATE (i)-[:LOCATED_AT]->(j) \
             CREATE (i)-[:HAS_TASK {seq: 1}]->(:Task {task_id: $task_id, incident_id: i.incident_id, \
                 kind: 'ROAD_PICKUP', status: 'QUEUED', required_asset: $asset, priority_rank: $rank, version: 0, \
                 created_at: datetime(), queued_at: datetime() - duration({seconds: $waited})})",
            &[
                ("name", name.into()),
                ("junction", junction.into()),
                ("headcount", headcount.into()),
                ("rank", rank.into()),
                ("task_id", task_id.clone().into()),
                ("asset", asset.into()),
                ("waited", waited_s.into()),
            ],
        )
        .await;
        task_id
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
            PipelineParams::default(),
        )
    }

    async fn task_status(&self, task_id: &str) -> String {
        let mut result = self
            .neo4j
            .graph
            .execute(query("MATCH (t:Task {task_id: $id}) RETURN t.status AS s").param("id", task_id))
            .await
            .unwrap();
        result.next().await.unwrap().unwrap().get("s").unwrap()
    }

    async fn outbox(&self, kind: &str, aggregate: &str) -> i64 {
        let mut result = self
            .neo4j
            .graph
            .execute(
                query("MATCH (e:OutboxEvent {type: $kind, aggregate_id: $id}) RETURN count(e) AS n")
                    .param("kind", kind)
                    .param("id", aggregate),
            )
            .await
            .unwrap();
        result.next().await.unwrap().unwrap().get::<i64>("n").unwrap()
    }

    /// Fail if any vehicle or task has more than one active vehicle
    /// reservation, or a vehicle is ASSIGNED/HELD without one.
    async fn assert_no_overbooking(&self) {
        let per_vehicle = self
            .count(
                "MATCH (r:Reservation {resource_kind: 'VEHICLE'}) WHERE r.vehicle_id STARTS WITH $d \
                   AND r.active_vehicle_key IS NOT NULL \
                 WITH r.vehicle_id AS v, count(r) AS c WHERE c > 1 RETURN count(v) AS n",
            )
            .await;
        let per_task = self
            .count(
                "MATCH (r:Reservation {resource_kind: 'VEHICLE'}) WHERE r.task_id STARTS WITH $d \
                   AND r.active_task_key IS NOT NULL \
                 WITH r.task_id AS t, count(r) AS c WHERE c > 1 RETURN count(t) AS n",
            )
            .await;
        let orphan_status = self
            .count(
                "MATCH (v:Vehicle) WHERE v.vehicle_id STARTS WITH $d AND v.status IN ['HELD', 'ASSIGNED'] \
                   AND NOT EXISTS { MATCH (r:Reservation {active_vehicle_key: v.vehicle_id}) } \
                 RETURN count(v) AS n",
            )
            .await;
        let assigned_twice = self
            .count(
                "MATCH (r:Reservation {resource_kind: 'VEHICLE', status: 'COMMITTED'}) WHERE r.task_id STARTS WITH $d \
                 WITH r.task_id AS t, count(r) AS c WHERE c > 1 RETURN count(t) AS n",
            )
            .await;
        assert_eq!((per_vehicle, per_task, orphan_status, assigned_twice), (0, 0, 0, 0), "overbooking in {}", self.dataset);
    }

    async fn cleanup(&self) {
        let statements = [
            "MATCH (r:Reservation) WHERE r.task_id STARTS WITH $d \
             OPTIONAL MATCH (r)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE r, a",
            "MATCH (n) WHERE (n:Task OR n:OpsIncident OR n:Vehicle) \
               AND coalesce(n.task_id, n.incident_id, n.vehicle_id) STARTS WITH $d \
             OPTIONAL MATCH (n)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE n, a",
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
        ] {
            let _: Result<(), _> = redis::cmd("DEL").arg(key).query_async(&mut conn).await;
        }
    }
}

async fn leader(matcher: &Matcher) -> u64 {
    matcher.lease().try_acquire().await.unwrap().expect("lease free")
}

/// One cycle assigns the vehicle on the fastest route, records the route and
/// its overlay version, reserves drop-off beds and publishes TASK_ASSIGNED.
#[tokio::test]
async fn cycle_assigns_routes_reserves_beds_and_publishes() {
    let Some(ctx) = setup(50).await else { return };
    let vehicle = ctx.vehicle("AMB-1", "AMBULANCE", 4, "A").await;
    let task = ctx.task("1", 1, "AMBULANCE", 2, "D", 0).await;
    let matcher = ctx.matcher().await;
    let token = leader(&matcher).await;
    assert_eq!(matcher.sync_queue().await.unwrap(), 1);

    let report = matcher.run_cycle(token).await.unwrap();
    assert_eq!(report.assigned, vec![(task.clone(), vehicle.clone())]);
    assert_eq!(report.dropoff_unresolved, 0);
    assert_eq!(ctx.task_status(&task).await, "ASSIGNED");
    let route = ctx.string("MATCH (t:Task) WHERE t.task_id STARTS WITH $d RETURN t.route_json AS v").await.unwrap();
    assert!(route.contains("/bd") && !route.contains("/cd"), "fastest route: {route}");
    assert_eq!(ctx.count("MATCH (t:Task) WHERE t.task_id STARTS WITH $d AND t.overlay_version = 0 RETURN count(t) AS n").await, 1);
    assert_eq!(ctx.count("MATCH (v:Vehicle {status: 'ASSIGNED'}) WHERE v.vehicle_id STARTS WITH $d RETURN count(v) AS n").await, 1);
    assert_eq!(ctx.count("MATCH (i:OpsIncident {status: 'ASSIGNED'}) WHERE i.incident_id STARTS WITH $d RETURN count(i) AS n").await, 1);
    assert_eq!(ctx.count("MATCH (s:Shelter {dataset: $d}) WHERE s.current_occupancy = 2 RETURN count(s) AS n").await, 1);
    assert_eq!(
        ctx.count("MATCH (r:Reservation {resource_kind: 'BEDS', status: 'COMMITTED', beds: 2}) WHERE r.task_id STARTS WITH $d RETURN count(r) AS n").await,
        1
    );
    assert_eq!(ctx.outbox("TASK_ASSIGNED", &task).await, 1);
    assert_eq!(ctx.outbox("DROPOFF_UNRESOLVED", &task).await, 0);
    assert_eq!(matcher.queue().len().await.unwrap(), 0, "assigned task leaves the queue");

    // A further cycle changes nothing.
    assert!(matcher.run_cycle(token).await.unwrap().assigned.is_empty());
    ctx.assert_no_overbooking().await;
    ctx.cleanup().await;
}

/// Strict tiers: CRITICAL gets the only vehicle although STABLE is closer.
#[tokio::test]
async fn critical_takes_the_only_vehicle_from_a_closer_stable_task() {
    let Some(ctx) = setup(50).await else { return };
    let vehicle = ctx.vehicle("AMB-1", "AMBULANCE", 4, "A").await;
    let _stable = ctx.task("stable", 0, "AMBULANCE", 1, "B", 0).await;
    let critical = ctx.task("critical", 2, "AMBULANCE", 1, "D", 0).await;
    let matcher = ctx.matcher().await;
    let token = leader(&matcher).await;
    matcher.sync_queue().await.unwrap();

    let report = matcher.run_cycle(token).await.unwrap();
    assert_eq!(report.assigned, vec![(critical, vehicle)]);
    ctx.cleanup().await;
}

/// The matcher, racing many direct holds ("dispatcher actions") on few
/// vehicles, never overbooks: the database constraints decide.
#[tokio::test]
async fn no_overbooking_when_dispatcher_actions_race_the_matcher() {
    let Some(ctx) = setup(500).await else { return };
    let mut vehicles = Vec::new();
    for n in 0..3 {
        vehicles.push(ctx.vehicle(&format!("AMB-{n}"), "AMBULANCE", 4, "A").await);
    }
    let mut tasks = Vec::new();
    for n in 0..12 {
        tasks.push(ctx.task(&format!("{n}"), (n % 3) as i64, "AMBULANCE", 1, "D", 0).await);
    }
    let matcher = ctx.matcher().await;
    let token = leader(&matcher).await;
    matcher.sync_queue().await.unwrap();

    let mut handles = Vec::new();
    for n in 0..40usize {
        let repo = ctx.repo.clone();
        let vehicle = vehicles[n % vehicles.len()].clone();
        let task = tasks[(n * 7) % tasks.len()].clone();
        let incident = task.replace("/T-", "/I-");
        handles.push(tokio::spawn(async move {
            let version = repo.get_vehicle(&vehicle).await.unwrap().map_or(0, |v| v.version);
            let until = Utc::now() + chrono::Duration::seconds(30);
            match repo.hold_vehicle(&task, &incident, &vehicle, version, until).await {
                Ok(_) => 1,
                Err(OpsRepoError::Conflict(_) | OpsRepoError::NotAvailable(_)) => 0,
                Err(e) => panic!("unexpected hold error: {e}"),
            }
        }));
    }
    let cycles = {
        let matcher = matcher.clone();
        tokio::spawn(async move {
            let mut assigned = 0;
            for _ in 0..5 {
                assigned += matcher.run_cycle(token).await.unwrap().assigned.len();
            }
            assigned
        })
    };
    let mut held = 0;
    for handle in handles {
        held += handle.await.unwrap();
    }
    let assigned = cycles.await.unwrap();
    // One more cycle finishes any dispatcher hold left uncommitted.
    let assigned = assigned + matcher.run_cycle(token).await.unwrap().assigned.len();
    eprintln!("direct holds won: {held}, matcher commits: {assigned}");

    ctx.assert_no_overbooking().await;
    let committed = ctx
        .count("MATCH (r:Reservation {resource_kind: 'VEHICLE', status: 'COMMITTED'}) WHERE r.task_id STARTS WITH $d RETURN count(r) AS n")
        .await;
    assert_eq!(committed, 3, "every vehicle ends up committed exactly once");
    assert_eq!(ctx.count("MATCH (t:Task {status: 'ASSIGNED'}) WHERE t.task_id STARTS WITH $d RETURN count(t) AS n").await, 3);
    ctx.cleanup().await;
}

/// A hold whose commit never happened (crash between hold and commit) is
/// finished by the next matcher with the same reservation.
#[tokio::test]
async fn hold_left_by_a_crashed_matcher_is_resumed_not_reallocated() {
    let Some(ctx) = setup(50).await else { return };
    let vehicle = ctx.vehicle("AMB-1", "AMBULANCE", 4, "A").await;
    let _other = ctx.vehicle("AMB-2", "AMBULANCE", 4, "A").await;
    let task = ctx.task("1", 1, "AMBULANCE", 1, "D", 0).await;
    let held = ctx
        .repo
        .hold_vehicle(&task, &task.replace("/T-", "/I-"), &vehicle, 0, Utc::now() + chrono::Duration::seconds(60))
        .await
        .unwrap();

    let matcher = ctx.matcher().await;
    let token = leader(&matcher).await;
    matcher.sync_queue().await.unwrap();
    let report = matcher.run_cycle(token).await.unwrap();
    assert_eq!(report.resumed_holds, 1);
    assert_eq!(report.assigned, vec![(task.clone(), vehicle)]);
    let committed = ctx
        .count("MATCH (r:Reservation {resource_kind: 'VEHICLE'}) WHERE r.task_id STARTS WITH $d RETURN count(r) AS n")
        .await;
    assert_eq!(committed, 1, "no second reservation");
    assert_eq!(
        ctx.repo.get_reservation(&held.reservation.reservation_id).await.unwrap().unwrap().status.as_str(),
        "COMMITTED"
    );
    ctx.cleanup().await;
}

/// An expired hold frees the vehicle, which is then assigned again.
#[tokio::test]
async fn expired_hold_frees_the_vehicle() {
    let Some(ctx) = setup(50).await else { return };
    let vehicle = ctx.vehicle("AMB-1", "AMBULANCE", 4, "A").await;
    let task = ctx.task("1", 1, "AMBULANCE", 1, "D", 0).await;
    let stale = ctx
        .repo
        .hold_vehicle(&task, &task.replace("/T-", "/I-"), &vehicle, 0, Utc::now() - chrono::Duration::seconds(1))
        .await
        .unwrap();

    let matcher = ctx.matcher().await;
    let token = leader(&matcher).await;
    matcher.sync_queue().await.unwrap();
    let report = matcher.run_cycle(token).await.unwrap();
    assert_eq!(report.resumed_holds, 0, "an expired hold is not resumed");
    // Expiry is global: a cycle in a parallel test may be expiring the hold
    // while this one lists vehicles, so allow the next cycle to assign it.
    let mut assigned = report.assigned;
    if assigned.is_empty() {
        assigned = matcher.run_cycle(token).await.unwrap().assigned;
    }
    assert_eq!(assigned, vec![(task, vehicle)]);
    assert_eq!(ctx.repo.get_reservation(&stale.reservation.reservation_id).await.unwrap().unwrap().status.as_str(), "EXPIRED");
    ctx.assert_no_overbooking().await;
    ctx.cleanup().await;
}

/// A hazard the snapshot has not seen yet fails live verification: the hold
/// is released, the overlay refreshed, and the next cycle routes around it.
#[tokio::test]
async fn failed_live_verification_releases_refreshes_and_reroutes() {
    let Some(ctx) = setup(50).await else { return };
    let vehicle = ctx.vehicle("AMB-1", "AMBULANCE", 4, "A").await;
    let task = ctx.task("1", 1, "AMBULANCE", 1, "D", 0).await;
    let matcher = ctx.matcher().await;
    let token = leader(&matcher).await;
    matcher.sync_queue().await.unwrap();

    // Written in Neo4j after the snapshot loaded; no refresher is running.
    ctx.repo
        .apply_hazard(
            &ActorContext::new("test-verifier", [ActorRole::HazardVerifier]),
            &HazardCommand {
                request_id: ctx.id("hz-bd"),
                dataset: ctx.dataset.clone(),
                segment_id: ctx.id("bd"),
                status: HazardStatus::Flooded,
                valid_until: Some(Utc::now() + chrono::Duration::hours(1)),
                reason: "test".into(),
                source: HazardSource::Operator,
            },
        )
        .await
        .unwrap();

    let first = matcher.run_cycle(token).await.unwrap();
    assert_eq!((first.verify_failures, first.assigned.len()), (1, 0));
    assert_eq!(ctx.task_status(&task).await, "QUEUED");
    assert_eq!(ctx.count("MATCH (v:Vehicle {status: 'AVAILABLE'}) WHERE v.vehicle_id STARTS WITH $d RETURN count(v) AS n").await, 1);
    assert_eq!(ctx.count("MATCH (t:Task {verify_failures: 1}) WHERE t.task_id STARTS WITH $d RETURN count(t) AS n").await, 1);
    assert_eq!(matcher.snapshot().overlay().version, 1, "overlay refreshed after the failure");

    let second = matcher.run_cycle(token).await.unwrap();
    assert_eq!(second.assigned, vec![(task.clone(), vehicle)]);
    let route = ctx.string("MATCH (t:Task) WHERE t.task_id STARTS WITH $d RETURN t.route_json AS v").await.unwrap();
    assert!(route.contains("/cd") && !route.contains("/bd"), "detour: {route}");
    assert_eq!(ctx.count("MATCH (t:Task {overlay_version: 1}) WHERE t.task_id STARTS WITH $d RETURN count(t) AS n").await, 1);
    ctx.cleanup().await;
}

/// Repeated verification failures escalate once; the task stays queued.
#[tokio::test]
async fn repeated_verification_failures_escalate_once() {
    let Some(ctx) = setup(50).await else { return };
    ctx.vehicle("AMB-1", "AMBULANCE", 4, "A").await;
    let task = ctx.task("1", 1, "AMBULANCE", 1, "D", 0).await;
    let matcher = ctx.matcher().await;
    let token = leader(&matcher).await;
    matcher.sync_queue().await.unwrap();
    // The live road data now refuses ambulances everywhere; the snapshot's
    // baseline (loaded once) still allows them, so every check fails.
    ctx.run("MATCH ()-[r:CONNECTS_TO {dataset: $d}]->() SET r.emergency = false", &[]).await;

    let mut escalated = 0;
    for _ in 0..4 {
        let report = matcher.run_cycle(token).await.unwrap();
        assert_eq!((report.verify_failures, report.assigned.len()), (1, 0));
        escalated += report.escalated;
    }
    assert_eq!(escalated, 1);
    assert_eq!(ctx.outbox("TASK_ESCALATED", &task).await, 1);
    assert_eq!(ctx.task_status(&task).await, "QUEUED");
    ctx.cleanup().await;
}

/// No eligible vehicle (headcount above every capacity) and waiting past
/// the tier's threshold each raise exactly one escalation.
#[tokio::test]
async fn escalates_unservable_and_overdue_tasks_once() {
    let Some(ctx) = setup(50).await else { return };
    // The only vehicle sits at D, from where nothing is reachable.
    ctx.vehicle("AMB-1", "AMBULANCE", 4, "D").await;
    let too_many = ctx.task("crowd", 1, "AMBULANCE", 30, "B", 0).await;
    let overdue = ctx.task("late", 1, "AMBULANCE", 1, "B", 11 * 60).await;
    let fresh = ctx.task("fresh", 1, "AMBULANCE", 1, "C", 0).await;
    let matcher = ctx.matcher().await;
    let token = leader(&matcher).await;
    matcher.sync_queue().await.unwrap();

    for _ in 0..2 {
        assert!(matcher.run_cycle(token).await.unwrap().assigned.is_empty());
    }
    assert_eq!(ctx.outbox("TASK_ESCALATED", &too_many).await, 1);
    assert_eq!(ctx.outbox("TASK_ESCALATED", &overdue).await, 1);
    assert_eq!(ctx.outbox("TASK_ESCALATED", &fresh).await, 0);
    let reasons = ctx.string("MATCH (t:Task) WHERE t.task_id = $d + '/T-crowd' RETURN head(t.escalations) AS v").await;
    assert_eq!(reasons.as_deref(), Some("NO_ELIGIBLE_VEHICLE"));
    let reasons = ctx.string("MATCH (t:Task) WHERE t.task_id = $d + '/T-late' RETURN head(t.escalations) AS v").await;
    assert_eq!(reasons.as_deref(), Some("OVERDUE"));
    ctx.cleanup().await;
}

/// No shelter with free beds: the pickup still goes out, with an alert.
#[tokio::test]
async fn no_free_beds_still_dispatches_with_an_alert() {
    let Some(ctx) = setup(0).await else { return };
    let vehicle = ctx.vehicle("AMB-1", "AMBULANCE", 4, "A").await;
    let task = ctx.task("1", 2, "AMBULANCE", 3, "D", 0).await;
    let matcher = ctx.matcher().await;
    let token = leader(&matcher).await;
    matcher.sync_queue().await.unwrap();

    let report = matcher.run_cycle(token).await.unwrap();
    assert_eq!(report.assigned, vec![(task.clone(), vehicle)]);
    assert_eq!(report.dropoff_unresolved, 1);
    assert_eq!(ctx.outbox("DROPOFF_UNRESOLVED", &task).await, 1);
    assert_eq!(ctx.count("MATCH (r:Reservation {resource_kind: 'BEDS'}) WHERE r.task_id STARTS WITH $d RETURN count(r) AS n").await, 0);
    ctx.cleanup().await;
}

/// A former leader cannot commit after a newer leader has; its hold is
/// finished by the new leader, so each task is assigned exactly once.
#[tokio::test]
async fn former_leader_is_fenced_and_its_hold_is_finished_by_the_new_one() {
    let Some(ctx) = setup(50).await else { return };
    ctx.vehicle("AMB-1", "AMBULANCE", 4, "A").await;
    ctx.vehicle("AMB-2", "AMBULANCE", 4, "A").await;
    let first = ctx.task("1", 1, "AMBULANCE", 1, "D", 0).await;

    let old = ctx.matcher().await;
    let mut old_lease = old.lease();
    let old_token = old_lease.try_acquire().await.unwrap().unwrap();
    // The old leader pauses; its lease is gone and a new leader takes over.
    old_lease.release().await.unwrap();
    let new = ctx.matcher().await;
    let new_token = leader(&new).await;
    assert!(new_token > old_token);
    new.sync_queue().await.unwrap();
    assert_eq!(new.run_cycle(new_token).await.unwrap().assigned.len(), 1);
    assert_eq!(ctx.task_status(&first).await, "ASSIGNED");

    // The old leader wakes up and tries to assign a new task.
    let second = ctx.task("2", 1, "AMBULANCE", 1, "D", 0).await;
    old.sync_queue().await.unwrap();
    let error = old.run_cycle(old_token).await.unwrap_err();
    assert!(matches!(error.downcast_ref::<OpsRepoError>(), Some(OpsRepoError::Fenced(_))), "{error}");
    assert_eq!(ctx.task_status(&second).await, "QUEUED");

    let report = new.run_cycle(new_token).await.unwrap();
    assert_eq!(report.resumed_holds, 1, "the fenced leader's hold is resumed");
    assert_eq!(ctx.task_status(&second).await, "ASSIGNED");
    ctx.assert_no_overbooking().await;
    ctx.cleanup().await;
}

/// The queue is a copy: after it is lost, a sync restores exactly the
/// queued tasks, in priority order.
#[tokio::test]
async fn queue_is_rebuilt_from_neo4j() {
    let Some(ctx) = setup(50).await else { return };
    let stable = ctx.task("s", 0, "AMBULANCE", 1, "D", 0).await;
    let critical = ctx.task("c", 2, "AMBULANCE", 1, "D", 0).await;
    let urgent = ctx.task("u", 1, "AMBULANCE", 1, "D", 0).await;
    let matcher = ctx.matcher().await;
    assert_eq!(matcher.sync_queue().await.unwrap(), 3);
    matcher.queue().clear().await.unwrap();
    assert_eq!(matcher.queue().len().await.unwrap(), 0);

    assert_eq!(matcher.sync_queue().await.unwrap(), 3);
    assert_eq!(matcher.queue().first(10).await.unwrap(), vec![critical.clone(), urgent.clone(), stable.clone()]);

    ctx.run("MATCH (t:Task {task_id: $id}) SET t.status = 'CANCELLED'", &[("id", urgent.clone().into())]).await;
    matcher.sync_queue().await.unwrap();
    let members: HashSet<String> = matcher.queue().first(10).await.unwrap().into_iter().collect();
    assert_eq!(members, HashSet::from([critical, stable]));
    ctx.cleanup().await;
}

/// The matcher loop elects one leader; when it stops, the standby takes over
/// within the lease TTL and serves new tasks; nothing is assigned twice.
#[tokio::test]
async fn standby_matcher_takes_over_after_the_leader_stops() {
    let Some(ctx) = setup(50).await else { return };
    for n in 0..3 {
        ctx.vehicle(&format!("AMB-{n}"), "AMBULANCE", 4, "A").await;
    }
    let first = ctx.task("1", 1, "AMBULANCE", 1, "D", 0).await;
    let params = PipelineParams {
        matcher_cycle_ms: 200,
        matcher_rebuild_secs: 1,
        lease_ttl_ms: 2_000,
        lease_renew_ms: 500,
        events_stream: format!("{}:events", ctx.dataset),
        ..PipelineParams::default()
    };
    let snapshot = |ctx: &Ctx| {
        let repo = ctx.repo.clone();
        let state = StateManager::new(ctx.neo4j.clone());
        let dataset = ctx.dataset.clone();
        async move { Arc::new(GraphSnapshot::load(repo, &state, &dataset).await.unwrap()) }
    };
    let make = |snapshot| {
        Matcher::new(ctx.repo.clone(), ctx.redis.clone(), StateManager::new(ctx.neo4j.clone()), snapshot, params.clone())
    };
    let (a_stop, b_stop) = (Shutdown::new(), Shutdown::new());
    let a = tokio::spawn(make(snapshot(&ctx).await).run(a_stop.clone()));
    let b = tokio::spawn(make(snapshot(&ctx).await).run(b_stop.clone()));

    wait_for(|| async { ctx.task_status(&first).await == "ASSIGNED" }).await;
    // Stop whichever is leader: stopping both one at a time keeps a standby.
    a_stop.trigger();
    let _ = a.await;
    let second = ctx.task("2", 1, "AMBULANCE", 1, "D", 0).await;
    wait_for(|| async { ctx.task_status(&second).await == "ASSIGNED" }).await;
    b_stop.trigger();
    let _ = b.await;

    ctx.assert_no_overbooking().await;
    let mut conn = ctx.redis.manager.clone();
    let _: Result<(), _> = redis::cmd("DEL").arg(format!("{}:events", ctx.dataset)).query_async(&mut conn).await;
    ctx.cleanup().await;
}

async fn wait_for<F, Fut>(condition: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..100 {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("condition not met within 10 s");
}
