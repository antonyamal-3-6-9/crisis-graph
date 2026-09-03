# AGENTS.md — CrisisGraph Engine Agent Specifications (Rust Engine)

## 1. Project Overview & Mission Statement
**CrisisGraph** is a mission-critical, air-gapped emergency logistics and evacuation routing engine implemented in **Rust** designed for real-time disaster scenarios (flash floods, cyclones, landslides).

### Core Failure Modes CrisisGraph Solves
1. **Static Map Invalidation:** Standard maps (Google Maps/OSM) assume fixed topology; floodwaters and debris render roads impassable dynamically.
2. **Vector RAG Blindness:** Vector search cannot compute topological reachability or shortest path mathematics.
3. **LLM Hallucinations:** Generative LLMs hallucinate non-existent connectivity and cannot reliably calculate shortest paths.
4. **WAN Infrastructure Outages:** Cloud API dependencies (OpenAI, Anthropic) fail when terrestrial fiber is cut. CrisisGraph runs fully air-gapped on local hardware.
5. **Runtime Concurrency Bottlenecks:** Built in Rust with Tokio async runtime for high-throughput, low-latency concurrent SOS triage without GIL or runtime GC spikes.

---

## 2. System Architecture & Tech Stack

```
[Citizen Raw SOS Stream]
           │
           ▼
  [Redis Message Queue] ──► (In-memory buffering & Concurrency Locks)
           │
           ▼
 [Local vLLM Subprocess] ──► (Qwen 2.5 7B / Llama 3.2 3B + Guided JSON Logit Masks over HTTP)
           │
           ▼
    [Neo4j Graph DB]    ──► (Bi-temporal Edge Decay + Shelter Asset Nodes via Bolt/neo4rs)
           │
           ▼
[Vanilla Rust Pipeline]
   ├── Stage 1: Triage Extraction (vLLM JSON API Client)
   ├── Stage 2: Asset & Shelter Allocation (Redis Mutex + Atomic Cypher)
   ├── Stage 3: Deterministic Pathfinder (Petgraph A* / Dijkstra over pass-only subgraph)
   └── Stage 4: Safety Verifier & Tactical Brief Generator
           │
           ▼
[Actionable Dispatch Order to Responders (NDRF / Field Command)]
```

### Technology Matrix
* **Language & Runtime:** Rust (2021 Edition) on Tokio Async Runtime.
* **Model Serving:** Separate local vLLM process (`http://localhost:8000/v1`) with Guided JSON logit masking / schema enforcement, consumed via async `reqwest`.
* **Graph Database:** Neo4j via `neo4rs` (Bi-temporal property graph for road networks, shelters, and dynamic hazard states).
* **Concurrency & Queue:** Redis / Valkey via `redis-rs` (Stream buffer + Distributed Mutex for atomic asset reservation).
* **Deterministic Routing:** `petgraph` (Dijkstra / A* graph algorithms over filtered active edges).
* **Orchestrator:** Vanilla Rust state-machine implementing the typed "Clipboard" pattern (Zero external Python/LangGraph overhead).
* **Data Validation & Serde:** `serde`, `serde_json`, `chrono`, `thiserror`.

---

## 3. Directory Layout & Module Responsibilities

```
crisis-graph/
├── Cargo.toml                  # Rust project manifest and crate dependencies
├── .env                        # Environment configurations (ports, credentials, model endpoints)
├── src/
│   ├── main.rs                 # CLI entry point and event stream processor
│   ├── lib.rs                  # Core library exports
│   ├── config.rs               # Central configuration loader (dotenvy + env vars)
│   ├── models/                 # Strongly-typed Rust data structures
│   │   ├── mod.rs
│   │   ├── state.rs            # CrisisState and PipelineState (The Clipboard)
│   │   ├── triage.rs           # SOS input, triage result, and hazard models
│   │   └── dispatch.rs         # Shelter, Asset, and Tactical Brief schemas
│   ├── db/                     # Database clients & state management
│   │   ├── mod.rs
│   │   ├── neo4j.rs            # Neo4j client connection pool and query executors
│   │   ├── redis_client.rs     # Redis connection pool & distributed locks
│   │   ├── state_manager.rs    # Temporal edge mutation and atomic shelter capacity
│   │   └── seed.rs             # Seed initial junction network and shelter nodes
│   ├── ingestion/              # Extraction and spatial resolution
│   │   ├── mod.rs
│   │   ├── extractor.rs        # HTTP client for separate vLLM process (guided JSON)
│   │   └── spatial_resolver.rs # Landmark to Junction ID resolver (lexical/fuzzy mapping)
│   ├── solver/                 # Deterministic routing
│   │   ├── mod.rs
│   │   └── pathfinder.rs       # Petgraph A*/Dijkstra solver over passable Neo4j edges
│   └── pipeline/               # Vanilla Rust state-machine orchestrator
│       ├── mod.rs
│       ├── stages.rs           # Stage 1-4 individual execution logic
│       └── runner.rs           # Pipeline orchestrator loop (The Clipboard Pattern)
└── tests/
    └── integration_tests.rs    # Test suite for 0% illegal traversals and route verification
```

---

## 4. Multi-Stage Pipeline Workflow (The "Clipboard" Pattern)

The system passes an explicit, typed state (`CrisisState`) sequentially across 4 core stages:

```
[Raw SOS Input] ──► (Stage 1: Triage Extraction)
                              │
                              ▼
                    (Stage 2: Asset & Shelter Allocation) [Redis Mutex]
                              │
                              ▼
                    (Stage 3: Deterministic Pathfinder) [Petgraph]
                              │
                              ▼
                    (Stage 4: Safety Verifier & Brief Generator)
                              │
                              ▼
                    [Verified Dispatch Brief]
```

### 4.1 Stage 1: AI Listener (Triage & Hazard Extraction)
* **Role:** Parse unstructured natural language SOS alerts into strict `TriageReport` structs.
* **Engine:** Consumes local vLLM server via async HTTP with guided JSON schema.
* **Outputs:**
  * `victim_location`: Landmark or Junction identifier.
  * `headcount`: Number of trapped individuals.
  * `required_asset`: `RescueBoat`, `Ambulance`, `EvacTruck`, etc.
  * `hazards`: List of road conditions (e.g., `road_segment: "J3-J6"`, `status: "FLOODED"`, `duration_hours: 6`).
* **Side-Effect:** Invokes `state_manager` to mutate affected edges in Neo4j with active weights (`999999.0`) and expiration timestamps (`valid_until`).

### 4.2 Stage 2: Database Clerk (Asset & Shelter Allocation)
* **Role:** Query and reserve nearest suitable shelter and rescue assets without race conditions.
* **Concurrency Safeguards:**
  * Uses Redis distributed locks keyed on shelter/asset IDs.
  * Executes atomic Cypher mutations asserting remaining capacity (`capacity - current_occupancy >= headcount`).
* **Outputs:**
  * `assigned_shelter_id`: e.g., `"S1"`.
  * `assigned_shelter_junction`: e.g., `"J1"`.
  * `assigned_asset`: e.g., `"RESCUE_BOAT_02"`.

### 4.3 Stage 3: Route Calculator (Deterministic Pathfinder)
* **Role:** Compute mathematically valid, non-hallucinated shortest paths from responder/shelter to victim.
* **Engine:** Petgraph (A* / Dijkstra) over Neo4j dynamic edge subgraph.
* **Rule:** Filters out edges where `status != 'OPEN'` AND `valid_until > datetime()`.
* **Outputs:**
  * `computed_path`: Ordered list of junctions (e.g., `["J1", "J4", "J5", "J2", "J3"]`).
  * `total_distance_km`: Sum of physical segment lengths.
  * `detour_reason`: Explanation if primary direct routes are blocked.

### 4.4 Stage 4: Safety Verifier & Tactical Brief Generator
* **Role:** Enforce strict guardrail assertions before dispatching to responders.
* **Assertions:**
  1. Path is contiguous and starts at responder location and ends at victim junction.
  2. Zero traversed edges are marked `FLOODED` or `BLOCKED`.
  3. Shelter capacity and rescue asset are confirmed allocated.
* **Outputs:**
  * Formatted tactical dispatch brief for NDRF/SDRF field command.
  * Status set to `Status::RoutedVerified` or `Status::EscalateHumanDispatcher`.

---

## 5. Data Engineering Specifications

### 5.1 Bi-Temporal Graph Edge Decay (Zero Polling Loops)
Edges represent physical roads with transient hazards:
* `status`: `"OPEN"` | `"BLOCKED"` | `"FLOODED"`
* `base_distance_km`: Static physical length (f64).
* `active_weight`: Default is `base_distance_km`. Set to `999999.0` when impassable.
* `valid_until`: Expiration timestamp (`DateTime<Utc>`).

#### Passable Graph Query:
```cypher
MATCH (a:Junction)-[r:CONNECTS_TO]->(b:Junction)
WHERE r.status = 'OPEN' OR r.valid_until < datetime()
RETURN a.id AS source, b.id AS target, r.base_distance_km AS weight;
```

### 5.2 Atomic State Mutation & Race Condition Protection
```cypher
MATCH (s:Shelter {id: $shelter_id})
WHERE (s.capacity - s.current_occupancy) >= $headcount AND s.boats_available >= 1
SET s.current_occupancy = s.current_occupancy + $headcount,
    s.boats_available = s.boats_available - 1,
    s.last_updated = datetime()
RETURN s.id AS shelter_id, s.current_occupancy AS occupancy;
```

---

## 6. Implementation Guidelines for Rust Engine
1. **Never use LLMs for path calculation:** Shortest-path routing must strictly be executed by Petgraph algorithms in Rust.
2. **Strict Strong Typing:** All inter-stage data contracts must be validated Rust structs with Serde annotations.
3. **Async & Safe Concurrency:** All I/O (Neo4j, Redis, vLLM HTTP) must be asynchronous and non-blocking using Tokio.
4. **No Polling for Road Expirations:** Rely on Cypher temporal conditions (`valid_until < datetime()`) during graph extraction.
5. **Guarded Resource Mutations:** Always wrap shelter and asset updates inside Redis mutex locks to prevent race conditions during concurrent emergency bursts.
6. **Air-Gapped Operation:** All model inference must use local vLLM instances; no external WAN/cloud API calls.
