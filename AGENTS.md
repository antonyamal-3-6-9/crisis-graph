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
Shelter and asset allocation (Redis lock + atomic Neo4j mutation)
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
- **Redis/Valkey + redis-rs:** distributed locks now; Redis Streams/consumer groups are a planned intake mechanism.
- **vLLM-compatible HTTP endpoint + reqwest:** SOS triage extraction. The default is localhost, but the endpoint is configurable and need not be local.
- **Petgraph:** deterministic shortest-path logic. It currently uses an undirected prototype graph and must become directed before real-road routing.
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
├── docs/
│   ├── geospatial-data-engineering.md
│   └── handoff-2026-09-04.md
├── src/
│   ├── bin/
│   │   ├── generate_boundary.rs       # generates pilot GeoJSON boundary
│   │   └── download_osm.rs            # downloads raw OSM road data
│   ├── db/
│   ├── ingestion/
│   ├── models/
│   ├── pipeline/
│   ├── solver/
│   ├── config.rs
│   ├── lib.rs
│   └── main.rs
└── tests/
```

## Current pipeline

The implementation passes a typed `CrisisState` (“clipboard”) through four stages:

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

## Next milestone: OSM normalization, not live pipeline changes

Implement a normalizer/import stage that reads the raw OSM JSON and writes inspectable, versioned outputs:

```text
data/processed/aluva-periyar-pilot/
  junctions.geojson
  road_segments.geojson
  import_report.json
```

The normalizer must:

1. Parse raw OSM nodes and vehicle-eligible highway ways.
2. Preserve source IDs/tags and source version.
3. Split roads at intersections and other topology-changing nodes.
4. Create **directed** segments, respecting `oneway` and relevant access restrictions.
5. Store geometry, endpoint IDs, and calculated physical length.
6. Validate dangling endpoints, duplicates, counts, and known one-way behaviour.
7. Keep baseline data rebuildable and completely separate from incident overlays.

Turn restrictions require edge-based route state (or equivalent inbound-segment state); a plain junction-to-junction graph cannot represent prohibited turns.

## Safety and implementation rules

1. Never use an LLM for path calculation or final road-safety certification.
2. Fail closed: no confirmed route, invalid graph, unsafe segment, or incomplete allocation must escalate.
3. Use strict typed structs for inter-stage contracts and validate external data.
4. Keep all I/O asynchronous with Tokio.
5. Guard resource mutation with a Redis ownership-checked lock and atomic Neo4j reservation.
6. Treat citizen/LLM hazard extraction as a candidate event. Verified field/dispatcher events are the route-blocking authority.
7. Before dispatch, independently verify every traversed segment against current operational state; the current verifier does not yet do this.
8. Do not use the demo Neo4j seed graph together with the imported Aluva graph.
9. Do not commit `.env`, raw downloaded map data, build output, or operational exports.
10. When changing the data model or importer, update `docs/geospatial-data-engineering.md` and the current handoff document.

## Verification baseline

```bash
cargo test --test integration_tests
```

Existing tests cover basic lexical resolution and prototype pathfinding. Add integration tests with Neo4j/Redis, directed-road tests, closure/expiry tests, and concurrent allocation tests as the system matures.
