//! M1 acceptance tests for the operational data model. Run against a live
//! Neo4j (skipped if unreachable). Every test uses its own `TEST-<uuid>`
//! prefix and removes its data afterwards.

use std::collections::HashMap;

use chrono::{Duration, Utc};
use neo4rs::query;
use uuid::Uuid;

use crisis_graph::config::Config;
use crisis_graph::db::{
    ensure_ops_schema, FleetCapacities, Neo4jClient, OpsRepoError, OpsRepository, StateManager,
    BASELINE_FLEET, OPS_CONSTRAINTS,
};
use crisis_graph::models::{
    AssetType, NewReport, ReservationStatus, ResourceKind, TaskKind, TaskStatus, VehicleStatus,
};

async fn connect() -> Option<(Neo4jClient, OpsRepository)> {
    let config = Config::from_env();
    let neo4j = match Neo4jClient::connect(&config).await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Neo4j unavailable; skipping live ops test: {error}");
            return None;
        }
    };
    if neo4j.ping().await.is_err() {
        eprintln!("Neo4j ping failed; skipping live ops test");
        return None;
    }
    ensure_ops_schema(&neo4j).await.expect("ensure ops schema");
    Some((neo4j.clone(), OpsRepository::new(neo4j)))
}

fn prefix() -> String {
    format!("TEST-{}", Uuid::new_v4())
}

async fn create_test_vehicles(neo4j: &Neo4jClient, prefix: &str, count: usize) -> Vec<String> {
    let ids: Vec<String> = (0..count).map(|i| format!("{prefix}-V{i}")).collect();
    neo4j
        .graph
        .run(
            query(
                "UNWIND $ids AS id \
                 CREATE (:Vehicle {vehicle_id: id, asset_type: 'AMBULANCE', org: 'test', \
                     capacity_persons: 4, status: 'AVAILABLE', version: 0, \
                     home_shelter_id: 'TEST', junction_id: null})",
            )
            .param("ids", ids.clone()),
        )
        .await
        .expect("create test vehicles");
    ids
}

async fn count(neo4j: &Neo4jClient, cypher: &str, prefix: &str) -> i64 {
    let mut result = neo4j
        .graph
        .execute(query(cypher).param("p", prefix))
        .await
        .expect("count query");
    result.next().await.unwrap().unwrap().get::<i64>("n").unwrap()
}

async fn cleanup(neo4j: &Neo4jClient, prefix: &str) {
    let statements = [
        // Audit/outbox nodes keyed by a reservation or report UUID
        "MATCH (r:Reservation) WHERE r.task_id STARTS WITH $p \
         OPTIONAL MATCH (r)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE a",
        "MATCH (r:Report) WHERE r.request_id STARTS WITH $p \
         MATCH (e:OutboxEvent {aggregate_id: r.report_id}) DETACH DELETE e",
        "MATCH (n) WHERE (n:Vehicle AND n.vehicle_id STARTS WITH $p) \
            OR (n:Reservation AND n.task_id STARTS WITH $p) \
            OR (n:Task AND n.task_id STARTS WITH $p) \
            OR (n:Report AND n.request_id STARTS WITH $p) \
            OR (n:Shelter AND n.id STARTS WITH $p) \
            OR (n:OutboxEvent AND n.aggregate_id STARTS WITH $p) \
            OR (n:AuditEvent AND n.aggregate_id STARTS WITH $p) \
         DETACH DELETE n",
    ];
    for statement in statements {
        let _ = neo4j.graph.run(query(statement).param("p", prefix)).await;
    }
}

fn held_until() -> chrono::DateTime<Utc> {
    Utc::now() + Duration::minutes(5)
}

/// Acceptance 1 + 6: many concurrent holds on few vehicles never oversell,
/// and audit/outbox are written exactly once per successful hold.
#[tokio::test]
async fn concurrent_holds_never_overbook_a_vehicle() {
    let Some((neo4j, repo)) = connect().await else { return };
    let p = prefix();
    let vehicles = create_test_vehicles(&neo4j, &p, 3).await;

    let mut handles = Vec::new();
    for i in 0..60 {
        let repo = repo.clone();
        let task = format!("{p}-T{i}");
        let vehicle = vehicles[i % vehicles.len()].clone();
        handles.push(tokio::spawn(async move {
            let outcome = repo.hold_vehicle(&task, "INC-TEST", &vehicle, 0, held_until()).await;
            (task, outcome)
        }));
    }

    let (mut succeeded, mut conflicts, mut unavailable) = (Vec::new(), 0, 0);
    for handle in handles {
        let (task, outcome) = handle.await.unwrap();
        match outcome {
            Ok(hold) => {
                assert!(hold.created);
                succeeded.push(task);
            }
            Err(OpsRepoError::Conflict(_)) => conflicts += 1,
            Err(OpsRepoError::NotAvailable(_)) => unavailable += 1,
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
    println!("holds: {} succeeded, {conflicts} blocked by constraint, {unavailable} saw vehicle taken", succeeded.len());

    let active = count(
        &neo4j,
        "MATCH (r:Reservation) WHERE r.task_id STARTS WITH $p AND r.active_vehicle_key IS NOT NULL \
         RETURN count(r) AS n",
        &p,
    )
    .await;
    let max_per_vehicle = count(
        &neo4j,
        "MATCH (r:Reservation)-[:OF_VEHICLE]->(v:Vehicle) \
         WHERE v.vehicle_id STARTS WITH $p AND r.status = 'HELD' \
         WITH v, count(r) AS holds RETURN coalesce(max(holds), 0) AS n",
        &p,
    )
    .await;
    let audits = count(
        &neo4j,
        "MATCH (a:AuditEvent {action: 'VEHICLE_HELD'}) WHERE a.aggregate_id STARTS WITH $p \
         RETURN count(a) AS n",
        &p,
    )
    .await;
    let outbox = count(
        &neo4j,
        "MATCH (e:OutboxEvent {type: 'VEHICLE_HELD'}) WHERE e.aggregate_id STARTS WITH $p \
         RETURN count(e) AS n",
        &p,
    )
    .await;
    cleanup(&neo4j, &p).await;

    assert_eq!(succeeded.len(), 3, "exactly one hold per vehicle must succeed");
    assert_eq!(conflicts + unavailable, 57);
    assert_eq!(active, 3);
    assert_eq!(max_per_vehicle, 1, "a vehicle must never have two active holds");
    assert_eq!(audits, 3, "one audit event per successful hold, none for failures");
    assert_eq!(outbox, 3, "one outbox event per successful hold, none for failures");
}

/// Acceptance 2: one task can hold only one vehicle at a time.
#[tokio::test]
async fn a_task_holds_at_most_one_vehicle() {
    let Some((neo4j, repo)) = connect().await else { return };
    let p = prefix();
    let vehicles = create_test_vehicles(&neo4j, &p, 5).await;
    let task = format!("{p}-T0");

    let mut handles = Vec::new();
    for vehicle in vehicles {
        let repo = repo.clone();
        let task = task.clone();
        handles.push(tokio::spawn(async move {
            repo.hold_vehicle(&task, "INC-TEST", &vehicle, 0, held_until()).await
        }));
    }
    let mut ok = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => ok += 1,
            Err(OpsRepoError::Conflict(_)) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
    let held = count(
        &neo4j,
        "MATCH (v:Vehicle {status: 'HELD'}) WHERE v.vehicle_id STARTS WITH $p RETURN count(v) AS n",
        &p,
    )
    .await;
    cleanup(&neo4j, &p).await;

    assert_eq!(ok, 1);
    assert_eq!(held, 1, "losing attempts must not leave vehicles HELD");
}

/// Acceptance 3: repeating a hold (e.g. after redelivery) returns the same
/// reservation and writes nothing new.
#[tokio::test]
async fn repeated_hold_is_idempotent() {
    let Some((neo4j, repo)) = connect().await else { return };
    let p = prefix();
    let vehicle = create_test_vehicles(&neo4j, &p, 1).await.remove(0);
    let task = format!("{p}-T0");

    let first = repo.hold_vehicle(&task, "INC-TEST", &vehicle, 0, held_until()).await.unwrap();
    // The retry still carries the stale expected version; it must not matter.
    let second = repo.hold_vehicle(&task, "INC-TEST", &vehicle, 0, held_until()).await.unwrap();
    let outbox = count(
        &neo4j,
        "MATCH (e:OutboxEvent {type: 'VEHICLE_HELD'}) WHERE e.aggregate_id STARTS WITH $p \
         RETURN count(e) AS n",
        &p,
    )
    .await;
    let active = repo.active_reservation(&task, ResourceKind::Vehicle).await.unwrap();
    cleanup(&neo4j, &p).await;

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(first.reservation.reservation_id, second.reservation.reservation_id);
    assert_eq!(outbox, 1);
    assert_eq!(active.unwrap().status, ReservationStatus::Held);
}

/// Acceptance 4: release and expiry free the vehicle; repeating is harmless.
#[tokio::test]
async fn release_and_expiry_free_the_vehicle() {
    let Some((neo4j, repo)) = connect().await else { return };
    let p = prefix();
    let vehicles = create_test_vehicles(&neo4j, &p, 2).await;

    // Release
    let hold = repo
        .hold_vehicle(&format!("{p}-T0"), "INC-TEST", &vehicles[0], 0, held_until())
        .await
        .unwrap();
    assert!(repo.commit_vehicle_hold(&hold.reservation.reservation_id).await.unwrap());
    let assigned = repo.get_vehicle(&vehicles[0]).await.unwrap().unwrap();
    assert_eq!(assigned.status, VehicleStatus::Assigned);

    assert!(repo.release_reservation(&hold.reservation.reservation_id, "test").await.unwrap());
    assert!(!repo.release_reservation(&hold.reservation.reservation_id, "test").await.unwrap());
    let freed = repo.get_vehicle(&vehicles[0]).await.unwrap().unwrap();
    assert_eq!(freed.status, VehicleStatus::Available);
    // A new task can now hold it at its current version.
    let rehold = repo
        .hold_vehicle(&format!("{p}-T1"), "INC-TEST", &vehicles[0], freed.version, held_until())
        .await;

    // Expiry
    let expiring = repo
        .hold_vehicle(&format!("{p}-T2"), "INC-TEST", &vehicles[1], 0, Utc::now() - Duration::seconds(1))
        .await
        .unwrap();
    let expired = repo.expire_due_holds(100).await.unwrap();
    let after = repo.get_reservation(&expiring.reservation.reservation_id).await.unwrap().unwrap();
    let vehicle_after = repo.get_vehicle(&vehicles[1]).await.unwrap().unwrap();
    cleanup(&neo4j, &p).await;

    assert!(rehold.is_ok(), "released vehicle must be holdable again: {rehold:?}");
    assert!(expired >= 1);
    assert_eq!(after.status, ReservationStatus::Expired);
    assert_eq!(vehicle_after.status, VehicleStatus::Available);
}

/// Acceptance 5: concurrent bed reservations never exceed shelter capacity.
#[tokio::test]
async fn concurrent_bed_reservations_respect_capacity() {
    let Some((neo4j, repo)) = connect().await else { return };
    let p = prefix();
    let shelter = format!("{p}-SHELTER");
    neo4j
        .graph
        .run(
            query("CREATE (:Shelter {id: $id, name: 'Test shelter', capacity: 10, current_occupancy: 0})")
                .param("id", shelter.clone()),
        )
        .await
        .unwrap();

    let mut handles = Vec::new();
    for i in 0..100 {
        let repo = repo.clone();
        let shelter = shelter.clone();
        let task = format!("{p}-T{i}");
        handles.push(tokio::spawn(async move {
            repo.reserve_beds(&shelter, 1, &task, "INC-TEST").await
        }));
    }
    let mut reservations = Vec::new();
    for handle in handles {
        match handle.await.unwrap() {
            Ok(reservation) => reservations.push(reservation),
            Err(OpsRepoError::NotAvailable(_)) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
    let occupancy = count(
        &neo4j,
        "MATCH (s:Shelter {id: $p + '-SHELTER'}) RETURN s.current_occupancy AS n",
        &p,
    )
    .await;

    for reservation in &reservations {
        assert!(repo.release_reservation(&reservation.reservation_id, "test").await.unwrap());
    }
    let after_release = count(
        &neo4j,
        "MATCH (s:Shelter {id: $p + '-SHELTER'}) RETURN s.current_occupancy AS n",
        &p,
    )
    .await;
    cleanup(&neo4j, &p).await;

    assert_eq!(reservations.len(), 10, "exactly the free capacity can be reserved");
    assert_eq!(occupancy, 10);
    assert_eq!(after_release, 0);
}

/// D-1: a retried report (same request_id) is stored once.
#[tokio::test]
async fn retried_report_is_stored_once() {
    let Some((neo4j, repo)) = connect().await else { return };
    let p = prefix();
    let report = NewReport {
        request_id: format!("{p}-REQ"),
        device_id: format!("{p}-DEVICE"),
        channel: "test".into(),
        raw_text: "Water entering house near Bank Junction, 4 people".into(),
        latitude: Some(10.1082),
        longitude: Some(76.3565),
        accuracy_m: Some(15.0),
    };
    let (first, created_first) = repo.create_report(&report).await.unwrap();
    let (second, created_second) = repo.create_report(&report).await.unwrap();
    let fetched = repo.get_report(&first.report_id).await.unwrap();
    let outbox = count(
        &neo4j,
        "MATCH (r:Report) WHERE r.request_id STARTS WITH $p \
         MATCH (e:OutboxEvent {aggregate_id: r.report_id, type: 'REPORT_RECEIVED'}) \
         RETURN count(e) AS n",
        &p,
    )
    .await;
    cleanup(&neo4j, &p).await;

    assert!(created_first);
    assert!(!created_second);
    assert_eq!(first.report_id, second.report_id);
    assert_eq!(fetched.unwrap().latitude, Some(10.1082));
    assert_eq!(outbox, 1);
}

/// Task transitions are guarded by status and version.
#[tokio::test]
async fn task_transitions_are_version_checked() {
    let Some((neo4j, repo)) = connect().await else { return };
    let p = prefix();
    let task_id = format!("{p}-T0");
    let task = repo
        .create_task(&task_id, "INC-TEST", TaskKind::RoadPickup, TaskStatus::Queued, AssetType::Ambulance)
        .await
        .unwrap();
    let again = repo
        .create_task(&task_id, "INC-TEST", TaskKind::RoadPickup, TaskStatus::Queued, AssetType::Ambulance)
        .await
        .unwrap();
    let assigned = repo
        .transition_task(&task_id, task.version, TaskStatus::Queued, TaskStatus::Assigned, "test")
        .await
        .unwrap();
    let stale = repo
        .transition_task(&task_id, task.version, TaskStatus::Queued, TaskStatus::Assigned, "test")
        .await;
    cleanup(&neo4j, &p).await;

    assert_eq!(again.version, 0, "re-creating a task must not reset or duplicate it");
    assert_eq!(assigned.status, TaskStatus::Assigned);
    assert_eq!(assigned.version, 1);
    assert!(matches!(stale, Err(OpsRepoError::NotAvailable(_))));
}

/// Acceptance 7: seeding the baseline fleet is repeatable and matches it.
#[tokio::test]
async fn baseline_fleet_seed_is_idempotent() {
    let Some((neo4j, repo)) = connect().await else { return };
    StateManager::new(neo4j.clone()).ensure_aluva_shelters().await.unwrap();
    repo.seed_baseline_fleet(FleetCapacities::default()).await.unwrap();
    let created_again = repo.seed_baseline_fleet(FleetCapacities::default()).await.unwrap();

    let mut result = neo4j
        .graph
        .execute(query(
            "MATCH (v:Vehicle)-[:BASED_AT]->(s:Shelter) \
             WHERE v.vehicle_id =~ '(AMB|TRK|BOAT)-(TH|UC|MP|TK)-[0-9]+' \
             RETURN s.id AS shelter, v.asset_type AS asset, count(v) AS n",
        ))
        .await
        .unwrap();
    let mut actual: HashMap<(String, String), i64> = HashMap::new();
    while let Some(row) = result.next().await.unwrap() {
        actual.insert((row.get("shelter").unwrap(), row.get("asset").unwrap()), row.get("n").unwrap());
    }

    assert_eq!(created_again, 0);
    for (shelter, _, ambulances, trucks, boats) in BASELINE_FLEET {
        for (asset, expected) in [("AMBULANCE", ambulances), ("EVAC_TRUCK", trucks), ("RESCUE_BOAT", boats)] {
            let got = actual.get(&(shelter.to_string(), asset.to_string())).copied().unwrap_or(0);
            assert_eq!(got, *expected as i64, "{shelter} {asset}");
        }
    }
}

/// Acceptance 8: the schema can be applied repeatedly and is present.
#[tokio::test]
async fn ops_schema_is_idempotent_and_present() {
    let Some((neo4j, _repo)) = connect().await else { return };
    ensure_ops_schema(&neo4j).await.unwrap();
    ensure_ops_schema(&neo4j).await.unwrap();

    let mut result = neo4j
        .graph
        .execute(query("SHOW CONSTRAINTS YIELD name RETURN collect(name) AS names"))
        .await
        .unwrap();
    let names: Vec<String> = result.next().await.unwrap().unwrap().get("names").unwrap();
    for expected in OPS_CONSTRAINTS {
        assert!(names.iter().any(|n| n == expected), "missing constraint {expected}");
    }
}
