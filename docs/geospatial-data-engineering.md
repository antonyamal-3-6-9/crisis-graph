# Geospatial Data Engineering Design

## Purpose

CrisisGraph needs a trustworthy, routable representation of a bounded emergency-response area. Routing is only as safe as the road topology and operational status data supplied to it.

This design separates a stable **baseline road network** from a fast-changing **operational hazard overlay**.

> CrisisGraph may use local models for resilient SOS extraction, but it is not an offline-only product. Real deployments can depend on connected field reporting, GIS feeds, GPS/AVL, weather and satellite services, and command-centre systems.

## Pilot scope

Start with one named municipality or a 10–20 km response radius around Aluva/Kochi. Do not begin with a state- or country-wide graph.

Pilot success means the team can reproducibly import the area, calculate valid directed vehicle routes, apply a verified closure, and prove that no resulting route uses the closed segment.

## Baseline data and live operations

```text
Baseline geographic network                 Live operational overlay
---------------------------                -------------------------
OSM road geometry and tags                 field hazard reports
one-way/access restrictions                flood / debris / closure status
road class, bridge, surface                validity window + confidence
junction coordinates                       source, timestamp, verifier
shelters and base locations                asset availability and occupancy
```

The effective graph for a dispatch request is derived from both layers. Operational events must not overwrite or destroy baseline geography.

## Sources and provenance

### Baseline roads

Use OpenStreetMap (OSM) for the initial road network. Download an `.osm.pbf` regional extract from Geofabrik instead of depending on a public query endpoint for bulk imports.

- [Geofabrik India extract](https://download-ext2.geofabrik.de/asia/india.html)
- [OSM export and ODbL licence](https://www.openstreetmap.org/export)

For the pilot, include vehicle-appropriate OSM ways: `motorway`, `trunk`, `primary`, `secondary`, `tertiary`, `unclassified`, `residential`, and `service`. Consider `track` only after field verification. Exclude walking- and cycling-only paths unless an explicit emergency policy permits them.

Keep OSM element IDs, source tags, source dataset version, and import timestamp. This provenance is required to investigate unsafe or implausible routes.

### Operational sources

| Data | Example source | Treatment |
|---|---|---|
| Road topology | OSM or licensed government GIS | Baseline network |
| Shelters and rescue bases | Government list plus manual curation | Human-approved asset data |
| Flood/weather context | Government weather, satellite, or GIS feed | Risk signal, not automatic closure |
| Road blockage | Dispatcher-verified field report | Route-blocking event |
| Citizen report | App, SMS, radio, call-centre | Candidate event requiring confidence/review |
| Responder status | GPS/AVL or command-centre update | Live allocation/location state |

Bhuvan/NRSC geospatial services may be considered as contextual or corroborating information after selecting a documented product/API integration: <https://bhuvan-app1.nrsc.gov.in/api/>.

## From OSM ways to a routing graph

OSM stores roads as ordered `way` objects. CrisisGraph requires directed, traversable segments.

```text
OSM way:
node A -------- node B -------- node C

Routing graph:
A -> B
B -> A     (only when the road is two-way)
B -> C
C -> B     (only when the road is two-way)
```

Create junctions at intersections, way endpoints, and other topology-changing locations. Retain intermediate OSM shape points as segment geometry, not routing nodes.

Each directed `CONNECTS_TO` relationship needs at least:

```text
segment_id              stable internal ID
osm_way_id              source provenance
from_junction_id
to_junction_id
geometry                LineString or encoded polyline
length_m
road_class
oneway
maxspeed_kph            optional
surface                 optional
bridge / tunnel flags
access flags            motorcar, emergency, hgv
base_travel_time_s
baseline_status         OPEN / RESTRICTED / UNSUITABLE
source_version
```

The current Petgraph solver uses an undirected graph. Before real-road import, change it to a directed graph and preserve `oneway`, access constraints, and vehicle capability restrictions. Turn restrictions require an edge-based routing representation (or an equivalent state that includes the inbound segment); a plain junction graph cannot express “this turn is forbidden.”

- [OSM access tags](https://wiki.openstreetmap.org/wiki/Access_tags)
- [OSM turn restrictions](https://wiki.openstreetmap.org/wiki/Relation%3Arestriction)

## Storage architecture

```text
Raw OSM PBF / licensed GIS source
              |
              v
Versioned staging data (GeoJSON, GeoPackage, Parquet, or PostGIS)
              |
              v
Validated normalized directed routing graph
              |
              v
Neo4j baseline graph + operational overlay
              |
              v
Petgraph route snapshot for deterministic calculation
```

For a small portfolio implementation, versioned GeoJSON files plus a Rust importer are enough. At larger scale, use PostGIS or another geospatial staging store. In all cases, Neo4j must be rebuildable from versioned source data.

## Temporal hazard overlay

A flood report is an operational event, not a permanent road-map mutation.

```text
segment: J3 -> J6
event status: FLOODED
valid_from: ISO-8601 timestamp
valid_until: ISO-8601 timestamp
confidence: 0.80
source: field_report
verified_by: dispatcher ID or null
created_at: event timestamp
```

At route time, derive the effective state:

```text
if verified active closure exists       => not traversable
else if active low-confidence report    => avoid, penalize, or send for review
else if vehicle restriction applies     => not traversable for that asset
else                                    => traversable
```

This permits competing reports, expiry, audit history, and later review. A single mutable `status` property is acceptable for the prototype but not sufficient for an accountable operational system.

## Import pipeline

1. Define and version a polygon boundary for the pilot area (`data/boundaries/aluva-periyar-pilot.geojson`).
2. Acquire and archive the matching OSM input with date/version (`data/raw/osm/aluva-periyar-pilot.osm.json`).
3. Filter eligible road ways and relevant nodes using `crisis_graph::geospatial::OsmNormalizer`.
4. Split ways into directed segments at topology-changing nodes (intersections, way endpoints, self-loops).
5. Interpret road direction (`oneway=yes`, `-1`, roundabouts, motorways), access, and vehicle capability restrictions (`motorcar`, `emergency`, `hgv`).
6. Calculate segment physical length via Haversine great-circle summation and derive base travel time from speed limits or road class defaults.
7. Emit versioned staging artefacts:
   - `data/processed/aluva-periyar-pilot/junctions.geojson` (21,256 junctions)
   - `data/processed/aluva-periyar-pilot/road_segments.geojson` (48,626 directed segments, 5,139 km)
   - `data/processed/aluva-periyar-pilot/import_report.json` (provenance, network metrics, and validation report)
8. Validate graph integrity (zero dangling endpoints, zero duplicate IDs, zero zero-length segments, validated one-way behaviour).
9. Load the validated normalized graph into Neo4j as baseline directed road network.
10. Load shelters, rescue bases, and asset metadata separately from road data.
11. Rebuild or update the baseline graph only through this importer; apply incidents only as operational overlay events.

### Normalization execution command

```bash
cargo run --bin normalize_osm -- \
  --input data/raw/osm/aluva-periyar-pilot.osm.json \
  --boundary data/boundaries/aluva-periyar-pilot.geojson \
  --output-dir data/processed/aluva-periyar-pilot \
  --source-version "aluva-periyar-pilot-2026-09-04" \
  --boundary-policy intersect
```

## End-to-End Implementation Flow & Concrete Examples (Aluva Pilot)

This section provides the complete reference implementation flow showing how raw coordinates and OpenStreetMap data are transformed into a validated, directed routing graph.

```text
  Phase 1               Phase 2                 Phase 3 & 4                  Phase 5
┌─────────────┐       ┌──────────────┐       ┌─────────────────┐       ┌─────────────────┐
│  Boundary   │ ───>  │   Raw OSM    │ ───>  │  Normalization  │ ───>  │   Validation    │
│ Generation  │       │ Acquisition  │       │   & Topology    │       │    & Staging    │
└─────────────┘       └──────────────┘       └─────────────────┘       └─────────────────┘
 [GeoJSON Poly]        [16MB Raw JSON]        [Rust Normalizer]         [Staging GeoJSON]
```

---

### Step 1: Geodesic Boundary Formulation

#### 1. Rationale
Emergency response requires a strictly bounded catchment area. The pilot area is centered on **Aluva Railway Station** (`10.10816° N, 76.35651° E`) with a **10 km radius**, covering the Periyar river floodplain.

#### 2. Execution Command
```bash
cargo run --bin generate_boundary -- \
  --name aluva-periyar-pilot \
  --lat 10.10816 --lon 76.35651 \
  --radius-km 10 \
  --output data/boundaries/aluva-periyar-pilot.geojson
```

#### 3. Mathematical Basis
Uses WGS84 great-circle destination point equations ($R = 6,371,008.8\text{ m}$) stepping across 64 radial bearing segments ($2\pi / 64$):
$$\phi_2 = \arcsin(\sin\phi_1 \cos\delta + \cos\phi_1 \sin\delta \cos\theta)$$
$$\lambda_2 = \lambda_1 + \arctan2(\sin\theta \sin\delta \cos\phi_1, \cos\delta - \sin\phi_1 \sin\phi_2)$$

#### 4. Concrete Boundary Output (`data/boundaries/aluva-periyar-pilot.geojson`)
```json
{
  "type": "Feature",
  "properties": {
    "id": "aluva-periyar-pilot-v1",
    "name": "aluva-periyar-pilot",
    "centre": { "latitude": 10.10816, "longitude": 76.35651 },
    "radius_km": 10.0,
    "crs": "EPSG:4326",
    "segments": 64
  },
  "geometry": {
    "type": "Polygon",
    "coordinates": [
      [
        [76.356510, 10.198092],
        [76.365466, 10.197658],
        [76.374336, 10.196363],
        ...
        [76.356510, 10.198092]
      ]
    ]
  }
}
```

---

### Step 2: Raw OSM Data Extraction

#### 1. Execution Command
```bash
cargo run --bin download_osm -- \
  --boundary data/boundaries/aluva-periyar-pilot.geojson \
  --output data/raw/osm/aluva-periyar-pilot.osm.json \
  --endpoint https://overpass.kumi.systems/api/interpreter
```

#### 2. Overpass QL Query
Targeted all vehicle-eligible highway classes intersecting the pilot radius, plus their referenced nodes:
```text
[out:json][timeout:180];
way(around:10000,10.10816,76.35651)
  ["highway"~"^(motorway|motorway_link|trunk|trunk_link|primary|primary_link|secondary|secondary_link|tertiary|tertiary_link|unclassified|residential|service|track)$"];
out body;
>;
out skel qt;
```

#### 3. Concrete Raw OSM Data Snippet (`data/raw/osm/aluva-periyar-pilot.osm.json` — 16 MB)
Total elements: **117,396** (104,429 nodes, 12,967 ways).

```json
// Raw Node: Just geographic coordinates
{
  "type": "node",
  "id": 5883279139,
  "lat": 10.0302438,
  "lon": 76.3100927
}

// Raw Way: An ordered sequence of 35 node IDs with tags
{
  "type": "way",
  "id": 30921568,
  "nodes": [4889497750, 343714817, 343714816, ..., 5974232533],
  "tags": {
    "highway": "secondary",
    "name": "Aluva-Perumbavoor Road",
    "oneway": "no",
    "surface": "asphalt",
    "maxspeed": "45"
  }
}
```

---

### Step 3: Topology Splitting & Directional Normalization

#### 1. Why Raw OSM Cannot Be Routed Directly
- **Intermediate nodes are geometry, not junctions:** Most nodes in an OSM way define road curvature. Turning every node into a graph node explodes graph size with redundant edges and slows pathfinding.
- **Missing graph connectivity:** If Road A and Road B cross at Node $X$, but Node $X$ is buried in the middle of both ways, graph routing cannot turn from Road A onto Road B unless both ways are split at Node $X$.
- **Undirected representation:** OSM ways are ordered node sequences. A two-way road requires distinct directed edges ($A \to B$ and $B \to A$) with inverted coordinate geometry so GIS tools and dispatchers can visualize and route both directions independently.

```text
Raw OSM Way (30921568):
Node 1 ─────── Node 2 ─────── Node 3 ─────── Node 4 ─────── Node 5
(Start)        (Curve)     (Intersection)   (Curve)        (End)
   │                             │                           │
   ▼                             ▼                           ▼
(:Junction 1)             (:Junction 3)               (:Junction 5)

Directed Routing Edges Generated:
- Forward Segment 0: Junction 1 -> Junction 3 (Geometry: [Node 1, Node 2, Node 3])
- Reverse Segment 0: Junction 3 -> Junction 1 (Geometry: [Node 3, Node 2, Node 1])
- Forward Segment 1: Junction 3 -> Junction 5 (Geometry: [Node 3, Node 4, Node 5])
- Reverse Segment 1: Junction 5 -> Junction 3 (Geometry: [Node 5, Node 4, Node 3])
```

#### 2. Normalizer Execution
```bash
cargo run --bin normalize_osm -- \
  --input data/raw/osm/aluva-periyar-pilot.osm.json \
  --boundary data/boundaries/aluva-periyar-pilot.geojson \
  --output-dir data/processed/aluva-periyar-pilot \
  --source-version "aluva-periyar-pilot-2026-09-04" \
  --boundary-policy intersect
```

#### 3. Core Processing Pipeline ([`src/geospatial/mod.rs`](file:///run/media/amal/Store/Projects/crisis-graph/src/geospatial/mod.rs))
- **Junction Node Identification:** Nodes referenced at way endpoints, nodes referenced by $\ge 2$ ways, and self-intersecting loop nodes become `(:Junction)` instances. Out of 104,429 raw nodes, **21,256** were classified as true junctions.
- **Physical Length Calculation:** Great-circle Haversine summation along each polyline vertex:
  $$d = 2 R \arcsin\left(\sqrt{\sin^2(\Delta\phi/2) + \cos\phi_1\cos\phi_2\sin^2(\Delta\lambda/2)}\right)$$
- **Base Travel Time:** Derived as $t = d / (v_{\text{speed}} \cdot \frac{1000}{3600})$, using parsed `maxspeed` or road class fallback speeds.
- **Directional Segment Generation:**
  - Standard two-way: Forward (`fwd`) and reverse (`rev`) segment pairs with inverted LineString geometry.
  - `oneway=yes`, motorway, or roundabout: Forward only (`fwd`).
  - `oneway=-1` / `reverse`: Reverse only (`rev`).
- **Vehicle Access Filtering:** Infers permissions for `motorcar`, `emergency`, and `hgv`.

---

### Step 4: Staging Outputs & Schemas

The normalization stage writes three versioned artifacts into `data/processed/aluva-periyar-pilot/`:

#### 1. Routing Junctions (`junctions.geojson` — 8.4 MB, 21,256 features)
```json
{
  "type": "Feature",
  "id": "node/18306111",
  "geometry": {
    "type": "Point",
    "coordinates": [76.3867873, 10.192707]
  },
  "properties": {
    "junction_id": "node/18306111",
    "osm_node_id": 18306111,
    "degree": 5,
    "in_degree": 2,
    "out_degree": 3,
    "inside_boundary": true
  }
}
```

#### 2. Directed Road Segments (`road_segments.geojson` — 61 MB, 48,626 features)
Concrete forward segment with 5-point polyline geometry:
```json
{
  "type": "Feature",
  "id": "way/1000300905/seg/0/fwd",
  "geometry": {
    "type": "LineString",
    "coordinates": [
      [76.3210264, 10.0342687],
      [76.3214521, 10.0344530],
      [76.3216001, 10.0345560],
      [76.3216039, 10.0345937],
      [76.3215172, 10.0348148]
    ]
  },
  "properties": {
    "segment_id": "way/1000300905/seg/0/fwd",
    "osm_way_id": 1000300905,
    "from_junction_id": "node/5755272548",
    "to_junction_id": "node/9233321223",
    "road_class": "unclassified",
    "road_name": null,
    "oneway": false,
    "direction": "forward",
    "length_m": 101.3,
    "speed_kph": 30.0,
    "maxspeed_kph": null,
    "base_travel_time_s": 12.16,
    "surface": null,
    "bridge": false,
    "tunnel": false,
    "lanes": null,
    "access": {
      "emergency": true,
      "hgv": true,
      "motorcar": true
    },
    "baseline_status": "OPEN",
    "source_version": "aluva-periyar-pilot-2026-09-04"
  }
}
```

*(Its two-way counterpart `way/1000300905/seg/0/rev` was simultaneously created with `from: node/9233321223`, `to: node/5755272548`, and reversed coordinates).*

#### 3. Audit Report (`import_report.json` — 2.5 KB)
```json
{
  "report_generated_at": "2026-09-05T18:13:03.350618340+00:00",
  "source_version": "aluva-periyar-pilot-2026-09-04",
  "raw_elements_count": 117396,
  "total_junctions_count": 21256,
  "total_directed_segments_count": 48626,
  "forward_segments_count": 24861,
  "reverse_segments_count": 23765,
  "oneway_segment_count": 1098,
  "twoway_segment_pair_count": 23764,
  "total_network_length_km": 5139.27,
  "mean_segment_length_m": 105.7,
  "validation": {
    "dangling_endpoints_count": 0,
    "duplicate_segment_ids_count": 0,
    "zero_length_segments_count": 0,
    "self_loops_count": 45,
    "known_oneway_validated": true,
    "passed": true
  }
}
```

---

### Step 5: Automated Integrity & Ground-Truth Validation

[`tests/geospatial_tests.rs`](file:///run/media/amal/Store/Projects/crisis-graph/tests/geospatial_tests.rs) verifies graph invariants against the actual generated staging files:

1. **Topology Invariants:**
   - Every `from_junction_id` and `to_junction_id` across all 48,626 segments exists in `junctions.geojson` (**0 dangling endpoints**).
   - Zero duplicate segment IDs.
   - Zero zero-length segments.
2. **Local Ground-Truth Validation:**
   - **Airport Road (`way/53020815`):** Tagged `oneway=yes`. Test confirms **only forward segments** exist in the graph.
   - **Kizhakkambalam Bus Stand road (`way/623556453`):** Tagged `oneway=-1`. Test confirms **only reverse segments** exist in the graph.
3. **Execution:**
   ```bash
   cargo test --test geospatial_tests
   # 1 passed; 0 failed; finished in 1.5s
   ```

## Required validation

The importer must report and test:

- Source and import version.
- Junction and directed-segment counts.
- Missing endpoints, invalid geometry, and duplicate segment IDs.
- Correct one-way behaviour on known local routes.
- Route distance consistency with stored segment lengths.
- Access restrictions for each supported asset class.
- Known origin-to-destination routes inspected against the source map.
- Closure test: every route avoids a verified closed segment.
- Expiry test: a segment becomes eligible only when no active newer closure exists.

## Trust and safety policy

An LLM may extract a candidate operational event from text, but it must not decide that a road is safe.

```text
“Bridge road near J6 is underwater”
        |
        v
candidate closure for J5 -> J6, with confidence
        |
        v
dispatcher verification or corroborating source
        |
        v
active route-blocking operational closure
```

Uncertain reports should be penalized or queued for dispatcher review. Verified closures should cause routing to fail closed.

## First data milestone

Deliver a reproducible import for one pilot area, 10–20 curated shelters/bases, and a small verified closure dataset. Demonstrate:

1. A valid directed route between known locations.
2. A one-way road that cannot be traversed in the prohibited direction.
3. A route changed by a verified flood/debris closure.
4. An unsafe or unreachable route escalated rather than dispatched.
5. A complete Neo4j rebuild from archived source input.

This milestone proves the geographic foundation before expanding coverage, adding a sophisticated UI, or integrating advanced real-time feeds.

