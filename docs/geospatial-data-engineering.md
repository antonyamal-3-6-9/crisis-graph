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

1. Define and version a polygon boundary for the pilot area.
2. Acquire and archive the matching OSM PBF input with date/version.
3. Filter eligible road ways and relevant nodes.
4. Split ways into directed segments at topology-changing nodes.
5. Interpret road direction, access, and vehicle restrictions.
6. Calculate segment length from geometry and derive base travel time where possible.
7. Emit versioned staging artefacts: `junctions.geojson`, `road_segments.geojson`, and `import_report.json`.
8. Validate and load the normalized graph into Neo4j.
9. Load shelters, rescue bases, and asset metadata separately from road data.
10. Rebuild or update the baseline graph only through this importer; apply incidents only as overlay events.

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

