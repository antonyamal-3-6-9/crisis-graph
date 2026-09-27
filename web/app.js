"use strict";

const API = "/api/v1";
const ALUVA = [10.10816, 76.35651];
const state = { incidents: [], shelters: [], hazards: null, reservations: [], graph: null, maps: {}, layers: {}, selectedIncident: null, incidentRouteMap: null };

const pageMeta = {
  "/": ["Emergency operations", "Operational overview"],
  "/dispatch": ["Decision-support laboratory", "SOS dispatch demonstration"],
  "/incidents": ["Persisted control plane", "Incident registry"],
  "/hazards": ["Dynamic map validity", "Operational hazard overlay"],
  "/resources": ["Concurrent allocation", "Relief hubs and reservations"],
};

const scenarios = {
  verified: "Aluva Railway Station has 3 injured passengers. Send an ambulance immediately.",
  malayalam: "ആലുവ മണപ്പുറം ക്ഷേത്രപ്പടികളിൽ 12 പേർ വെള്ളത്തിൽ കുടുങ്ങിയിരിക്കുന്നു. രക്ഷാബോട്ട് വേണം.",
  ambiguous: "Bank Junction aduthu help venam. Aalkkar undu, pakshe exact countum vehicleum confirm alla.",
  truck: "Send an evacuation truck to Aluva Railway Station for 10 stranded passengers.",
};

document.addEventListener("DOMContentLoaded", async () => {
  configurePage();
  bindEvents();
  startClock();
  initializeMaps();
  initEventStream();
  await Promise.allSettled([loadHealth(), loadGraphSummary(), loadShelters(), loadHazards(), loadIncidents(), loadReservations()]);
  invalidateVisibleMaps();
});

function configurePage() {
  const path = pageMeta[location.pathname] ? location.pathname : "/";
  document.querySelectorAll(".view").forEach((el) => el.classList.toggle("active", el.dataset.view === path));
  document.querySelectorAll(".nav a").forEach((el) => el.classList.toggle("active", el.dataset.route === path));
  document.getElementById("page-eyebrow").textContent = pageMeta[path][0];
  document.getElementById("page-title").textContent = pageMeta[path][1];
}

function bindEvents() {
  document.getElementById("dispatch-form")?.addEventListener("submit", submitDispatch);
  document.querySelectorAll("[data-scenario]").forEach((button) => button.addEventListener("click", () => {
    document.getElementById("sos-text").value = scenarios[button.dataset.scenario];
    document.getElementById("dispatch-form").requestSubmit();
  }));
  document.getElementById("incident-search")?.addEventListener("input", renderIncidentList);
  document.getElementById("incident-filter")?.addEventListener("change", renderIncidentList);
  document.getElementById("refresh-incidents")?.addEventListener("click", loadIncidents);
  document.getElementById("simulate-flood")?.addEventListener("click", simulateFlood);
  document.getElementById("clear-hazards")?.addEventListener("click", clearHazards);
  document.getElementById("reset-resources")?.addEventListener("click", resetResources);
}

function startClock() {
  const update = () => document.getElementById("clock").textContent = `${new Intl.DateTimeFormat("en-IN", { timeZone: "Asia/Kolkata", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }).format(new Date())} IST`;
  update();
  setInterval(update, 1000);
}

async function fetchJson(url, options) {
  const response = await fetch(url, options);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return response.json();
}

async function fetchOptionalJson(url) {
  const response = await fetch(url);
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return response.json();
}

async function loadHealth() {
  try {
    await fetchJson(`${API}/health`);
    setConnection("api", true, "Engine API online");
  } catch (error) {
    setConnection("api", false, "Engine API unavailable");
  }
}

function setConnection(kind, online, label) {
  const dot = document.getElementById(`${kind}-dot`);
  const text = document.getElementById(`${kind}-label`);
  if (dot) dot.className = online ? "online" : "warn";
  if (text) text.textContent = label;
}

async function loadGraphSummary() {
  try {
    state.graph = await fetchJson(`${API}/admin/graph/summary`);
    setText("metric-junctions", number(state.graph.junctions));
    setText("metric-segments", number(state.graph.directed_segments));
    setText("metric-hazards", number(state.graph.active_hazards));
    setText("metric-reservations", number(state.graph.active_reservations));
  } catch (error) {
    console.error("Graph summary failed", error);
  }
}

async function loadShelters() {
  try {
    state.shelters = await fetchJson(`${API}/shelters`);
    renderSheltersOnMaps();
    renderResourceCards();
  } catch (error) {
    renderFailure("resource-cards", "Relief hubs could not be loaded.");
  }
}

async function loadHazards() {
  try {
    state.hazards = await fetchJson(`${API}/hazards`);
    const count = state.hazards.features?.length || 0;
    setText("hazard-count", `${count} active`);
    setText("metric-hazards", number(count));
    renderHazardsOnMaps();
    renderHazardList();
  } catch (error) {
    renderFailure("hazard-list", "Operational overlay could not be loaded.");
  }
}

async function loadIncidents() {
  try {
    state.incidents = await fetchJson(`${API}/incidents?limit=100`);
    const reviewCount = state.incidents.filter((item) => item.status === "REVIEW_REQUIRED").length;
    setText("metric-review", number(reviewCount));
    renderOverviewIncidents();
    renderIncidentList();
  } catch (error) {
    renderFailure("overview-incidents", "Incident registry could not be loaded.");
    renderFailure("incident-list", "Incident registry could not be loaded.");
  }
}

async function loadReservations() {
  try {
    state.reservations = await fetchJson(`${API}/resources/reservations?limit=100`);
    renderReservations();
  } catch (error) {
    const table = document.getElementById("reservation-table");
    if (table) table.innerHTML = '<tr><td colspan="7" class="muted-cell">Reservation ledger could not be loaded.</td></tr>';
  }
}

function initializeMaps() {
  if (!window.L) return;
  ["overview-map", "dispatch-map", "hazard-map"].forEach((id) => {
    if (!document.getElementById(id)) return;
    const map = L.map(id, { minZoom: 12, maxZoom: 18, maxBounds: [[10.0, 76.24], [10.22, 76.47]], maxBoundsViscosity: 1 }).setView(ALUVA, 13);
    L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", { maxZoom: 19, attribution: "© OpenStreetMap contributors" }).addTo(map);
    L.circle(ALUVA, { radius: 10000, color: "#245c7c", weight: 1, opacity: .45, fillOpacity: .015, dashArray: "6 7" }).addTo(map);
    state.maps[id] = map;
    state.layers[id] = {
      shelters: L.layerGroup().addTo(map),
      hazards: L.layerGroup().addTo(map),
      route: L.layerGroup().addTo(map),
    };
  });
}

function invalidateVisibleMaps() {
  Object.values(state.maps).forEach((map) => setTimeout(() => map.invalidateSize(), 30));
}

function renderSheltersOnMaps() {
  Object.values(state.layers).forEach(({ shelters }) => {
    shelters.clearLayers();
    state.shelters.forEach((hub) => {
      const icon = L.divIcon({ className: "hub-marker", html: hub.id.includes("HOSPITAL") ? "✚" : hub.id.includes("MANAPPURAM") ? "≋" : "H", iconSize: [30, 30], iconAnchor: [15, 15] });
      L.marker([hub.lat, hub.lon], { icon }).bindPopup(`<b>${escapeHtml(hub.name)}</b><br><small>${escapeHtml(hub.id)}</small><br>Occupancy ${hub.current_occupancy}/${hub.capacity}<br>Amb ${hub.ambulances_available} · Truck ${hub.trucks_available} · Boat ${hub.boats_available}`).addTo(shelters);
    });
  });
}

function renderHazardsOnMaps() {
  Object.values(state.layers).forEach(({ hazards }) => {
    hazards.clearLayers();
    if (!state.hazards?.features?.length) return;
    L.geoJSON(state.hazards, {
      style: { color: "#b13e36", weight: 7, opacity: .9, dashArray: "4 6" },
      onEachFeature: (feature, layer) => layer.bindPopup(`<b>${escapeHtml(feature.properties.status)}</b><br>${escapeHtml(feature.properties.road_name)}<br><small>${escapeHtml(feature.properties.segment_id)}</small>`),
    }).addTo(hazards);
  });
}

function renderHazardList() {
  const target = document.getElementById("hazard-list");
  if (!target) return;
  const features = state.hazards?.features || [];
  target.innerHTML = features.length ? features.map((feature) => `<div class="compact-row"><b>${escapeHtml(feature.properties.status)} · ${escapeHtml(feature.properties.road_name)}</b><small>${escapeHtml(feature.properties.segment_id)}</small></div>`).join("") : '<div class="empty-state"><h3>No active restrictions</h3><p>The operational overlay is clear.</p></div>';
}

function renderOverviewIncidents() {
  const target = document.getElementById("overview-incidents");
  if (!target) return;
  const incidents = state.incidents.slice(0, 7);
  target.innerHTML = incidents.length ? incidents.map((incident) => `<a class="feed-row" href="/incidents"><b>${escapeHtml(incident.incident_id)}</b>${statusBadge(incident.status)}<p>${escapeHtml(incident.original_sos.raw_text)}</p></a>`).join("") : '<div class="empty-state"><h3>No persisted incidents</h3><p>Use Dispatch Lab to begin the demonstration.</p></div>';
}

function renderIncidentList() {
  const target = document.getElementById("incident-list");
  if (!target) return;
  const query = document.getElementById("incident-search")?.value.trim().toLowerCase() || "";
  const filter = document.getElementById("incident-filter")?.value || "ALL";
  const rows = state.incidents.filter((incident) => {
    const triage = currentTriage(incident);
    const haystack = `${incident.incident_id} ${incident.original_sos.raw_text} ${triage?.victim_location_raw || ""}`.toLowerCase();
    return (filter === "ALL" || incident.status === filter) && (!query || haystack.includes(query));
  });
  target.innerHTML = rows.length ? rows.map((incident) => `<button class="incident-row ${state.selectedIncident === incident.incident_id ? "selected" : ""}" data-incident-id="${escapeHtml(incident.incident_id)}"><code>${escapeHtml(incident.incident_id)}</code>${statusBadge(incident.status)}<p>${escapeHtml(incident.original_sos.raw_text)}</p><small>v${incident.version} · ${relativeTime(incident.updated_at)}</small></button>`).join("") : '<div class="empty-state"><h3>No matching incidents</h3><p>Adjust the search or state filter.</p></div>';
  target.querySelectorAll("[data-incident-id]").forEach((row) => row.addEventListener("click", () => selectIncident(row.dataset.incidentId)));
}

async function selectIncident(id) {
  state.selectedIncident = id;
  renderIncidentList();
  const target = document.getElementById("incident-detail");
  if (state.incidentRouteMap) {
    state.incidentRouteMap.remove();
    state.incidentRouteMap = null;
  }
  target.innerHTML = '<div class="skeleton">Loading incident evidence and audit history…</div>';
  try {
    const encodedId = encodeURIComponent(id);
    const [incident, audit, persistedRoute] = await Promise.all([
      fetchJson(`${API}/incidents/${encodedId}`),
      fetchJson(`${API}/incidents/${encodedId}/audit`),
      fetchOptionalJson(`${API}/incidents/${encodedId}/route`).catch((error) => ({ projection_error: error.message })),
    ]);
    const triage = currentTriage(incident);
    const reasons = triage?.uncertainty_reasons || [];
    const route = persistedRoute?.route;
    const routeIsActive = ["ROUTE_VERIFIED", "ASSIGNED", "ACKNOWLEDGED", "EN_ROUTE", "ARRIVED"].includes(incident.status);
    const routeIsInvalidated = incident.status === "ROUTE_INVALIDATED";
    target.innerHTML = `<div class="detail-title"><div><code>${escapeHtml(incident.incident_id)}</code><p>Version ${incident.version} · ${escapeHtml(incident.original_sos.source_channel || "unknown source")} · ${formatTime(incident.created_at)}</p></div>${statusBadge(incident.status)}</div>
      <div class="raw-message">${escapeHtml(incident.original_sos.raw_text)}</div>
      <div class="facts">
        ${fact("Victim location", triage?.victim_location_raw)}${fact("Resolved junction", triage?.resolved_junction_id)}${fact("Headcount", triage?.headcount)}
        ${fact("Requested asset", triage?.required_asset)}${fact("Confidence label", triage?.confidence_score)}${fact("Human review", triage?.needs_human_review == null ? null : triage.needs_human_review ? "Required" : "Not required")}
      </div>
      ${reasons.length ? `<div class="notice warning detail-notice"><b>Uncertainty</b><span>${reasons.map(escapeHtml).join(" · ")}</span></div>` : ""}
      ${route ? `<section class="route-record"><div class="route-record-head"><div><p class="kicker">Immutable route evidence</p><h3>${routeIsInvalidated ? "Invalidated" : routeIsActive ? "Verified" : "Historical"} route v${route.route_version}</h3></div><span class="tag ${routeIsInvalidated ? "danger" : routeIsActive ? "success" : "muted"}">${routeIsInvalidated ? "Do not use" : routeIsActive ? `${route.verified_segment_count} segments certified` : "Audit evidence"}</span></div>
        <div class="incident-route-map" id="incident-route-map"></div>
        <div class="facts route-facts">${fact("Origin", route.origin_shelter_name)}${fact("Assigned unit", route.assigned_asset_id)}${fact("Triage revision", `v${route.triage_revision}`)}${fact("Distance", `${Number(route.total_distance_km).toFixed(2)} km`)}${fact("Travel time", `${(Number(route.total_travel_time_s) / 60).toFixed(1)} min`)}${fact("Objective", route.cost_objective)}${fact("Verified at", formatTime(route.verified_at))}</div>
        <p class="route-proof"><code>${escapeHtml(route.route_id)}</code> · ${escapeHtml(route.dataset)} · ${escapeHtml(route.detour_reason || "Direct clear path")} · current incident state ${escapeHtml(incident.status)}</p>
      </section>` : persistedRoute?.projection_error ? `<div class="notice warning detail-notice"><b>Route projection unavailable</b><span>The incident record remains readable, but its stored junction path could not be projected onto the current baseline map.</span></div>` : `<div class="notice detail-notice"><b>No persisted verified route</b><span>This incident stopped before route certification, or predates immutable route evidence storage.</span></div>`}
      <div class="timeline"><h3>Immutable audit timeline</h3>${audit.length ? audit.map((event) => `<div class="timeline-row"><b>${escapeHtml(event.action)}</b><p>${escapeHtml(event.from_status)} → ${escapeHtml(event.to_status)} · ${escapeHtml(event.reason)}</p><small>v${event.previous_version} → v${event.new_version} · ${formatTime(event.occurred_at)} · ${escapeHtml(event.actor_id)}</small></div>`).join("") : '<p class="supporting">No audit events found.</p>'}</div>`;
    if (persistedRoute?.geojson) renderIncidentRoute(persistedRoute.geojson, routeIsActive, routeIsInvalidated);
  } catch (error) {
    target.innerHTML = '<div class="empty-state"><h3>Incident unavailable</h3><p>The persisted record could not be loaded.</p></div>';
  }
}

function renderIncidentRoute(geojson, isActive, isInvalidated) {
  const container = document.getElementById("incident-route-map");
  if (!container || !window.L || !geojson?.features?.length) return;
  const map = L.map(container, { minZoom: 12, maxZoom: 18, maxBounds: [[10.0, 76.24], [10.22, 76.47]], maxBoundsViscosity: 1 }).setView(ALUVA, 13);
  L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", { maxZoom: 19, attribution: "© OpenStreetMap contributors" }).addTo(map);
  const layer = L.geoJSON(geojson, {
    style: { color: isInvalidated ? "#b13e36" : isActive ? "#1f8a62" : "#647873", weight: 6, opacity: .9, dashArray: isActive ? null : "7 6" },
    pointToLayer: (feature, latlng) => L.circleMarker(latlng, { radius: 8, color: "#fff", weight: 2, fillColor: feature.properties.type === "victim" ? "#b13e36" : "#245c7c", fillOpacity: 1 }),
    onEachFeature: (feature, featureLayer) => featureLayer.bindPopup(feature.properties.type === "victim" ? `Victim junction<br><small>${escapeHtml(feature.properties.junction_id)}</small>` : feature.properties.type === "shelter" ? `<b>${escapeHtml(feature.properties.name)}</b><br>${escapeHtml(feature.properties.asset)}` : `Verified route v${escapeHtml(feature.properties.route_version)}`),
  }).addTo(map);
  const bounds = layer.getBounds();
  if (bounds.isValid()) map.fitBounds(bounds, { padding: [35, 35], maxZoom: 16 });
  state.incidentRouteMap = map;
  setTimeout(() => map.invalidateSize(), 30);
}

async function submitDispatch(event) {
  event.preventDefault();
  const input = document.getElementById("sos-text");
  const button = document.getElementById("dispatch-btn");
  const rawText = input.value.trim();
  if (!rawText) return;
  button.disabled = true;
  button.querySelector("span").textContent = "Running persisted safety pipeline…";
  document.getElementById("dispatch-map-state").textContent = "Processing";
  try {
    const result = await fetchJson(`${API}/dispatch`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ raw_text: rawText, source_channel: document.getElementById("channel-select").value }) });
    renderDispatchResult(result);
    await Promise.allSettled([loadIncidents(), loadShelters(), loadReservations(), loadGraphSummary()]);
  } catch (error) {
    showToast("Dispatch failed", error.message, true);
    document.getElementById("dispatch-result").innerHTML = `<div class="empty-state"><h3>Pipeline unavailable</h3><p>${escapeHtml(error.message)}</p></div>`;
  } finally {
    button.disabled = false;
    button.querySelector("span").textContent = "Run triage, allocation and routing";
  }
}

function renderDispatchResult(data, fromStream = false) {
  const verified = data.status === "RoutedVerified" && data.incident_status === "ROUTE_VERIFIED";
  const triage = data.triage || {};
  const target = document.getElementById("dispatch-result");
  if (target) target.innerHTML = `<div class="result-head"><div><p class="kicker">${verified ? "Certified deterministic result" : "Fail-closed outcome"}</p><h2>${verified ? "Route verified for dispatch proposal" : "Human escalation required"}</h2><code>${escapeHtml(data.alert_id)} · incident v${data.incident_version}</code></div>${statusBadge(data.incident_status)}</div>
    <div class="result-metrics"><div><span>Distance</span><b>${data.distance_km ? data.distance_km.toFixed(2) + " km" : "Not computed"}</b></div><div><span>Travel time</span><b>${data.travel_time_s ? (data.travel_time_s / 60).toFixed(1) + " min" : "Not computed"}</b></div><div><span>Asset</span><b>${escapeHtml(data.assigned_asset || "Unassigned")}</b></div><div><span>Segments verified</span><b>${number(data.segment_count || 0)}</b></div></div>
    <div class="evidence-grid"><div class="evidence"><h3>AI candidate extraction</h3><dl><dt>Location</dt><dd>${value(triage.victim_location_raw)}</dd><dt>Headcount</dt><dd>${value(triage.headcount)}</dd><dt>Requested asset</dt><dd>${value(triage.required_asset)}</dd><dt>Confidence</dt><dd>${value(triage.confidence_score)}</dd><dt>Review flag</dt><dd>${data.review_required ? "Required" : "Clear"}</dd></dl></div><div class="evidence"><h3>Deterministic controls</h3><dl><dt>Persistence</dt><dd>${data.control_plane_persisted ? "Neo4j committed" : "Failed"}</dd><dt>Duplicate</dt><dd>${data.duplicate_suppressed ? "Suppressed" : "No"}</dd><dt>Junction</dt><dd>${value(data.victim_junction)}</dd><dt>Origin</dt><dd>${value(data.assigned_shelter?.name)}</dd><dt>Route</dt><dd>${verified ? "100% segments passed" : "Unavailable / rejected"}</dd></dl></div></div>
    ${(triage.uncertainty_reasons || []).length ? `<ul class="error-list">${triage.uncertainty_reasons.map((reason) => `<li>${escapeHtml(reason)}</li>`).join("")}</ul>` : ""}${data.errors?.length ? `<ul class="error-list">${data.errors.map((reason) => `<li>${escapeHtml(reason)}</li>`).join("")}</ul>` : ""}`;
  setText("dispatch-map-state", verified ? "Route verified" : "No dispatchable route");
  renderRoute(data.geojson, verified);
  if (fromStream) showToast(data.alert_id, verified ? "Stream incident routed and verified." : "Stream incident requires human review.", !verified);
}

function renderRoute(geojson, verified) {
  ["dispatch-map", "overview-map"].forEach((id) => {
    const bundle = state.layers[id];
    if (!bundle) return;
    bundle.route.clearLayers();
    if (!geojson?.features?.length) return;
    const layer = L.geoJSON(geojson, {
      style: { color: verified ? "#1f8a62" : "#b57418", weight: 6, opacity: .9 },
      pointToLayer: (feature, latlng) => L.circleMarker(latlng, { radius: 8, color: "#fff", weight: 2, fillColor: feature.properties.type === "victim" ? "#b13e36" : "#245c7c", fillOpacity: 1 }),
    }).addTo(bundle.route);
    const bounds = layer.getBounds();
    if (bounds.isValid()) state.maps[id].fitBounds(bounds, { padding: [45, 45], maxZoom: 16 });
  });
}

function renderResourceCards() {
  const target = document.getElementById("resource-cards");
  if (!target) return;
  target.innerHTML = state.shelters.map((hub) => {
    const ratio = hub.capacity ? Math.min(100, Math.round(hub.current_occupancy / hub.capacity * 100)) : 0;
    return `<article class="resource-card"><h3>${escapeHtml(hub.name)}</h3><code>${escapeHtml(hub.id)}</code><div class="capacity"><div class="capacity-head"><span>Occupancy</span><b>${hub.current_occupancy} / ${hub.capacity}</b></div><div class="bar"><i style="width:${ratio}%"></i></div></div><div class="fleet"><span>🚑 ${hub.ambulances_available}</span><span>🚚 ${hub.trucks_available}</span><span>🚤 ${hub.boats_available}</span></div></article>`;
  }).join("");
}

function renderReservations() {
  const target = document.getElementById("reservation-table");
  if (!target) return;
  target.innerHTML = state.reservations.length ? state.reservations.map((item) => `<tr><td><code>${escapeHtml(item.incident_id)}</code></td><td>${escapeHtml(item.shelter_name)}</td><td>${escapeHtml(item.asset_type)}</td><td>${item.headcount}</td><td>${statusBadge(item.status)}</td><td>${formatTime(item.reserved_at)}</td><td>${escapeHtml(item.release_reason || (item.status === "RESERVED" ? "Active allocation" : "Released"))}</td></tr>`).join("") : '<tr><td colspan="7" class="muted-cell">No reservation records yet.</td></tr>';
}

async function simulateFlood() {
  await mutateDemo(`${API}/hazards`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ segment_id: "way/1080426401/seg/0/rev", status: "FLOODED", duration_hours: 4 }) }, "Flood overlay applied", "The known segment is excluded from subsequent routes.");
  await Promise.allSettled([loadHazards(), loadGraphSummary()]);
}

async function clearHazards() {
  await mutateDemo(`${API}/hazards/clear`, { method: "POST" }, "Overlay cleared", "Operational hazards were removed; baseline geography was untouched.");
  await Promise.allSettled([loadHazards(), loadGraphSummary()]);
}

async function resetResources() {
  await mutateDemo(`${API}/shelters/reset`, { method: "POST" }, "Demo resources reset", "Shelter capacity and fleet values were restored.");
  await Promise.allSettled([loadShelters(), loadReservations(), loadGraphSummary()]);
}

async function mutateDemo(url, options, title, message) {
  try { await fetchJson(url, options); showToast(title, message); } catch (error) { showToast("Demonstration action failed", error.message, true); }
}

function initEventStream() {
  const source = new EventSource(`${API}/events`);
  source.onopen = () => setConnection("stream", true, "Redis/SSE live feed");
  source.onerror = () => setConnection("stream", false, "Live feed reconnecting");
  source.onmessage = async (event) => {
    try {
      const data = JSON.parse(event.data);
      renderDispatchResult(data, true);
      await Promise.allSettled([loadIncidents(), loadShelters(), loadReservations()]);
    } catch (error) { console.error("Invalid SSE dispatch event", error); }
  };
}

function currentTriage(incident) { return incident.triage_revisions?.at(-1)?.triage || null; }
function statusBadge(status) { const kind = status === "REVIEW_REQUIRED" || status === "RELEASED" ? "review" : status === "ROUTE_VERIFIED" || status === "RESERVED" || status === "COMPLETED" ? "verified" : "progress"; return `<span class="status ${kind}">${escapeHtml(status || "UNKNOWN")}</span>`; }
function fact(label, content) { return `<div class="fact"><span>${escapeHtml(label)}</span><b>${value(content)}</b></div>`; }
function value(content) { return content === null || content === undefined || content === "" ? "Not supplied" : escapeHtml(String(content)); }
function number(content) { return new Intl.NumberFormat("en-IN").format(content || 0); }
function setText(id, content) { const node = document.getElementById(id); if (node) node.textContent = content; }
function formatTime(input) { if (!input) return "—"; const date = new Date(input); return Number.isNaN(date.valueOf()) ? escapeHtml(String(input)) : new Intl.DateTimeFormat("en-IN", { dateStyle: "medium", timeStyle: "short", timeZone: "Asia/Kolkata" }).format(date); }
function relativeTime(input) { const seconds = Math.round((new Date(input).valueOf() - Date.now()) / 1000); const absolute = Math.abs(seconds); const [amount, unit] = absolute < 60 ? [seconds, "second"] : absolute < 3600 ? [Math.round(seconds / 60), "minute"] : absolute < 86400 ? [Math.round(seconds / 3600), "hour"] : [Math.round(seconds / 86400), "day"]; return new Intl.RelativeTimeFormat("en", { numeric: "auto" }).format(amount, unit); }
function escapeHtml(input) { return String(input ?? "").replace(/[&<>'"]/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" }[char])); }
function renderFailure(id, message) { const target = document.getElementById(id); if (target) target.innerHTML = `<div class="empty-state"><h3>Data unavailable</h3><p>${escapeHtml(message)}</p></div>`; }
function showToast(title, message, error = false) { const region = document.getElementById("toast-region"); const toast = document.createElement("div"); toast.className = `toast${error ? " error" : ""}`; toast.innerHTML = `<b>${escapeHtml(title)}</b><span>${escapeHtml(message)}</span>`; region.appendChild(toast); setTimeout(() => toast.remove(), 6500); }
