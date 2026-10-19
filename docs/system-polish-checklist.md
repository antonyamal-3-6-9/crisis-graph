# CrisisGraph System Polish and Operator Visibility Checklist

This checklist records what the prototype provides today, what can be inspected
directly, and what should be strengthened next. Checked items are verified in
the current repository or the running local system. Unchecked items are planned
work, not existing capabilities.

Last live inventory: 2026-09-25.

## Demonstration scope decision

CrisisGraph is intentionally presented as an open, local engineering
demonstration rather than a production emergency-operations product. The
console carries a permanent warning that it has no authentication and is not
for operational dispatch. OIDC/JWT, public WhatsApp/SMS/radio gateways, named
responder delivery, and mutable human-review actions are documented production
boundaries, not claims made by this prototype.

The selected completion scope is multilingual candidate extraction,
deterministic validation and routing, atomic resource allocation, persisted
incident/audit state, independent segment verification, fail-closed
escalation, Redis Stream intake, SSE updates, and an inspectable local
command-centre demonstration.

## 1. Neo4j visibility and queryability

### Current live inventory

- [x] 21,256 `Junction` nodes are loaded for `aluva-periyar-pilot`.
- [x] 4 `Shelter` nodes are registered.
- [x] 48,626 directed `CONNECTS_TO` relationships are loaded.
- [x] The junction ID uniqueness constraint exists.
- [x] Junction dataset, OSM node ID, and point indexes are online.
- [x] Segment dataset and segment ID indexes are online.
- [x] Baseline geography and mutable operational fields are stored separately
  on each road relationship.
- [x] No active non-open hazard segment existed at the time of this inventory.
- [x] Neo4j Browser is locally reachable at `http://localhost:7474`.
- [x] Add a read-only graph inventory summary to the CrisisGraph web console.
- [x] Add a read-only `/api/v1/admin/graph/summary` endpoint rather than placing
  database credentials or arbitrary Cypher execution in the public browser UI.
- [ ] Add lookup views for junction ID, OSM way ID, segment ID, road name, and
  nearest junction to a coordinate.
- [ ] Show active overlay provenance, verification state, validity interval,
  and expiry—not only the resulting road status.
- [x] Show shelter allocations and fleet reservations with incident ownership.
- [ ] Add pagination and query limits to every graph-inspection endpoint.
- [ ] Add authorization and audit logs before exposing graph inspection outside
  a local development environment.

### Useful read-only Cypher queries

Run these in Neo4j Browser. They reveal state without mutating it.

```cypher
MATCH (n)
RETURN labels(n) AS labels, count(*) AS count
ORDER BY count DESC;
```

```cypher
MATCH ()-[r]->()
RETURN type(r) AS relationship, count(*) AS count
ORDER BY count DESC;
```

```cypher
MATCH (s:Shelter)
RETURN s.id, s.name, s.junction_id, s.capacity, s.current_occupancy,
       s.ambulances_available, s.trucks_available, s.boats_available
ORDER BY s.id;
```

```cypher
MATCH (a:Junction)-[r:CONNECTS_TO]->(b:Junction)
WHERE r.segment_id = $segment_id
RETURN a.id AS from, b.id AS to, r;
```

```cypher
MATCH (a:Junction)-[r:CONNECTS_TO]->(b:Junction)
WHERE r.operational_status IS NOT NULL
  AND r.operational_status <> 'OPEN'
  AND (r.valid_until IS NULL OR r.valid_until >= datetime())
RETURN r.segment_id, r.road_name, r.operational_status, r.valid_until,
       a.id AS from, b.id AS to
ORDER BY r.valid_until;
```

```cypher
SHOW CONSTRAINTS;
SHOW INDEXES;
```

Representative stored properties currently include:

- `Junction`: `id`, `osm_node_id`, `dataset`, `lat`, `lon`, `location`,
  `inside_boundary`, `degree`, `in_degree`, and `out_degree`.
- `CONNECTS_TO`: `segment_id`, `osm_way_id`, `dataset`, `source_version`,
  `direction`, `oneway`, `road_name`, `road_class`, `length_m`,
  `travel_time_s`, `speed_kph`, `motorcar`, `hgv`, `emergency`, and
  `baseline_status`. Operational overlays add fields such as
  `operational_status` and `valid_until` when applicable.
- `Shelter`: identity, junction, capacity, occupancy, and available ambulance,
  truck, and boat counts.

## 2. HTTP API and contract inventory

The current Axum surface is implemented in `src/bin/server.rs`. There is no
OpenAPI document yet, no authentication layer, and errors generally return an
HTTP status without a stable JSON error body.

### Existing endpoints

| Method | Endpoint | Request | Successful response |
| --- | --- | --- | --- |
| `GET` | `/api/v1/health` | None | Service status, timestamp, pilot name, Redis stream, and consumer group. This does not currently probe every dependency. |
| `GET` | `/api/v1/shelters` | None | Array of shelter records containing ID, name, junction ID, capacity, occupancy, vehicle counts, latitude, and longitude. |
| `POST` | `/api/v1/shelters/reset` | None | `{status, message}` after destructively restoring demonstration capacity and fleet values. |
| `GET` | `/api/v1/hazards` | None | GeoJSON `FeatureCollection` of active hazardous segments. |
| `POST` | `/api/v1/hazards` | `{segment_id, status, duration_hours?}` | `{status, updated_segments, segment_id}`. Duration defaults to four hours. |
| `POST` | `/api/v1/hazards/clear` | None | `{status, cleared_segments}` after clearing all pilot operational hazards. |
| `POST` | `/api/v1/dispatch` | `{raw_text, source_channel?}` | A synchronous `DispatchResponse`; the same response is also broadcast over SSE. |
| `GET` | `/api/v1/events` | None | Server-Sent Events whose data payload is a serialized `DispatchResponse`. |
| `POST` | `/api/v1/sos/stream` | `{raw_text, source_channel?}` | `{status: "queued", stream_message_id, alert_id, stream}`. |
| `POST` | `/api/v1/sos/simulate_spike` | None | Queues five fixed demonstration alerts and returns their IDs and stream IDs. |
| `GET` | `/api/v1/incidents?limit=50` | Optional bounded limit | Most recently updated persisted incident snapshots. |
| `GET` | `/api/v1/incidents/{incident_id}` | Incident ID path parameter | One complete persisted incident snapshot or `404`. |
| `GET` | `/api/v1/incidents/{incident_id}/route` | Incident ID path parameter | Latest immutable verified-route decision plus a GeoJSON projection from baseline junction coordinates, or `404` when no route was certified. Returns `409` rather than drawing a partial route if a persisted junction is missing from the current baseline graph. |
| `GET` | `/api/v1/incidents/{incident_id}/audit` | Incident ID path parameter | Ordered immutable audit events for the incident. |
| `GET` | `/api/v1/resources/reservations?limit=50` | Optional bounded limit | Recent incident-owned resource reservations and release disposition. |
| `GET` | `/api/v1/admin/graph/summary` | None | Bounded pilot counts for junctions, segments, shelters, active hazards, and active reservations. |

### Dispatch response contract

```json
{
  "alert_id": "SOS-...",
  "incident_status": "ROUTE_VERIFIED",
  "incident_version": 7,
  "control_plane_persisted": true,
  "duplicate_suppressed": false,
  "review_required": false,
  "status": "RoutedVerified | EscalateHumanDispatcher",
  "victim_junction": "node/... | null",
  "assigned_shelter": {
    "id": "...",
    "name": "...",
    "junction_id": "node/..."
  },
  "assigned_asset": "AMBULANCE | EVAC_TRUCK | RESCUE_BOAT | null",
  "headcount": 5,
  "triage": {
    "victim_location_raw": "... | null",
    "resolved_junction_id": "node/... | null",
    "headcount": 5,
    "required_asset": "AMBULANCE | EVAC_TRUCK | RESCUE_BOAT | HELICOPTER | null",
    "hazards": [],
    "confidence_score": 0.95,
    "needs_human_review": false,
    "uncertainty_reasons": [],
    "inference": {
      "backend": "llama-cpp | vllm",
      "model": "...",
      "adapter_id": "... | null",
      "adapter_sha256": "... | null",
      "prompt_version": "triage-extraction-v2.1.1",
      "schema_version": "triage-extraction-v2",
      "latency_ms": 0
    }
  },
  "distance_km": 4.1,
  "travel_time_s": 600.0,
  "is_detour": false,
  "detour_reason": null,
  "segment_count": 52,
  "tactical_brief": "...",
  "geojson": {"type": "FeatureCollection", "features": []},
  "errors": []
}
```

`assigned_shelter` and `triage` may also be `null`. Numeric route values become
zero and route GeoJSON may be empty when routing is skipped or fails.

### Contract-strengthening checklist

- [x] Versioned `/api/v1` route prefix.
- [x] Typed Rust request and success-response structures for dispatch.
- [x] Strict structured inference schema and typed post-inference validation.
- [ ] Generate and maintain an OpenAPI 3.1 specification with examples.
- [ ] Introduce one typed error envelope: error code, safe message, incident ID,
  retryability, and optional validation details.
- [ ] Validate non-empty SOS text and impose explicit byte/character limits at
  the API boundary.
- [x] Reject unknown hazard statuses with `400 Bad Request` rather than silently
  mapping invalid input to `FLOODED`.
- [ ] Return `404` or `422` when a requested segment does not exist or no segment
  was updated.
- [ ] Add authentication and roles for dispatcher, observer, supervisor, and
  administrator actions.
- [ ] Protect reset, clear, hazard mutation, and spike simulation as privileged
  demonstration/administration operations.
- [ ] Add idempotency keys to intake/dispatch and explicit request correlation
  IDs to prevent accidental duplicate reservations.
- [ ] Add request timeouts, rate limits, body limits, and dependency-aware
  readiness checks.
- [ ] Add endpoint contract tests for success, malformed JSON, invalid enums,
  missing fields, unavailable inference, and unavailable databases.
- [ ] Correct the UI copy that claims a parallel worker pool. The server
  currently starts one consumer task and processes each fetched batch serially.

## 3. Malayalam and Manglish handling through validation

### What exists now

- [x] HTTP and Rust strings preserve UTF-8 Malayalam input.
- [x] The same model prompt and JSON schema are used for English, Malayalam,
  and Manglish; the contract does not depend on a particular script.
- [x] The model is instructed to preserve stated facts, abstain on missing or
  contradictory scalars, and remove surrounding relational language from a
  victim location.
- [x] Synthetic SFT and evaluation data contain separately labelled Malayalam
  and Manglish records.
- [x] Malayalam/Manglish wording received semantic/manual review during dataset
  development, in addition to automated schema checks.
- [x] Inference output is parsed with `deny_unknown_fields` and deterministically
  checked for schema version, enums, bounds, empty strings, hazard completeness,
  and review consistency.
- [x] Missing dispatch-critical facts or incomplete hazard fields force human
  review regardless of model confidence.
- [x] GPS coordinates embedded in text are snapped to the nearest graph
  junction, independent of language.
- [ ] Runtime validation does **not** determine whether Malayalam grammar is
  natural or semantically correct. JSON validity is not language correctness.
- [ ] Runtime location resolution has no Malayalam landmark gazetteer and no
  Manglish transliteration layer. Its current named landmark aliases are
  predominantly English.
- [ ] The runtime contract does not carry detected language/script or the
  normalization/transliteration method used.
- [ ] There is no Unicode-normalization policy test covering equivalent
  Malayalam representations.

### Actual runtime flow

1. The raw UTF-8 report is sent unchanged to the configured OpenAI-compatible
   `llama.cpp` or vLLM endpoint.
2. Grammar/JSON-schema constrained generation restricts the output shape.
3. Rust parses a closed typed structure and validates values and ranges.
4. Rust independently forces review for missing critical fields and incomplete
   hazards.
5. Location resolution tries GPS first, then a small lexical landmark table.
6. If location resolution fails or the extraction requires review, allocation
   and routing are skipped and the incident escalates.

This means Malayalam extraction can succeed while dispatch still escalates if
the extracted Malayalam place name has no GPS coordinate and no matching alias.

### Strengthening checklist

- [ ] Preserve three values separately: original report text, extracted raw
  place text, and normalized lookup key. Never overwrite the source wording.
- [ ] Define and test Unicode normalization for Malayalam input.
- [ ] Add a versioned Aluva gazetteer containing English, Malayalam, common
  Manglish/transliterated aliases, OSM identity, and graph junction mappings.
- [ ] Resolve aliases deterministically and return match method, confidence,
  candidate list, and ambiguity—not only a junction ID.
- [ ] Escalate ambiguous place matches; never choose among similar names using
  LLM intuition.
- [ ] Add runtime tests for pure Malayalam landmark names, inflected names,
  spelling variants, Manglish code-switching, GPS-plus-text disagreement, and
  out-of-pilot locations.
- [ ] Report extraction and location-resolution metrics separately by English,
  Malayalam, and Manglish.
- [ ] Keep periodic native-speaker/domain review in the evaluation process;
  automated schema validation cannot replace it.

## 4. Existing UI: current operator value

The existing browser UI is a demonstration dispatcher console. It can:

- [x] Display a Leaflet map constrained to the Aluva pilot area.
- [x] Switch among tactical-styled OSM, standard OSM, and Esri satellite tiles
  when internet tile access is available.
- [x] Display the 10 km pilot boundary.
- [x] Show the four relief hubs on the map and list current capacity, occupancy,
  and available ambulances, trucks, and boats.
- [x] Accept free-text SOS messages in English, Malayalam, or Manglish and attach
  a selected source-channel label.
- [x] Run the synchronous extraction-allocation-routing-verification pipeline.
- [x] Show routed-versus-escalated status, distance, estimated time, allocated
  asset, selected hub, and the generated tactical brief.
- [x] Draw the computed route and origin/victim markers when a route exists.
- [x] Copy the tactical brief to the clipboard.
- [x] Display active hazard segments returned by the server.
- [x] Apply one hard-coded flood simulation segment, clear all hazards, and
  reset demonstration shelter/fleet state.
- [x] Listen for live dispatch results over SSE and show toast notifications.
- [x] Queue five fixed surge-test messages through Redis Streams.

It currently cannot:

- [ ] Browse or query the underlying Neo4j graph.
- [ ] Show all road segments or inspect why a particular edge is accessible,
  restricted, closed, or selected.
- [x] Show a persistent incident queue, dispatch history, or audit timeline.
- [x] Show the full extraction and uncertainty fields in a dedicated review
  panel; they are present in the API response but mostly hidden behind the
  tactical brief.
- [x] Reopen a certified historical route after refresh from its persisted
  ordered junction/segment evidence and display its origin, assigned unit,
  metrics, verification time, and map projection.
- [ ] Review, approve, reject, or expire candidate hazard observations.
- [ ] Apply a hazard by clicking a road; the current demo uses one hard-coded
  segment ID.
- [ ] Select a victim by clicking the map.
- [ ] Animate a vehicle or report live vehicle position/status.
- [ ] Authenticate operators or distinguish observer/dispatcher/admin actions.
- [ ] Identify a responder recipient, transmit the route/order, track delivery,
  receive acknowledgement, or show whether a crew is en route.
- [ ] Work as a fully offline map because its visible base tiles and some web
  assets are fetched from external CDNs.

### Recommended UI polish order

- [x] **First:** incident detail/review panel showing original message, extracted
  fields, uncertainty, inference provenance, allocation, route, verification,
  and errors without requiring inspection of raw JSON.
- [x] **Second:** persistent incident list with clear lifecycle/status filters
  and an audit timeline.
- [ ] **Third:** typed `HazardObservation` review queue with `PENDING`,
  `VERIFIED`, `REJECTED`, and `EXPIRED` states. Only an authorized transition to
  `VERIFIED` may update the routing overlay.
- [ ] **Fourth:** read-only graph/segment inspection and route explanation.
- [ ] **Fifth:** authentication, authorization, confirmations, and audit records
  around every mutation.
- [ ] **Later:** click-to-dispatch, moving-vehicle simulation, and richer visual
  demonstrations after the operator workflow is trustworthy.

## 5. Authentication, human review, assignment, and visualization plan

JWT authentication alone does not make a review workflow reliable. The system
also needs authorization, persisted workflow state, atomic transitions, audit
records, assignment ownership, and reliable delivery/acknowledgement.

### Current human-review behavior and missing pieces

- [x] Deterministic guards can set `needs_human_review` and stop allocation and
  routing when SOS facts are incomplete, ambiguous, or contradictory.
- [x] A failed location resolution, allocation, route, or verification produces
  `EscalateHumanDispatcher`.
- [x] Hazards extracted from language are treated as candidates and do not
  automatically affect routing.
- [x] HTTP and Redis intake now persist versioned incidents and append an audit
  event for every successful pipeline transition.
- [x] Review-required incidents are persisted before returning an escalation;
  incomplete triage stops before resource allocation.
- [x] Duplicate incident IDs are atomically suppressed before inference,
  allocation, or SSE rebroadcast, including Redis redelivery after a missed
  acknowledgement.
- [x] Shelter/asset reservations are owned by incident ID, reject duplicate
  reservation attempts, and are idempotently compensated when later routing,
  verification, or control-plane persistence fails.
- [x] A successful independent verifier transition atomically appends immutable
  route evidence (route version, ordered junctions and segments, allocation,
  objective, metrics, and verification identity/time) to the incident snapshot.
- [x] The generic state-transition API cannot mark a route verified without the
  dedicated verified-route evidence action.
- [ ] Persisted incidents do not yet have an assigned reviewer, deadline, or
  authenticated resolution action.
- [ ] There is no authenticated action for correcting or approving extracted
  SOS fields and resuming the pipeline.
- [ ] Candidate hazards are not yet persisted as typed `HazardObservation`
  review items.
- [ ] The current hazard mutation endpoint bypasses a human-review lifecycle and
  directly edits the operational overlay.
- [ ] No review decision currently records reviewer identity, rationale, source
  evidence, before/after values, or timestamp.

### Identity and authorization

- [ ] Use an established OpenID Connect identity provider to issue JWT access
  tokens; do not implement password storage or JWT signing inside CrisisGraph.
- [ ] Validate JWT signature, algorithm, issuer, audience, expiry, not-before,
  token ID, and required claims against a trusted key set.
- [ ] Use short-lived access tokens. Handle browser sessions using secure,
  `HttpOnly`, `SameSite` cookies or a backend-for-frontend design rather than
  storing long-lived bearer tokens in browser `localStorage`.
- [ ] Define explicit roles and permissions:
  - `OBSERVER`: read authorized operational views only.
  - `DISPATCHER`: review SOS incidents, correct candidate facts, approve or
    reject dispatch, assign responders, and cancel assignments.
  - `HAZARD_VERIFIER`: verify/reject hazard observations and set validity.
  - `RESPONDER`: view only assigned incidents/routes, acknowledge, and report
    operational status.
  - `EOC_ADMIN`: manage users/roles, view the whole pilot, and perform protected
    demonstration/administrative actions.
  - `SYSTEM_SERVICE`: narrowly scoped service-to-service processing.
- [ ] Enforce permissions in Axum middleware and handler policy checks; hiding
  UI controls is not authorization.
- [ ] Scope responder access to assigned incidents and the minimum operational
  context needed for safe execution.
- [ ] Add an audited, time-limited break-glass mechanism for emergencies rather
  than silently bypassing policy.
- [ ] Define logout/revocation behavior, key rotation, clock-skew tolerance,
  failed-authentication logging, and rate limits.

### SOS review workflow

Persist a versioned incident instead of representing human review only as a
string returned to one browser:

```text
RECEIVED
  -> TRIAGE_EXTRACTED
  -> REVIEW_REQUIRED ---------------------------> REJECTED / CLOSED
          | dispatcher corrects/approves
          v
     TRIAGE_APPROVED
          v
     ALLOCATING -> ROUTING -> VERIFYING
          |                     |
          +------ failure ------+-> REVIEW_REQUIRED
                                |
                                v
                         ROUTE_VERIFIED
                                v
                      ASSIGNED -> ACKNOWLEDGED
                                v
                     EN_ROUTE -> ARRIVED -> COMPLETED
```

- [ ] Persist original SOS, extraction, deterministic validation failures,
  revisions, current state, version, and assigned reviewer.
- [ ] Add an authenticated review endpoint that accepts explicit field changes,
  decision, reason, and expected incident version.
- [ ] Never edit the original model extraction in place; create a reviewed
  revision with provenance.
- [ ] Re-run deterministic validation after every correction.
- [ ] Resume allocation/routing only from an approved, complete revision.
- [ ] When review resume is implemented, allow a new reservation cycle for an
  incident whose earlier reservation was released while retaining immutable
  reservation-cycle history.
- [ ] Use compare-and-set/version checks so two dispatchers cannot approve stale
  versions or overwrite one another.
- [ ] Define review timeout, reassignment, escalation, cancellation, and
  duplicate-incident handling.
- [ ] Ensure failed downstream routing/verification releases or compensates any
  resource reservation before returning the incident to review.

### Hazard review workflow

```text
Citizen / field / feed / LLM candidate
                v
             PENDING
             /     \
 authorized verify reject
           /         \
      VERIFIED      REJECTED
          |
          | valid_until reached or superseded
          v
       EXPIRED
```

- [ ] Store the raw report, extracted candidate fields, source, reporter class,
  received time, confidence/evidence, proposed road match, and validity window.
- [ ] Let a verifier resolve the exact graph segment(s), status, and validity
  before approval.
- [ ] Only the atomic transition to `VERIFIED` may create/update a route-blocking
  operational overlay.
- [ ] `REJECTED` observations must never affect routing; retain them for audit
  and duplicate/spam analysis.
- [ ] Expiry or supersession must remove the observation's authority without
  mutating baseline OSM geography.
- [ ] Require version checks and idempotency keys for verify/reject/expire
  actions.
- [ ] Define how multiple reports about the same segment combine and which
  authority wins when verified observations conflict.
- [ ] Re-run route safety checks when a newly verified hazard intersects an
  active assignment.

### Reliable event processing

- [ ] Persist workflow state before notifying clients; an SSE message is not the
  system of record.
- [ ] Publish state-change events through a transactional outbox or equivalent
  recoverable mechanism so database state and emitted events cannot diverge.
- [ ] Use stable event IDs, idempotent consumers, retry limits, and a dead-letter
  path for poison messages.
- [ ] Support replay/recovery after dashboard or responder reconnection; the
  current in-memory broadcast channel loses events for disconnected clients.
- [ ] Run multiple named Redis Stream consumers only after reservation and state
  transitions are proven safe under concurrency.
- [ ] Record actor, action, object ID, previous/new state, timestamp, reason,
  correlation ID, and request origin in an append-only audit trail.

### Command-centre/admin visualization

The open local console now has distinct overview, dispatch, incidents, hazards,
and resources routes. It displays live graph counts, relief hubs, active
hazards, route geometry, model extraction evidence, deterministic outcomes,
incident status/version, immutable audit events, and the reservation ledger.
Controls that directly mutate demonstration hazards or reset resources are
visibly labelled as trusted-operator simulation actions.

- [ ] Add an authenticated, role-aware operations map that can show all active
  incidents, verified hazards, relief hubs, vehicles/resources, assignments,
  and route versions within the operator's scope.
- [ ] Distinguish candidate hazards from verified route-blocking hazards using
  unambiguous symbols and colours; do not present model output as ground truth.
- [x] Display the latest certified route for a selected persisted incident with
  origin, destination, assigned unit, route version, verification time, and
  deterministic metrics.
- [ ] Display every active route simultaneously with origin, destination, assigned vehicle/team,
  route status, verification time, expiry, and current acknowledgement state.
- [ ] Add filters for incident state, resource type, responder, review queue,
  stale routes, unacknowledged assignments, and unresolved hazards.
- [ ] Allow an authorized operator to select an incident or vehicle and inspect
  its timeline, current route, all traversed segment IDs, nearby hazards, and
  safety-verification evidence.
- [ ] Show resource availability versus reserved/in-use/out-of-service state;
  an integer fleet count alone is insufficient for assignment tracking.
- [ ] Never expose unrestricted Cypher or credentials in the application UI.

### Responder/driver visualization and route safety

A driver should not receive the entire administrator view. The responder view
should expose the assigned task and enough surrounding hazard context to execute
it safely.

- [ ] Create a stable `DispatchAssignment` containing incident ID, responder/team
  ID, vehicle ID, route ID and version, route geometry, destination, issued time,
  validity time, and verification result.
- [ ] Provide a responder view showing the assigned route, current position when
  GPS/AVL is available, destination, ETA, relevant nearby verified hazards, and
  clear acknowledgement/status controls.
- [ ] A verified route must never intentionally traverse an active blocked or
  flooded segment. Hazards should appear as avoided/nearby context, not as an
  instruction to drive through them.
- [ ] Require the responder to acknowledge the exact route version received.
- [ ] If a new verified hazard intersects the assigned route, atomically mark
  that route version `INVALIDATED`, notify both responder and dispatcher, and
  run deterministic re-routing and verification.
- [ ] Do not silently swap the route underneath the driver. Deliver the new
  version, explain why the prior route is invalid, and require acknowledgement.
- [ ] Define a fail-safe instruction for loss of connectivity or route
  invalidation, such as hold at a safe position and contact control; do not ask
  an LLM to improvise driving instructions.
- [ ] Track `SENT`, `ACKNOWLEDGED`, `EN_ROUTE`, `ARRIVED`, `COMPLETED`, `FAILED`,
  and `CANCELLED` transitions with timestamps and actor identity.
- [ ] Treat live vehicle animation as real telemetry only when sourced from
  GPS/AVL. Clearly label simulated movement in demonstrations.

### Authentication and workflow verification

- [ ] Unit-test claim validation and every role/permission decision.
- [ ] Integration-test that observers/responders cannot mutate hazards,
  resources, users, or unrelated incidents.
- [ ] Test expired, malformed, wrong-audience, wrong-issuer, revoked, and
  incorrectly signed tokens.
- [ ] Test concurrent SOS approval and concurrent hazard-verification conflicts.
- [ ] Test reconnection/replay, duplicate delivery, worker crash, database
  failure, and notification timeout behavior.
- [ ] Test the full safety case: a verified hazard invalidates an active route,
  the old route cannot remain dispatchable, a deterministic replacement is
  verified, and the responder acknowledges its new version.

## 6. Deferred external delivery integrations

Route calculation is not operational delivery. The prototype currently returns
the result to the API caller and connected dashboard browsers only. It does not
choose or notify a real responder.

Leave this deferred for the current milestone, but retain it as an explicit
system boundary:

- [ ] Define responder/team and vehicle identities separately from an asset
  count at a shelter.
- [ ] Add an authorized dispatcher approval step where policy requires it.
- [ ] Define a delivery lifecycle such as `PROPOSED`, `APPROVED`, `SENT`,
  `ACKNOWLEDGED`, `EN_ROUTE`, `ARRIVED`, `FAILED`, and `CANCELLED`.
- [ ] Deliver a signed/versioned dispatch package containing incident ID,
  assignment, route version, validity time, hazards, and safety-verification
  result.
- [ ] Integrate an actual responder channel later (command-centre terminal,
  mobile client, radio operator workflow, or another authorized system).
- [ ] Require acknowledgement and timeout/escalation instead of treating a
  successful HTTP/UI render as notification.

## 7. Immediate engineering sequence

- [ ] Freeze this API inventory as an OpenAPI contract and add contract tests.
- [ ] Fix hazard-status validation and protect destructive demo endpoints.
- [x] Define typed incident status, immutable triage revision, review decision,
  actor role/permission, optimistic version, and audit-event contracts.
- [x] Implement atomic Neo4j incident snapshots and audit events with stale-write
  rejection and pending-review lookup.
- [x] Wire both synchronous HTTP and Redis Stream intake through the same
  fail-closed, stage-aware persisted orchestrator.
- [x] Include persisted incident status, version, review flag, persistence
  health, and duplicate-suppression state in dispatch responses.
- [x] Expose bounded, read-only incident lookup and audit-history endpoints for
  the local demonstration console.
- [ ] Expose authenticated review and resume endpoints if the project scope is
  later expanded beyond the open local demonstration.
- [ ] Define the hazard-observation, responder, vehicle, and dispatch-assignment
  state contracts. The immutable route-version contract is now implemented.
- [ ] Integrate OIDC/JWT validation and server-side role enforcement before
  exposing review or assignment mutations.
- [x] Build a read-only incident detail panel showing the original report,
  current immutable triage revision, uncertainty, status/version, and audit
  timeline.
- [ ] Implement the typed `HazardObservation` lifecycle and authorization gate.
- [ ] Persist state transitions and audit history, then make SSE a replayable
  projection rather than the only notification mechanism.
- [x] Build historical verified-route inspection into the command-centre
  incident view.
- [ ] Build an all-active-routes operational map and scoped responder route view.
- [ ] Add Malayalam/Manglish-aware deterministic location aliases and tests.
- [x] Add a read-only graph summary API/UI.
- [ ] Add segment-level inspection only if needed for a later visualization
  milestone.
- [x] Add incident-owned resource reservations and idempotent compensation when
  a later route, verification, or persistence stage fails.
- [ ] Add external responder delivery integrations only after authenticated
  assignment, route versioning, acknowledgement, and invalidation work locally.

## 8. Closure roadmap for the current local demonstration

This is the completion gate for the agreed portfolio/demo scope: Aluva pilot,
local operator UI, model-assisted candidate triage, deterministic routing,
independent route verification, persisted incidents/routes, and direct hazard
simulation. The unchecked production-hardening items above are not completion
blockers unless the scope is explicitly expanded.

### A. Manual browser review

- [ ] Open `http://localhost:3000/incidents` while the local API and inference
  server are running.
- [ ] Select the already persisted happy-path demo incident
  `SOS-95592EC7`. Confirm `ROUTE_VERIFIED`, the original SOS, extracted triage,
  assigned UC College shelter and ambulance, route map, 4.107 km / 276 s
  metrics, 52 certified segments, route version 1, and audit timeline.
- [ ] Refresh the browser, reopen the incident, and confirm the route is
  reconstructed from persisted evidence rather than depending on the original
  dispatch response.
- [ ] Submit one incomplete/ambiguous SOS. Confirm it enters review, has no
  dispatchable route, and does not retain an active resource reservation.
- [ ] Use the trusted local hazard simulation on a segment from the saved route.
  Submit a new routable SOS and confirm it either avoids that segment or safely
  escalates. Clear the simulated hazard afterward and confirm the active hazard
  display is clear.
- [ ] Confirm the page labels the console as a local demonstration and does
  not imply that a real responder has been notified.

### B. Automated acceptance checks

Already run for this implementation pass:

- [x] `cargo check --bin server`
- [x] `cargo test --bin server --test control_plane_tests` — GeoJSON projection
  and 14 control-plane tests passed.
- [x] `cargo test --lib pipeline::stages::tests` — 5 brief/validation tests
  passed, including the corrected success-status reason.
- [x] `cargo test --test incident_repository_tests` — live Neo4j persistence,
  reload, route evidence, audit, and stale-write test passed.
- [x] Live dispatch through Qwen3-4B V3, Neo4j, and the directed route graph
  returned `ROUTE_VERIFIED`; route evidence and GeoJSON reloaded after API
  restart.
- [ ] Run the complete relevant test suite once during final review with its
  configured Neo4j/Redis services reachable; distinguish infrastructure
  connectivity failures from code failures.
- [ ] Run `git diff --check` and inspect the final staged diff.

### C. Scope decision and closeout

- [x] The typed `HazardObservation` workflow (`PENDING`, `VERIFIED`,
  `REJECTED`, `EXPIRED`) is explicitly deferred for this local demo. Current
  `/api/v1/hazards` is a trusted-operator simulation control; it directly
  changes the operational overlay and has no authentication.
- [x] OIDC/JWT, citizen/public intake, WhatsApp/SMS forwarding, real responder
  notification, acknowledgement, and vehicle telemetry are deferred. The UI
  must continue to describe routes as proposals/results for the local demo,
  not as delivered dispatches.
- [ ] Amal completes the browser review and hazard simulation above.
- [ ] Review the source diff and retain only intended implementation files;
  preserve unrelated existing documentation edits and exclude `.env`, model
  artifacts, raw map data, and generated operational data.
- [ ] After review, decide which files to stage and commit. Do not stage or push
  by default as part of this checklist.

### Completion criterion

Mark this demo scope complete after A and B's remaining checks pass, the scope
decision in C remains accepted, and the final source diff is reviewed. Production
gaps listed elsewhere in this document remain recorded as future work; they do
not keep this agreed local demonstration open.
