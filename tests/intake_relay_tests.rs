//! M2 acceptance tests: intake API and outbox relay, against live Neo4j and
//! Redis (skipped if unreachable). Streams, lease names and data use a unique
//! `TEST-<uuid>` prefix and are removed afterwards.
//!
//! A relay pass publishes every unpublished event in the database, so all
//! relay scenarios run sequentially in one test and assert on their own
//! event IDs.

use std::time::{Duration, Instant};

use neo4rs::query;
use redis::streams::StreamRangeReply;
use redis::FromRedisValue;
use serde_json::{json, Value};
use uuid::Uuid;

use crisis_graph::api::{intake_routes, ApiState};
use crisis_graph::config::{Config, PipelineParams};
use crisis_graph::db::{ensure_ops_schema, Neo4jClient, OpsRepoError, OpsRepository, RedisClient};
use crisis_graph::models::NewReport;
use crisis_graph::relay::OutboxRelay;
use crisis_graph::runtime::Shutdown;

async fn connect() -> Option<(Neo4jClient, RedisClient, OpsRepository)> {
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
    let repo = OpsRepository::new(neo4j.clone());
    Some((neo4j, redis, repo))
}

fn prefix() -> String {
    format!("TEST-{}", Uuid::new_v4())
}

fn test_params(prefix: &str) -> PipelineParams {
    PipelineParams {
        extract_stream: format!("{prefix}:extract"),
        events_stream: format!("{prefix}:events"),
        outbox_batch_size: 1000,
        ..PipelineParams::default()
    }
}

fn new_report(prefix: &str, n: usize) -> NewReport {
    NewReport {
        request_id: format!("{prefix}-REQ-{n}"),
        device_id: format!("{prefix}-DEVICE"),
        channel: "test".into(),
        raw_text: format!("Test report {n}: water rising near Bank Junction"),
        latitude: None,
        longitude: None,
        accuracy_m: None,
    }
}

/// `event_id`s of every message in a stream, in order.
async fn stream_event_ids(redis: &RedisClient, stream: &str) -> Vec<String> {
    let mut conn = redis.manager.clone();
    let reply: StreamRangeReply = redis::cmd("XRANGE")
        .arg(stream)
        .arg("-")
        .arg("+")
        .query_async(&mut conn)
        .await
        .unwrap_or_default();
    reply
        .ids
        .iter()
        .filter_map(|entry| entry.map.get("event_id"))
        .filter_map(|value| String::from_redis_value(value).ok())
        .collect()
}

/// The outbox event recorded for a report.
async fn report_event(neo4j: &Neo4jClient, report_id: &str) -> (String, bool) {
    let mut result = neo4j
        .graph
        .execute(
            query(
                "MATCH (e:OutboxEvent {aggregate_id: $id, type: 'REPORT_RECEIVED'}) \
                 RETURN e.event_id AS event_id, e.published AS published",
            )
            .param("id", report_id),
        )
        .await
        .unwrap();
    let row = result.next().await.unwrap().expect("report must have an outbox event");
    (row.get("event_id").unwrap(), row.get("published").unwrap())
}

async fn cleanup(neo4j: &Neo4jClient, redis: &RedisClient, prefix: &str) {
    run_cleanup(
        neo4j,
        "MATCH (r:Report) WHERE r.request_id STARTS WITH $p \
         OPTIONAL MATCH (e:OutboxEvent {aggregate_id: r.report_id}) \
         DETACH DELETE e, r",
        prefix,
    )
    .await;
    run_cleanup(neo4j, "MATCH (f:LeaseFence) WHERE f.name STARTS WITH $p DELETE f", prefix).await;
    let mut conn = redis.manager.clone();
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("*{prefix}*"))
        .query_async(&mut conn)
        .await
        .unwrap_or_default();
    for key in keys {
        let _: Result<(), _> = redis::cmd("DEL").arg(key).query_async(&mut conn).await;
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

async fn spawn_api(repo: OpsRepository) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = intake_routes(ApiState { repo });
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Acceptance 1 + 2: retries are idempotent and invalid requests get 400.
#[tokio::test]
async fn intake_api_is_idempotent_and_validates() {
    let Some((neo4j, redis, repo)) = connect().await else { return };
    let p = prefix();
    let base = spawn_api(repo).await;
    let client = reqwest::Client::new();
    let body = json!({
        "request_id": format!("{p}-REQ-1"),
        "device_id": format!("{p}-DEVICE"),
        "raw_text": "Water entering house near Bank Junction, 4 people, elderly woman",
        "latitude": 10.1082,
        "longitude": 76.3565,
        "accuracy_m": 15.0
    });

    let first = client.post(format!("{base}/api/v1/reports")).json(&body).send().await.unwrap();
    let first_status = first.status();
    let first_replayed = first.headers().get("idempotent-replayed").cloned();
    let first_body: Value = first.json().await.unwrap();

    let second = client.post(format!("{base}/api/v1/reports")).json(&body).send().await.unwrap();
    let second_status = second.status();
    let second_replayed = second.headers().get("idempotent-replayed").cloned();
    let second_body: Value = second.json().await.unwrap();

    let report_id = first_body["report_id"].as_str().unwrap().to_string();
    let fetched = client.get(format!("{base}/api/v1/reports/{report_id}")).send().await.unwrap();
    let fetched_status = fetched.status();
    let fetched_body: Value = fetched.json().await.unwrap();

    let missing = client
        .post(format!("{base}/api/v1/reports"))
        .json(&json!({ "device_id": "d", "raw_text": "help" }))
        .send()
        .await
        .unwrap();
    let missing_status = missing.status();
    let missing_body: Value = missing.json().await.unwrap();
    let bad_lat = client
        .post(format!("{base}/api/v1/reports"))
        .json(&json!({ "request_id": format!("{p}-REQ-2"), "device_id": "d", "raw_text": "help",
                       "latitude": 200.0, "longitude": 76.3 }))
        .send()
        .await
        .unwrap()
        .status();
    let unknown = client
        .get(format!("{base}/api/v1/reports/does-not-exist"))
        .send()
        .await
        .unwrap()
        .status();

    let reports = {
        let mut result = neo4j
            .graph
            .execute(query("MATCH (r:Report) WHERE r.request_id STARTS WITH $p RETURN count(r) AS n").param("p", p.clone()))
            .await
            .unwrap();
        result.next().await.unwrap().unwrap().get::<i64>("n").unwrap()
    };
    cleanup(&neo4j, &redis, &p).await;

    assert_eq!(first_status, 202);
    assert!(first_replayed.is_none());
    assert_eq!(second_status, 202);
    assert_eq!(second_replayed.unwrap(), "true");
    assert_eq!(first_body, second_body, "a retry must return the original response");
    assert_eq!(first_body["status"], "PENDING");
    assert_eq!(fetched_status, 200);
    assert_eq!(fetched_body["report_id"], first_body["report_id"]);
    assert_eq!(missing_status, 400);
    assert_eq!(missing_body["error"], "request_id is required");
    assert_eq!(bad_lat, 400);
    assert_eq!(unknown, 404);
    assert_eq!(reports, 1, "two submissions with one request_id must store one report");
}

/// Acceptance 3–6: crash recovery, at-least-once delivery, fencing, failover.
#[tokio::test]
async fn outbox_relay_delivers_reliably_with_one_leader() {
    let Some((neo4j, redis, repo)) = connect().await else { return };
    let p = prefix();
    let params = test_params(&p);
    let lease_name = format!("{p}-relay");
    let relay = OutboxRelay::with_lease_name(repo.clone(), redis.clone(), params.clone(), lease_name.clone());
    let extract = params.extract_stream.clone();

    // --- 3. Crash after commit, before publish: the event is still delivered.
    let (report1, _) = repo.create_report(&new_report(&p, 1)).await.unwrap();
    let (event1, published_before) = report_event(&neo4j, &report1.report_id).await;
    assert!(!published_before, "nothing has published it yet");

    let mut leader = relay.lease();
    let token1 = leader.try_acquire().await.unwrap().expect("first relay becomes leader");
    relay.run_once(token1).await.unwrap();
    let (_, published_after) = report_event(&neo4j, &report1.report_id).await;
    let ids = stream_event_ids(&redis, &extract).await;
    assert_eq!(ids.iter().filter(|id| **id == event1).count(), 1);
    assert!(published_after);

    // --- 4. Crash after publish, before marking: published again, same event_id.
    let (report2, _) = repo.create_report(&new_report(&p, 2)).await.unwrap();
    let (event2, _) = report_event(&neo4j, &report2.report_id).await;
    let pending = repo.fetch_unpublished(10_000).await.unwrap();
    let ours: Vec<_> = pending.into_iter().filter(|e| e.event_id == event2).collect();
    relay.publish(&ours).await.unwrap(); // "crash" here: never marked
    relay.run_once(token1).await.unwrap();
    let ids = stream_event_ids(&redis, &extract).await;
    assert_eq!(
        ids.iter().filter(|id| **id == event2).count(),
        2,
        "at-least-once: duplicate delivery with the same event_id for consumers to discard"
    );

    // --- 5. Two relays: only the holder can acquire; a stale leader is fenced.
    let mut standby = relay.lease();
    assert_eq!(standby.try_acquire().await.unwrap(), None, "lease is held");

    // The leader pauses and its lease expires; the standby takes over.
    let mut conn = redis.manager.clone();
    let _: () = redis::cmd("DEL").arg(format!("lease:{lease_name}")).query_async(&mut conn).await.unwrap();
    let token2 = standby.try_acquire().await.unwrap().expect("standby becomes leader");
    assert!(token2 > token1, "fencing tokens increase with every tenure");

    let (report3, _) = repo.create_report(&new_report(&p, 3)).await.unwrap();
    relay.run_once(token2).await.unwrap();
    let (_, published3) = report_event(&neo4j, &report3.report_id).await;
    assert!(published3);

    // The paused leader resumes with its old token and tries to publish.
    let (report4, _) = repo.create_report(&new_report(&p, 4)).await.unwrap();
    let stale = relay.run_once(token1).await;
    let fenced = matches!(
        stale.as_ref().err().and_then(|e| e.downcast_ref::<OpsRepoError>()),
        Some(OpsRepoError::Fenced(_))
    );
    assert!(fenced, "a stale leader must be rejected, got {stale:?}");
    let (_, published4) = report_event(&neo4j, &report4.report_id).await;
    assert!(!published4, "the stale leader must not mark events published");
    drop(leader);

    // --- 6. Failover: the leader stops renewing; a running standby takes over.
    let failover_lease = format!("{p}-failover");
    let fast = PipelineParams {
        lease_ttl_ms: 500,
        lease_renew_ms: 100,
        outbox_poll_ms: 50,
        ..params.clone()
    };
    let failover_relay =
        OutboxRelay::with_lease_name(repo.clone(), redis.clone(), fast, failover_lease.clone());
    let mut crashed = failover_relay.lease();
    crashed.try_acquire().await.unwrap().expect("crashed leader held the lease");
    // `crashed` never renews: it has died.

    let shutdown = Shutdown::new();
    let standby_task = tokio::spawn(failover_relay.clone().run(shutdown.clone()));
    let (report5, _) = repo.create_report(&new_report(&p, 5)).await.unwrap();
    let (event5, _) = report_event(&neo4j, &report5.report_id).await;

    let started = Instant::now();
    let mut delivered = false;
    while started.elapsed() < Duration::from_secs(5) {
        if stream_event_ids(&redis, &extract).await.contains(&event5) {
            delivered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let takeover = started.elapsed();
    shutdown.trigger();
    let _ = tokio::time::timeout(Duration::from_secs(5), standby_task).await;
    let lease_left: Option<String> = redis::cmd("GET")
        .arg(format!("lease:{failover_lease}"))
        .query_async(&mut conn)
        .await
        .unwrap();
    drop(crashed);
    cleanup(&neo4j, &redis, &p).await;

    assert!(delivered, "standby must take over and deliver within the lease TTL");
    println!("failover delivery after {takeover:?}");
    assert!(lease_left.is_none(), "standby must release its lease on shutdown");
}
