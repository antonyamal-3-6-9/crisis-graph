# AGENTS.md — CrisisGraph Engineering Context

## Mission

CrisisGraph is a Rust emergency-logistics and dynamic evacuation-routing engine for hazards such as flooding, cyclones, debris, and landslides.

It is a **connected, resilient system**, not an offline-only system. Local vLLM inference is a resilience and latency option when WAN connectivity is poor; a realistic deployment can also use connected field reports, call-centre/radio intake, GIS, GPS/AVL, weather/satellite information, and command-centre systems.

The core safety rule is non-negotiable: an LLM may extract a candidate incident or hazard from language, but it must never calculate, invent, or certify a route. Deterministic graph logic makes and verifies routing decisions.

## Design goals

1. **Dynamic map validity:** Baseline maps require a live traversability overlay during hazards.
2. **Topological correctness:** Routing must use a directed graph, valid connectivity, and physical/access constraints rather than LLM reasoning.
3. **Safe dispatch:** A missing, unreachable, stale, or unsafe route must escalate to a human dispatcher.
4. **Concurrent allocation:** Capacity and assets must not be overbooked during simultaneous incidents.
5. **Traceability:** Baseline geographic data, operational reports, route decisions, and asset reservations need provenance and auditability.

## Architecture

```text
SOS / field reports / connected feeds
              |
              v
Triage extraction (local or reachable vLLM-compatible endpoint)
              |
              v
Candidate hazard / location resolution
              |
              +--> operational hazard overlay in Neo4j
              |
              v
Shelter and asset allocation (matcher: batch assignment + constraint-backed Neo4j reservations;
                              legacy server: Redis lock + atomic Neo4j mutation)
              |
              v
Passable directed graph snapshot
              |
              v
Deterministic Petgraph route calculation
              |
              v
Independent safety verification and tactical brief
              |
              v
Dispatcher / responder action or human escalation
```

## Current stack

- **Rust 2021 + Tokio:** asynchronous application/runtime.
- **Neo4j + neo4rs:** baseline road graph, shelters, resources, and dynamic operational state.
- **Redis/Valkey + redis-rs:** Streams and consumer groups (`sos:extract`, `ops:events`), leader leases with fencing tokens, the matcher priority queue (`alloc:queue:{dataset}`); the legacy server still uses distributed locks.
- **vLLM-compatible HTTP endpoint + reqwest:** SOS triage extraction. The default is localhost, but the endpoint is configurable and need not be local.
- **Petgraph:** deterministic shortest-path logic on a directed graph (one-way, access and hazard constraints); the baseline is loaded once per process into a shared snapshot (`arc-swap` hazard overlay).
- **pathfinding + rayon:** Hungarian assignment per priority tier and parallel cost-table searches in the matcher.
- **axum + metrics:** intake/hazard API, `/healthz` `/readyz` `/metrics` (Prometheus) on every role; `sha2` for operator token hashes.
- **OpenTelemetry (`tracing-opentelemetry`, OTLP/HTTP):** one trace per SOS across roles via W3C `traceparent`; exported to Jaeger when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Prometheus, Grafana and Jaeger run natively from `ops/`.
- **Serde/chrono/thiserror/tracing:** typed contracts, time, errors, and instrumentation.

## Repository layout

```text
crisis-graph/
├── AGENTS.md
├── Cargo.toml
├── .env                              # local only; never commit
├── data/
│   ├── boundaries/
│   │   └── aluva-periyar-pilot.geojson
│   └── raw/                          # ignored; downloaded/generated source datasets
├── web/                              # operations console (new pipeline); web/legacy/ = old console for `server`
├── ops/                              # Prometheus config + alert rules, Grafana provisioning + dashboard, run-*.sh
├── docs/
│   ├── asynchronous-multimodal-pipelined-relay.md
│   └── geospatial-data-engineering.md
├── src/
│   ├── bin/
│   │   ├── generate_boundary.rs       # generates pilot GeoJSON boundary
│   │   ├── download_osm.rs            # downloads raw OSM road data
│   │   ├── normalize_osm.rs           # normalizes raw OSM into directed graph GeoJSON
│   │   ├── server.rs                  # legacy all-in-one dispatch server (:3000)
│   │   ├── crisisgraph.rs             # new pipeline: one role per process (--role)
│   │   ├── migrate_ops.rs             # creates the operational data model (idempotent)
│   │   ├── load_baseline_graph.rs     # loads normalized junctions/segments into Neo4j
│   │   ├── bench_matcher.rs           # batch vs one-at-a-time allocation bench (read-only)
│   │   └── surge.rs                   # load test: run a scenario, write results, clean up a run
│   ├── db/
│   │   ├── ops_schema.rs              # operational constraints/indexes
│   │   └── ops_repo/                  # vehicles, reservations, beds, reports, tasks, incidents,
│   │                                  # hazards (T7), allocation (T2 commit, escalations), outbox
│   ├── geospatial/                    # OSM normalization, topology splitting, and validation
│   ├── ingestion/
│   ├── models/
│   ├── pipeline/
│   ├── api/                           # intake, hazard command, console API + commands, SSE events, operator tokens
│   ├── graph/                         # shared graph snapshot (baseline once + hazard overlay)
│   ├── matcher/                       # matcher role: priority queue, tiered Hungarian assignment, T2/T3
│   ├── sim/                           # simulated driver (demo only); surge scenarios + run cleanup
│   ├── relay.rs                       # outbox relay (leader lease + fencing)
│   ├── runtime/                       # roles, lease, breaker, shutdown, /healthz /readyz /metrics,
│   │                                  # telemetry (tracing + OTLP), stream sampler
│   ├── triage/                        # triage workers: extraction, minimal dedup, incidents;
│   │                                  # simulated engine (load tests only)
│   ├── solver/
│   ├── config.rs
│   ├── lib.rs
│   └── main.rs
└── tests/
    ├── geospatial_tests.rs            # validates normalized pilot staging artifacts
    ├── ops_model_tests.rs             # operational model: overbooking stress, idempotency
    ├── intake_relay_tests.rs          # intake API, relay crash recovery, fencing, failover
    ├── triage_worker_tests.rs         # checkpoint reuse, duplicates, D-2/D-3, breaker, dead letters
    ├── graph_hazard_tests.rs          # hazard records/T7, snapshot swap, hazard API and roles
    ├── matcher_tests.rs               # allocation: overbooking race, resume, fencing, failover, tiers
    ├── console_api_tests.rs           # console API, review/complete commands, SSE replay, end-to-end dispatch
    ├── dispatcher_commands_tests.rs   # merge/split (T4), vehicle status and breakdown, simulated driver
    ├── mission_tests.rs               # boat-then-road missions: conversion, trigger (T6), completion, cancel
    ├── observability_tests.rs         # one trace intake → assignment, M8 metrics, simulated engine, surge cleanup
    ├── neo4j_baseline_tests.rs        # directed baseline loader and hazard verification
    ├── aluva_pipeline_tests.rs        # legacy end-to-end pipeline (needs llama.cpp on :8000)
    └── integration_tests.rs
```

## Legacy pipeline (`server` binary)

The legacy all-in-one path (its console is `web/legacy/` on :3000; the main console moved to the new pipeline in M7) passes a typed `CrisisState` (“clipboard”) through four stages:

1. **Triage extraction** — parse raw SOS text into location, headcount, asset type, and hazards.
2. **Allocation** — find and atomically reserve eligible shelter capacity and an asset, guarded by a Redis lock.
3. **Pathfinding** — fetch passable edges from Neo4j and run Petgraph shortest path.
4. **Verification/brief** — check basic path endpoints and allocation state, then emit either a tactical brief or human escalation.

This is a working prototype, not a production dispatch system. It currently uses sample alerts in `main.rs`, a seeded demonstration graph, basic lexical spatial resolution, and simple verification. Do not describe it as production-ready or “mission-critical” in user-facing claims.

## Aluva geospatial pilot

The first real-data pilot is deliberately bounded:

```text
Name: Aluva–Periyar Emergency Routing Pilot
Centre: Aluva Railway Station
Latitude: 10.10816
Longitude: 76.35651
Coverage: 10 km radius
Primary hazard: Periyar river flooding and road inundation
Assets: rescue boats, ambulances, evacuation trucks
```

The tracked boundary is `data/boundaries/aluva-periyar-pilot.geojson`. It is WGS84/EPSG:4326 and consists of a closed 64-segment geodesic polygon.

Raw OSM data has been downloaded locally to:

```text
data/raw/osm/aluva-periyar-pilot.osm.json
```

That file is intentionally ignored by Git. It contains raw OSM ways/nodes and is **not yet routable graph data**.

### Regeneration commands

```bash
cargo run --bin generate_boundary -- \
  --name aluva-periyar-pilot \
  --lat 10.10816 --lon 76.35651 \
  --radius-km 10 \
  --output data/boundaries/aluva-periyar-pilot.geojson
```

```bash
cargo run --bin download_osm -- \
  --boundary data/boundaries/aluva-periyar-pilot.geojson \
  --output data/raw/osm/aluva-periyar-pilot.osm.json \
  --endpoint https://overpass.kumi.systems/api/interpreter
```

For a small bounded pilot, Overpass is acceptable. For larger areas or recurring regional imports, use archived Geofabrik OSM PBF extracts and version the source dataset.

### Local setup (native, no containers)

The development machine has no GPU and no Docker. Services run natively:

```bash
# Neo4j 5 (bolt :7687, browser :7474) and Redis (:6379) as system services
sudo systemctl start neo4j redis      # service names may differ by distro
redis-cli ping                         # -> PONG

# Triage inference: llama.cpp on CPU (OpenAI-compatible, :8000)
llama-server -m <qwen3-4b-gguf> --port 8000 -np 1   # -np = INFERENCE_MAX_CONCURRENCY
```

Configure `.env` from `.env.example` (`INFERENCE_BACKEND=llama-cpp`). CPU inference is slow; size `STREAM_CLAIM_IDLE_MS` to about 3x the measured p99 extraction latency.

### New pipeline (core rebuild, in progress)

```bash
# Create the operational data model next to the legacy one (idempotent)
cargo run --bin migrate_ops

# Run one role per process (endpoints: /healthz, /readyz, /metrics)
cargo run --bin crisisgraph -- --role api        # :3100 (also serves the operations console: http://localhost:3100)
cargo run --bin crisisgraph -- --role triage     # :9101
cargo run --bin crisisgraph -- --role matcher    # :9102
cargo run --bin crisisgraph -- --role graph      # :9103
cargo run --bin crisisgraph -- --role relay      # :9104
cargo run --bin crisisgraph -- --role driver     # :9105, simulated responders (demo only)

# Submit a report (request_id is required; a retry with the same request_id
# returns the original 202 with `idempotent-replayed: true`)
curl -X POST localhost:3100/api/v1/reports -H 'content-type: application/json' \
  -d '{"request_id":"<uuid>","device_id":"<device>","raw_text":"...","latitude":10.11,"longitude":76.35,"accuracy_m":20}'
curl localhost:3100/api/v1/reports/<report_id>

# Acceptance tests (live Neo4j + Redis; triage tests use a scripted engine,
# plus one real-engine test that runs only when llama.cpp answers on :8000)
cargo test --test ops_model_tests
cargo test --test intake_relay_tests
cargo test --test triage_worker_tests
cargo test --test graph_hazard_tests
cargo test --test matcher_tests
cargo test --test console_api_tests
cargo test --test dispatcher_commands_tests
cargo test --test mission_tests
cargo test --test observability_tests

# Batch-quality bench (read-only; real graph and fleet)
cargo run --release --bin bench_matcher -- --sizes 10,25,40,60 --seeds 20 --seed 2026
```

Triage workers (`--role triage`) consume `sos:extract` (consumer group `triage-workers`), save the extraction on the report before deduplicating (so a redelivery never calls the model twice), then in one transaction either attach a same-device follow-up (D-2) or create an `:OpsIncident` (`TRIAGE_APPROVED` + queued `ROAD_PICKUP` task, or `REVIEW_REQUIRED`). Nearby open incidents are flagged `POSSIBLE_DUPLICATE` (D-3), never merged. When extraction fails or the inference circuit breaker is open, the report still becomes a `REVIEW_REQUIRED` incident for manual triage. With a self-hosted backend (llama.cpp or vLLM), the role first sends one warm-up request (real system prompt, `max_tokens: 1`, its own 300 s timeout) so the prompt is cached before the first report, and `/readyz` waits for it (`model_warm`). A failed warm-up only logs a warning. On the laptop a warm extraction takes about 20 s, and a cold one about 54 s (`ml/reports/local-inference-optimisation.md`, Finding 6). On one T4, vLLM with the guidance grammar library (`--structured-outputs-config '{"backend":"guidance"}'`) serves about 39 extractions/min at 32 in flight, with outputs identical to the default library (`ml/reports/qwen3-4b-sft-v4-serving-t4.md`).

Since M8 the matcher also applies **group plans** (T8, `src/matcher/groups.rs`, `src/db/ops_repo/groups.rs`). A queued road pickup that cannot be served as requested is fixed automatically and the dispatcher is alerted. Two cases count:

- **No route:** no vehicle of the requested type can reach the victims from any depot, e.g. trucks on `hgv=no` lanes.
- **Too big:** the group is larger than one vehicle of that type.

The choice is `models::group_plan`, decided on graph reachability only, never by the LLM:

- ambulances for a medical need, else one truck, else ambulances;
- as many vehicles as the group needs.

Each task carries its own `persons`, used for seats and drop-off beds. The alerts are `ASSET_SWITCHED` and `GROUP_SPLIT`. If a planned incident's headcount later rises, vehicles are added. A pickup that no road type can reach still goes to the boat-then-road check. Tasks no vehicle could ever carry never take a slot in the matcher's window.

**Dispatch readiness** (since M8, owner decision) is one rule for both gates (`models::blocking_review_reasons`, used by the pipeline stage and `triage_is_dispatch_ready`):

- Once the victims are located (F-1), our own satisfied checks no longer block: "no place named", missing hazard details, a missing headcount.
- A missing vehicle type (F-2) and any doubt the model raises in its own words still go to a person.
- The extractor retries `429` and `503`, then fails closed.

The measured results, with every finding and fix, are in `docs/load-test-results.md`. It is generated by `surge page` from `docs/results/*.json` plus `docs/results/findings.md`.

Since M8 the pipeline is **observable and load-tested**. Every role exports metrics from pipeline doc §4.11, including:

- stream length, pending count, oldest pending age and dead letters (`src/runtime/streams.rs`);
- time to assignment and queue wait by tier (histograms);
- reservation conflicts blocked, by source;
- extraction failures by reason;
- commands by result.

`ops/` holds the Prometheus scrape config and alert rules (`promtool check config|rules`) and the provisioned Grafana dashboard *CrisisGraph pipeline*. Run each tool as a user process: `ops/run-prometheus.sh` (:9090), `ops/run-grafana.sh` (:3300), `ops/run-jaeger.sh` (OTLP :4318, UI :16686). Their data goes in the git-ignored `.ops-data/`. Set `GRAFANA_URL` / `JAEGER_URL` on the api role for console links.

**Tracing** (`src/runtime/telemetry.rs`): intake starts a trace and stores its `traceparent` on the report and outbox event. The relay copies it into the stream message. Triage, the matcher (`matcher.assign`) and the driver continue it, and incidents and tasks keep it, so one SOS is one trace in Jaeger.

**Load tests** (`cargo run --release --bin surge -- run|cleanup|page`; results in `docs/results/*.json` and `docs/load-test-results.md`):

- The tool posts seeded reports on real junctions at stepped rates, in English, Manglish and Malayalam, with neighbours (D-3) and repeat callers (D-2). It floods roads mid-run (T7), waits for the pipeline to drain, then records throughput, latencies by tier, queue lag, verification failures and blocked overbooking.
- `surge cleanup --run-id <id>` removes everything the run created and restores vehicles, beds and hazards.
- **Run A** uses `INFERENCE_BACKEND=simulated`. The report carries its ground truth as a `#sim{…}` tag, returned after a seeded log-normal delay, so the run measures CrisisGraph rather than the model. Results are labelled "extraction: simulated".
- **Run B** uses `INFERENCE_BACKEND=openai` (Gemini's OpenAI-compatible endpoint) with `INFERENCE_MAX_RPM`; 429s are retried.
- Hosted models only ever receive **synthetic** surge reports, never the frozen test sets.

Since M6b victims **cut off by flood water** get a boat-then-road mission. When a queued road pickup's junction is unreachable by road from every depot, the matcher plans on the shared snapshot from each boat start (`RoutingGraph::multimodal`, flooded segments as water, timed at `BOAT_SPEED_KMH`). A terminal or island plan converts the task, in one transaction, into a QUEUED `WATER_EXTRACTION` (boat, towed by road to the staging point) and a `PENDING_TRIGGER` `ROAD_EVACUATION`. The road leg uses the requested road vehicle, else an ambulance for 4 or fewer people or a medical need, else a truck. The conversion raises a `MISSION_CONVERTED` alert. A fragmented cut escalates `NO_SURFACE_ROUTE`. The water leg is stored and shown as an **unverified estimate**; only road legs are verified live, and only road legs reserve drop-off beds. The road leg is queued on "victim aboard" or "send now" (`POST /api/v1/tasks/{id}/trigger`, T6), or automatically when the boat task completes. The boat then waits at the staging point. A dispatcher can cancel the plan (`POST /api/v1/incidents/{id}/mission/cancel`): the incident goes back to a road pickup that is never converted again. The simulated driver reports victim aboard halfway through a boat mission.

Since M6a the dispatcher can **merge** possible duplicates (`POST /api/v1/incidents/{survivor}/merge`, T4: exactly one vehicle stays sent; the survivor keeps its own, else the absorbed incident's assignment moves over; headcount = the larger, urgency only raised, the absorbed incident becomes `MERGED` with `MERGED_INTO`), **split** a wrongly attached follow-up into its own `REVIEW_REQUIRED` incident (`/split`), and change **vehicle status** (`POST /api/v1/vehicles/{id}/status`: out of service and back; out of service on a task is a breakdown that re-queues the task and frees its beds). All are buttons in the console. The **driver** role is a simulated responder for demos only: it completes assigned tasks through the task-complete command after `(2 × eta + SIM_ON_SCENE_SECS) × SIM_TIME_SCALE` seconds, with replay-safe request IDs, so several drivers or a restart never complete twice.

Since M7 the operations console runs on the new pipeline: the api role serves `web/` at `http://localhost:3100` (Overview, Intake, Incidents, Hazards, Fleet & beds, Alerts). Its API (`src/api/console.rs`) has open reads (`/api/v1/incidents`, `/incidents/{id}`, `/incidents/{id}/route`, `/vehicles`, `/shelters`, `/queue`, `/alerts`, `/hazards/active`, `/segments/near`, `/system`), two token-protected dispatcher commands pulled forward from M6 (`POST /api/v1/incidents/{id}/review`: approve with corrected facts and a location, creating the queued task, or reject; `POST /api/v1/tasks/{id}/complete`: T5, the vehicle becomes available at the drop-off), and `POST /api/v1/dispatch` (submit and wait up to `DISPATCH_WAIT_SECS`). Commands are version-checked, role-checked and replay-safe by `request_id` (`:Command` records). Live updates are server-sent events (`GET /api/v1/events`, `src/api/events.rs`) read from `ops:events` on a dedicated Redis connection, with `Last-Event-ID` replay; the console drops duplicates by `event_id` and ignores stale versions. Paste an operator token in the console's side rail to enable actions. The legacy console moved to `web/legacy/` and is still served by `server` on :3000. Merge/split, vehicle status and the simulated driver were added in M6a (below).

Since M5 the matcher role (`src/matcher/`) assigns vehicles: one leader per dataset (lease `matcher:{dataset}` + fencing token) runs a cycle every 2 s, or 250 ms after an event that changes tasks, vehicles or hazards. It takes the first tasks from the priority queue `alloc:queue:{dataset}` (tier + aging, rebuilt from Neo4j), builds a travel-time cost table on its own graph snapshot (one search per depot and asset type, in parallel), and assigns per strict tier with the Hungarian algorithm. Each pair runs T2: hold → route → live `verify_path_segments` → fenced commit with drop-off beds (nearest shelter with enough free beds; none → still dispatched with `DROPOFF_UNRESOLVED`). A failed check releases the hold, refreshes the overlay and retries; three failures, a headcount no vehicle can carry, or a wait past the tier threshold raise `TASK_ESCALATED` once. A hold left uncommitted by a crashed or fenced matcher is resumed with the same reservation. Run two matcher processes (`--port 9102`, `--port 9112`) for failover.

Since M4 hazards are `:Hazard` records, never properties on road segments: every change is transaction T7 (record + `overlay_version` + 1 + audit + `HAZARDS_CHANGED`), allowed for `HAZARD_VERIFIER` / `EOC_ADMIN` through `POST /api/v1/hazards` and `/api/v1/hazards/clear` on the api role, authenticated with operator tokens (`OPERATOR_TOKENS`, SHA-256 hashes; none configured means 401). The graph role loads the baseline graph once (`src/graph/`, `GraphSnapshot`), swaps the hazard overlay atomically on `HAZARDS_CHANGED` (plus a periodic refresh), and `/readyz` waits for the snapshot. Routes carry the `overlay_version` they used; verification stays live. Run `cargo run --bin migrate_ops` once to move any old road-property hazards into records.

Since M3b the extractor uses schema V3 (`victim_profile`) with prompt v3.2, served with the SFT-V4 adapter (`llama-server ... --lora ml/artifacts/qwen3-4b-triage-sft-v4/adapter-f16.gguf`). Urgency is derived in Rust (`src/models/victim.rs`, urgency-rules-v5, checked against `ml/data/urgency-rules-v5.jsonl`); the model's `suggested_urgency` can only raise it. CRITICAL incidents also emit `INCIDENT_CRITICAL`; UNKNOWN incidents are dispatched at URGENT rank and listed for dispatcher callback. Finding F-1: the caller's GPS locates the victims only when the caller is on scene (`src/ingestion/location_policy.rs`); remote callers are located at the described place, or sent to review. Finding F-2: a missing asset is never guessed; a category-based `suggested_asset` is shown to the dispatcher. Unprocessable messages go to `sos:extract:dead`. New-pipeline incidents use the `:OpsIncident` label until the legacy console is removed.

The relay is leader-elected (Redis lease `lease:outbox-relay` with a fencing token checked in Neo4j via `:LeaseFence`), publishes `REPORT_RECEIVED` to `sos:extract` and other events to `ops:events`, and delivers at least once: consumers must discard duplicates by `event_id`.

The legacy `server` binary (:3000) keeps working until the new pipeline replaces it. Both share shelter bed counters (`capacity` / `current_occupancy`); vehicles in the new model are individual `:Vehicle` records, and reservations are protected by uniqueness constraints on `active_vehicle_key` / `active_task_key`.

## Geospatial data model

Separate immutable/rebuildable baseline geography from mutable operational information:

```text
Baseline road network                        Operational overlay
---------------------                        -------------------
OSM geometry, OSM IDs and tags               field/dispatcher hazard reports
road class, direction, access                FLOODED / BLOCKED / OPEN state
bridge/tunnel/surface                        valid interval, confidence
segment length and base travel time          reporter/source and verifier
junction coordinates                         audit history
```

A directed road segment must preserve at least:

```text
segment_id, osm_way_id, from_junction_id, to_junction_id,
geometry, length_m, road_class, oneway, access flags,
base_travel_time_s, baseline_status, source_version
```

Do not mutate baseline map data in response to an incident. Apply hazards as temporal overlay events with `valid_from`, `valid_until`, confidence, source, and optional dispatcher verification.

## Completed milestone: OSM normalization pipeline

The OSM normalization stage is implemented and verified (`src/geospatial/mod.rs` and `src/bin/normalize_osm.rs`). It reads raw OSM JSON and emits inspectable, versioned outputs:

```text
data/processed/aluva-periyar-pilot/
  junctions.geojson          # 21,256 Point features
  road_segments.geojson      # 48,626 directed LineString segments (5,139 km)
  import_report.json         # validation (0 dangling, 0 duplicate, 0 zero-length)
```

Run command:
```bash
cargo run --bin normalize_osm -- \
  --input data/raw/osm/aluva-periyar-pilot.osm.json \
  --boundary data/boundaries/aluva-periyar-pilot.geojson \
  --output-dir data/processed/aluva-periyar-pilot \
  --source-version "aluva-periyar-pilot-2026-09-04" \
  --boundary-policy intersect
```

## Completed milestone: Directed routing solver & Neo4j baseline loader

1. **Directed Petgraph solver:**
   - Upgraded `DeterministicPathfinder` to `petgraph::graph::DiGraph<String, RoutingEdge>`.
   - Full support for one-way constraints, vehicle accessibility (`Ambulance`, `EvacTruck`, `RescueBoat`), cost objectives (`FastestTime`, `ShortestDistance`), and dynamic hazard avoidance.
2. **Neo4j directed baseline loader:**
   - Implemented `src/db/baseline_loader.rs` and `src/bin/load_baseline_graph.rs`.
   - Batch ingests junctions and directed road segments using Cypher `UNWIND` transactions.
   - Built uniqueness constraints and indexes on `(:Junction {id, dataset})` and `[:CONNECTS_TO {segment_id, dataset}]`.
   - Ingested 21,256 junctions and 48,626 directed segments in 7.1s.
3. **Dynamic operational overlay & verification:**
   - `StateManager::get_passable_directed_subgraph` extracts passable directed edges filtering out active hazard closures.
   - `StateManager::apply_segment_hazard` and `clear_operational_hazards` support temporal overlays (since M4 they write `:Hazard` records through T7).
   - Verified via integration tests in `tests/neo4j_baseline_tests.rs`.

Run command:
```bash
cargo run --bin load_baseline_graph -- \
  --junctions data/processed/aluva-periyar-pilot/junctions.geojson \
  --segments data/processed/aluva-periyar-pilot/road_segments.geojson \
  --dataset aluva-periyar-pilot \
  --clean
```

## Completed milestone: Pipeline integration & dispatcher verification

1. **Pipeline routing stage upgrade:**
   - Upgraded `src/pipeline/stages.rs` to stream directed graph snapshots from Neo4j (`get_passable_directed_subgraph`) through `DeterministicPathfinder::find_directed_route`.
   - Wired GPS coordinate parsing and nearest-neighbor junction snapping (`SpatialResolver::parse_coordinates`, `StateManager::find_nearest_junction`).
   - Mapped prominent Aluva landmarks (*Railway Station, Town Hall, UC College, Manappuram Shiva Temple, Pump Junction, Bank Junction, Taluk Hospital*) directly to Neo4j junctions.
   - Seeded 4 operational relief hubs and vehicle depots across Aluva in Neo4j (`S_ALUVA_TOWNHALL`, `S_UC_COLLEGE`, `S_MANAPPURAM_DEPOT`, `S_TALUK_HOSPITAL`).
2. **Independent dispatcher verification stage:**
   - Implemented `StateManager::verify_path_segments` to independently cross-check 100% of traversed OSM road segments against active operational closures in Neo4j.
   - Fails closed with `DispatchStatus::EscalateHumanDispatcher` if any segment is hazardous or inaccessible (e.g. oversized trucks on service alleys).
   - Validated via end-to-end integration tests in `tests/aluva_pipeline_tests.rs`.

## Safety and implementation rules

1. Never use an LLM for path calculation or final road-safety certification.
2. Fail closed: no confirmed route, invalid graph, unsafe segment, or incomplete allocation must escalate.
3. Use strict typed structs for inter-stage contracts and validate external data.
4. Keep all I/O asynchronous with Tokio.
5. Guard resource mutation atomically: in the new pipeline, one fenced matcher decides and uniqueness constraints (`active_vehicle_key` / `active_task_key`) make overbooking impossible; the legacy server uses a Redis ownership-checked lock and an atomic Neo4j reservation.
6. Treat citizen/LLM hazard extraction as a candidate event. Verified field/dispatcher events are the route-blocking authority.
7. Before dispatch, independently verify every traversed segment against current operational state (`verify_path_segments`).
8. Do not use the demo Neo4j seed graph together with the imported Aluva graph.
9. Do not commit `.env`, raw downloaded map data, build output, or operational exports.
10. When changing the data model or importer, update `docs/geospatial-data-engineering.md`.

## Verification baseline

```bash
cargo test --lib                       # unit tests (urgency vectors, solver, assignment, config)
cargo test --test ops_model_tests      # live Neo4j/Redis suites below skip if unreachable
cargo test --test intake_relay_tests
cargo test --test triage_worker_tests
cargo test --test graph_hazard_tests
cargo test --test matcher_tests
cargo test --test console_api_tests
cargo test --test dispatcher_commands_tests
cargo test --test mission_tests
cargo test --test observability_tests
cargo test --test neo4j_baseline_tests
cargo test --test aluva_pipeline_tests # legacy end to end; needs llama.cpp on :8000
```

Live tests run on their own `TEST-<uuid>` datasets and clean up after themselves; a failed run can leave its test data behind, so check for `TEST-` nodes after a failure.
