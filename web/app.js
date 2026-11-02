"use strict";
// CrisisGraph operations console on the new pipeline (M7).
//
// Reads come from the api role (/api/v1/...). Live updates come from the
// server-sent event feed (/api/v1/events); each event is deduplicated by
// event_id and only triggers a refetch, and a refetched record older than
// the one on screen is ignored, so duplicate or stale events never move the
// view backwards. Commands (review, complete, hazards) need an operator
// token, kept in sessionStorage for this tab only.

const API = "/api/v1";
const ALUVA = [10.10816, 76.35651];
const BOUNDS = [[10.0, 76.24], [10.22, 76.47]];
const TOKEN_KEY = "crisisgraph.operatorToken";
const TRACK_KEY = "crisisgraph.trackedReports";
const DEVICE_KEY = "crisisgraph.deviceId";

const pageMeta = {
  "/": ["Emergency operations", "Operational overview"],
  "/intake": ["New pipeline", "SOS intake"],
  "/incidents": ["Dispatcher workspace", "Incidents"],
  "/hazards": ["Operational overlay", "Hazards"],
  "/fleet": ["Resources", "Fleet & beds"],
  "/alerts": ["Needs attention", "Alerts"],
};

// Event type -> parts of the console to refetch.
const EVENT_EFFECTS = {
  INCIDENT_READY: ["incidents", "queue", "selected", "tracked", "alerts"],
  INCIDENT_REVIEW_REQUIRED: ["incidents", "selected", "tracked", "alerts"],
  INCIDENT_UPDATED: ["incidents", "queue", "selected", "tracked"],
  INCIDENT_CRITICAL: ["incidents", "alerts", "selected"],
  INCIDENT_REJECTED: ["incidents", "selected", "tracked", "alerts"],
  POSSIBLE_DUPLICATE_FLAGGED: ["incidents", "selected"],
  VEHICLE_HELD: ["vehicles"],
  TASK_ASSIGNED: ["incidents", "vehicles", "queue", "shelters", "selected", "tracked", "routes"],
  TASK_COMPLETED: ["incidents", "vehicles", "selected", "tracked", "routes"],
  TASK_QUEUED: ["queue", "incidents", "selected"],
  TASK_ESCALATED: ["alerts", "queue", "selected"],
  VEHICLE_AVAILABLE: ["vehicles"],
  RESERVATION_RELEASED: ["vehicles", "queue", "shelters"],
  RESERVATION_EXPIRED: ["vehicles", "queue"],
  BEDS_RESERVED: ["shelters"],
  DROPOFF_UNRESOLVED: ["alerts", "selected"],
  HAZARDS_CHANGED: ["hazards", "system"],
  INCIDENT_MERGED: ["incidents", "selected", "queue", "tracked"],
  INCIDENT_SPLIT: ["incidents", "selected"],
  TASK_CANCELLED: ["incidents", "queue", "selected", "vehicles", "routes"],
  VEHICLE_STATUS_CHANGED: ["vehicles", "queue", "selected"],
  MISSION_CONVERTED: ["incidents", "selected", "queue", "alerts", "routes"],
  MISSION_CANCELLED: ["incidents", "selected", "queue", "vehicles", "routes"],
};

const scenarios = {
  critical: {
    title: "CRITICAL: unconscious elder", hint: "Rust rules decide CRITICAL", tone: "danger",
    text: "My grandfather is unconscious at Bank Junction Aluva. Water is at chest level and rising. There are 3 of us. Please send an ambulance.",
    lat: 10.1076, lon: 76.3525, acc: 15,
  },
  boat: {
    title: "Malayalam boat rescue", hint: "Multilingual extraction", tone: "",
    text: "ആലുവ മണപ്പുറം ക്ഷേത്രത്തിനടുത്ത് 6 പേർ വെള്ളത്തിൽ കുടുങ്ങിയിരിക്കുന്നു. വെള്ളം അരയ്ക്ക് മുകളിലാണ്. രക്ഷാബോട്ട് വേണം.",
    lat: null, lon: null, acc: null,
  },
  remote: {
    title: "Remote caller (finding F-1)", hint: "Routed to the parents, not the phone", tone: "warning",
    text: "I am calling from Kochi. My parents, 2 people, are trapped near Manappuram Shiva Temple in Aluva. Water is at waist level. Please send a boat.",
    lat: 9.9816, lon: 76.2999, acc: 30,
  },
  unclear: {
    title: "Unclear Manglish", hint: "Must stop for review", tone: "warning",
    text: "Bank Junction aduthu help venam. Aalkkar undu, pakshe exact countum vehicleum confirm alla.",
    lat: 10.1076, lon: 76.3525, acc: 25,
  },
};

const state = {
  incidents: [], vehicles: [], shelters: [], hazards: null, overlayVersion: null, queue: null, alerts: null,
  system: null, selected: null, detail: null, routes: new Map(), tracked: [], seen: new Set(),
  maps: {}, layers: {}, pending: new Set(), refreshTimer: null, fallbackPoll: null, picked: null, segments: [],
};

document.addEventListener("DOMContentLoaded", async () => {
  state.tracked = loadSession(TRACK_KEY, []);
  configurePage();
  bindEvents();
  renderScenarios();
  renderToken();
  startClock();
  initializeMaps();
  connectFeed();
  await refresh(["system", "incidents", "vehicles", "shelters", "hazards", "queue", "alerts", "tracked", "routes"]);
  setInterval(() => queueRefresh("system", "incidents", "queue", "vehicles", "tracked"), 15000);
  setInterval(() => queueRefresh("tracked"), 3000);
});

// ------------------------------------------------------------ navigation

function navigateTo(path) {
  if (location.pathname !== path) history.pushState(null, "", path);
  configurePage();
  invalidateMaps();
}

function configurePage() {
  const path = pageMeta[location.pathname] ? location.pathname : "/";
  document.querySelectorAll(".view").forEach((el) => el.classList.toggle("active", el.dataset.view === path));
  document.querySelectorAll(".nav a").forEach((el) => el.classList.toggle("active", el.dataset.route === path));
  setText("page-eyebrow", pageMeta[path][0]);
  setText("page-title", pageMeta[path][1]);
}

function bindEvents() {
  window.addEventListener("popstate", () => { configurePage(); invalidateMaps(); });
  document.addEventListener("click", (event) => {
    const link = event.target.closest("a[href^='/']");
    if (link && pageMeta[link.getAttribute("href")]) {
      event.preventDefault();
      navigateTo(link.getAttribute("href"));
      return;
    }
    const open = event.target.closest("[data-open-incident]");
    if (open) {
      event.preventDefault();
      navigateTo("/incidents");
      selectIncident(open.dataset.openIncident);
    }
  });
  document.getElementById("intake-form").addEventListener("submit", submitReport);
  document.getElementById("token-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const value = document.getElementById("token-input").value.trim();
    if (value) sessionStorage.setItem(TOKEN_KEY, value); else sessionStorage.removeItem(TOKEN_KEY);
    document.getElementById("token-input").value = "";
    renderToken();
    if (state.detail) renderDetail(state.detail);
    renderSegmentChoices();
    renderHazardList();
  });
  document.getElementById("incident-search").addEventListener("input", renderIncidentList);
  document.getElementById("incident-filter").addEventListener("change", renderIncidentList);
  document.getElementById("refresh-incidents").addEventListener("click", () => refresh(["incidents", "selected"]));
  document.getElementById("vehicle-filter").addEventListener("change", renderVehicleTable);
  const device = document.getElementById("device-id");
  device.value = loadSession(DEVICE_KEY, null) || `console-${uuid().slice(0, 8)}`;
  device.addEventListener("change", () => saveSession(DEVICE_KEY, device.value.trim()));
}

function startClock() {
  const tick = () => setText("clock", new Intl.DateTimeFormat("en-IN", { timeStyle: "medium", timeZone: "Asia/Kolkata" }).format(new Date()) + " IST");
  tick();
  setInterval(tick, 1000);
}

// ------------------------------------------------------------ data access

function token() { return sessionStorage.getItem(TOKEN_KEY); }

async function api(path, { method = "GET", body, auth = false } = {}) {
  const headers = {};
  if (body !== undefined) headers["content-type"] = "application/json";
  if (auth && token()) headers.authorization = `Bearer ${token()}`;
  const response = await fetch(`${API}${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  let data = null;
  try { data = await response.json(); } catch (_) { /* empty body */ }
  if (!response.ok) {
    const error = new Error(data?.error || `${response.status} ${response.statusText}`);
    error.status = response.status;
    throw error;
  }
  return { data, status: response.status, replayed: response.headers.get("idempotent-replayed") === "true" };
}

function commandError(error) {
  const reasons = { 401: "Operator token missing or not recognised.", 403: "Your role may not do this.", 409: "Someone changed this first. The view was refreshed; check and try again." };
  return reasons[error.status] || error.message;
}

/** Collect refresh requests briefly, so a burst of events is one refetch. */
function queueRefresh(...parts) {
  parts.forEach((p) => state.pending.add(p));
  clearTimeout(state.refreshTimer);
  state.refreshTimer = setTimeout(() => {
    const parts = [...state.pending];
    state.pending.clear();
    refresh(parts);
  }, 350);
}

async function refresh(parts) {
  const want = new Set(parts);
  const jobs = [];
  if (want.has("system")) jobs.push(loadSystem());
  if (want.has("incidents")) jobs.push(loadIncidents());
  if (want.has("vehicles")) jobs.push(loadVehicles());
  if (want.has("shelters")) jobs.push(loadShelters());
  if (want.has("hazards")) jobs.push(loadHazards());
  if (want.has("queue")) jobs.push(loadQueue());
  if (want.has("alerts")) jobs.push(loadAlerts());
  if (want.has("selected") && state.selected) jobs.push(loadDetail(state.selected));
  if (want.has("tracked")) jobs.push(pollTracked());
  await Promise.allSettled(jobs);
  if (want.has("routes") || want.has("incidents")) await loadRoutes();
  renderKpis();
}

async function loadSystem() {
  try {
    const { data } = await api("/system");
    state.system = data;
    dot("relay", data.relay_leader, data.relay_leader ? "Relay: leader running" : "Relay: no leader (events delayed)");
    dot("matcher", data.matcher_leader, data.matcher_leader ? `Matcher: leader · queue ${data.queue_length}` : "Matcher: no leader (no assignment)");
    dot("driver", data.simulated_driver ? null : false, data.simulated_driver ? "Simulated driver: completing missions (demo)" : "Simulated driver: off");
  } catch (error) {
    dot("relay", false, "Relay: API unreachable");
    dot("matcher", false, "Matcher: API unreachable");
    dot("driver", false, "Simulated driver: unknown");
  }
}

async function loadIncidents() {
  try {
    const { data } = await api("/incidents?limit=200");
    state.incidents = data.incidents;
    renderIncidentList();
    renderOverviewIncidents();
    const review = state.incidents.filter((i) => i.display_status === "REVIEW_REQUIRED").length;
    badge("nav-review", review);
  } catch (error) {
    renderFailure("incident-list", error.message);
  }
}

async function loadVehicles() {
  try {
    state.vehicles = (await api("/vehicles")).data.vehicles;
    renderVehicleTable();
    renderVehiclesOnMaps();
  } catch (error) {
    renderFailure("vehicle-table", error.message);
  }
}

async function loadShelters() {
  try {
    state.shelters = (await api("/shelters")).data.shelters;
    renderShelterCards();
    renderSheltersOnMaps();
  } catch (error) {
    renderFailure("shelter-cards", error.message);
  }
}

async function loadHazards() {
  try {
    const { data } = await api("/hazards/active");
    state.hazards = data.geojson;
    state.overlayVersion = data.overlay_version;
    renderHazardsOnMaps();
    renderHazardList();
  } catch (error) {
    renderFailure("hazard-list", error.message);
  }
}

async function loadQueue() {
  try {
    state.queue = (await api("/queue")).data;
    renderQueueTable();
  } catch (error) {
    renderFailure("queue-table", error.message);
  }
}

async function loadAlerts() {
  try {
    state.alerts = (await api("/alerts")).data;
    renderAlerts();
  } catch (error) {
    renderFailure("alert-events", error.message);
  }
}

/** Routes of assigned incidents, for the overview map. */
async function loadRoutes() {
  const assigned = state.incidents.filter((i) => i.display_status === "ASSIGNED").slice(0, 15);
  const keep = new Set(assigned.map((i) => i.incident_id));
  for (const id of [...state.routes.keys()]) if (!keep.has(id)) state.routes.delete(id);
  await Promise.allSettled(assigned.map(async (incident) => {
    const cached = state.routes.get(incident.incident_id);
    if (cached && cached.version === incident.version) return;
    try {
      const { data } = await api(`/incidents/${encodeURIComponent(incident.incident_id)}/route`);
      state.routes.set(incident.incident_id, { version: incident.version, geojson: data });
    } catch (_) { /* no route yet */ }
  }));
  renderRoutesOnOverview();
}

// ------------------------------------------------------------ live feed

function connectFeed() {
  if (!window.EventSource) { startFallbackPolling(); return; }
  const source = new EventSource(`${API}/events`);
  source.onopen = () => {
    dot("feed", true, "Live feed connected");
    stopFallbackPolling();
    queueRefresh("incidents", "vehicles", "queue", "alerts", "hazards", "selected", "tracked");
  };
  source.onerror = () => {
    dot("feed", null, "Live feed reconnecting · polling every 5 s");
    startFallbackPolling();
  };
  Object.keys(EVENT_EFFECTS).forEach((type) => source.addEventListener(type, (message) => onEvent(type, message)));
}

function onEvent(type, message) {
  let event;
  try { event = JSON.parse(message.data); } catch (_) { return; }
  // At-least-once delivery: drop an event already seen.
  const key = event.event_id || message.lastEventId;
  if (state.seen.has(key)) return;
  state.seen.add(key);
  if (state.seen.size > 5000) state.seen = new Set([...state.seen].slice(-2500));
  queueRefresh(...EVENT_EFFECTS[type], "system");
  if (type === "INCIDENT_CRITICAL") showToast("CRITICAL incident", "A critical incident was created or raised.", true);
  if (type === "MISSION_CONVERTED") showToast("Boat-then-road mission", "Victims are cut off by water: a boat goes to the staging point, the road leg waits for 'victim aboard'.", true);
  if (type === "DROPOFF_UNRESOLVED") showToast("Drop-off unresolved", "A vehicle was dispatched without free beds; choose a drop-off.", true);
  if (type === "TASK_ESCALATED") showToast("Task escalated", event.payload?.reason ? `Reason: ${event.payload.reason}` : "A task needs a person.", true);
}

function startFallbackPolling() {
  if (state.fallbackPoll) return;
  state.fallbackPoll = setInterval(() => queueRefresh("incidents", "vehicles", "queue", "alerts", "hazards", "selected", "system"), 5000);
}

function stopFallbackPolling() {
  clearInterval(state.fallbackPoll);
  state.fallbackPoll = null;
}

// ------------------------------------------------------------ maps

function initializeMaps() {
  if (!window.L) return;
  for (const id of ["overview-map", "intake-map", "hazard-map"]) {
    const map = L.map(id, { minZoom: 11, maxZoom: 18, maxBounds: BOUNDS, maxBoundsViscosity: 1 }).setView(ALUVA, 13);
    L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", { maxZoom: 19, attribution: "© OpenStreetMap contributors" }).addTo(map);
    L.circle(ALUVA, { radius: 10000, color: "#245c7c", weight: 1, opacity: .45, fillOpacity: .015, dashArray: "6 7" }).addTo(map);
    state.maps[id] = map;
    state.layers[id] = {
      shelters: L.layerGroup().addTo(map), hazards: L.layerGroup().addTo(map), routes: L.layerGroup().addTo(map),
      vehicles: L.layerGroup().addTo(map), pick: L.layerGroup().addTo(map),
    };
  }
  state.maps["hazard-map"].on("click", (event) => pickSegments(event.latlng));
}

function invalidateMaps() {
  Object.values(state.maps).forEach((map) => setTimeout(() => map.invalidateSize(), 40));
}

function renderSheltersOnMaps() {
  if (!window.L) return;
  Object.values(state.layers).forEach(({ shelters }) => {
    shelters.clearLayers();
    state.shelters.filter((s) => s.position).forEach((s) => {
      const icon = L.divIcon({ className: "hub-marker", html: s.shelter_id.includes("HOSPITAL") ? "✚" : s.shelter_id.includes("MANAPPURAM") ? "≋" : "H", iconSize: [30, 30], iconAnchor: [15, 15] });
      L.marker([s.position.lat, s.position.lon], { icon, zIndexOffset: -100 })
        .bindPopup(`<b>${escapeHtml(s.name || s.shelter_id)}</b><br><small>${escapeHtml(s.shelter_id)}</small><br>Beds ${s.occupancy}/${s.capacity} used<br>Vehicles ${s.available}/${s.fleet} available`)
        .addTo(shelters);
    });
  });
}

/** Vehicles grouped by position (many share a depot), coloured by status. */
function renderVehiclesOnMaps() {
  if (!window.L) return;
  const groups = new Map();
  state.vehicles.filter((v) => v.position).forEach((v) => {
    const key = `${v.position.lat.toFixed(5)},${v.position.lon.toFixed(5)}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(v);
  });
  for (const id of ["overview-map", "intake-map"]) {
    const layer = state.layers[id]?.vehicles;
    if (!layer) continue;
    layer.clearLayers();
    for (const vehicles of groups.values()) {
      const busy = vehicles.filter((v) => v.status !== "AVAILABLE").length;
      const status = busy === vehicles.length ? "ASSIGNED" : "AVAILABLE";
      const icon = L.divIcon({ className: `vehicle-marker ${status}`, html: String(vehicles.length), iconSize: [22, 22], iconAnchor: [-4, 26] });
      const rows = vehicles.map((v) => `${escapeHtml(v.vehicle_id)} · ${escapeHtml(v.asset_type)} · ${escapeHtml(v.status)}${v.task_id ? " · on task" : ""}`).join("<br>");
      L.marker([vehicles[0].position.lat, vehicles[0].position.lon], { icon }).bindPopup(`<b>${vehicles.length} vehicle(s)</b><br><small>${rows}</small>`).addTo(layer);
    }
  }
}

function renderHazardsOnMaps() {
  if (!window.L) return;
  Object.values(state.layers).forEach(({ hazards }) => {
    hazards.clearLayers();
    if (!state.hazards?.features?.length) return;
    L.geoJSON(state.hazards, {
      style: { color: "#b13e36", weight: 7, opacity: .9, dashArray: "4 6" },
      onEachFeature: (f, layer) => layer.bindPopup(`<b>${escapeHtml(f.properties.status)}</b> · ${escapeHtml(f.properties.road_name || "road")}<br>${escapeHtml(f.properties.reason)}<br><small>${escapeHtml(f.properties.actor_id)} · until ${f.properties.valid_until ? formatTime(f.properties.valid_until) : "cleared"}</small>`),
    }).addTo(hazards);
  });
}

function drawRoute(layer, geojson) {
  const pointColor = { pickup: "#a73b35", staging: "#a86612", dropoff: "#245c7c" };
  const pointLabel = { pickup: "Victims", staging: "Staging point (boat ↔ road hand-over)", dropoff: "Drop-off shelter" };
  return L.geoJSON(geojson, {
    style: (feature) => feature.properties.kind === "water"
      ? { color: "#2f6fb0", weight: 5, opacity: .9, dashArray: "6 7" }
      : { color: feature.properties.task_kind === "WATER_EXTRACTION" ? "#8a6a2f" : "#1f6f54", weight: 5, opacity: .85 },
    onEachFeature: (feature, l) => {
      if (feature.properties.kind === "water") l.bindTooltip(`Water leg (${escapeHtml(feature.properties.mode || "")}): unverified estimate, ${duration(feature.properties.travel_time_s)}`);
      else if (feature.properties.kind === "route") l.bindTooltip(`${escapeHtml(feature.properties.task_kind || "Road")} · ${duration(feature.properties.travel_time_s)} · verified live`);
    },
    pointToLayer: (feature, latlng) => L.circleMarker(latlng, {
      radius: 7, weight: 2, color: "#fff", fillColor: pointColor[feature.properties.kind] || "#245c7c", fillOpacity: 1,
    }).bindTooltip(pointLabel[feature.properties.kind] || feature.properties.kind),
  }).addTo(layer);
}

function renderRoutesOnOverview() {
  const layer = state.layers["overview-map"]?.routes;
  if (!layer) return;
  layer.clearLayers();
  for (const { geojson } of state.routes.values()) drawRoute(layer, geojson);
}

// ------------------------------------------------------------ overview

function renderKpis() {
  const open = state.incidents.filter((i) => !["COMPLETED", "REJECTED", "MERGED"].includes(i.display_status));
  setText("kpi-open", number(open.length));
  setText("kpi-review", number(state.incidents.filter((i) => i.display_status === "REVIEW_REQUIRED").length));
  if (state.queue) {
    setText("kpi-queue", number(state.queue.total));
    const tiers = state.queue.tiers;
    setText("kpi-queue-sub", `${tiers.CRITICAL.length} critical · ${tiers.URGENT.length} urgent · ${tiers.STABLE.length} stable`);
  }
  const available = state.vehicles.filter((v) => v.status === "AVAILABLE");
  setText("kpi-vehicles", `${available.length}/${state.vehicles.length}`);
  const byType = ["AMBULANCE", "EVAC_TRUCK", "RESCUE_BOAT"].map((t) => `${available.filter((v) => v.asset_type === t).length} ${shortAsset(t)}`).join(" · ");
  setText("kpi-vehicles-sub", byType);
  setText("kpi-hazards", number(state.hazards?.features?.length || 0));
  setText("kpi-hazards-sub", `overlay version ${state.overlayVersion ?? "—"}`);
}

function renderOverviewIncidents() {
  const target = document.getElementById("overview-incidents");
  const rows = state.incidents.slice(0, 10);
  target.innerHTML = rows.length ? rows.map((i) => `
    <a class="feed-row" href="#" data-open-incident="${escapeHtml(i.incident_id)}">
      <b>${escapeHtml(shortId(i.incident_id))}</b>${statusBadge(i.display_status)}
      <p>${urgencyBadge(i.urgency)} ${escapeHtml(i.required_asset || i.suggested_asset || "asset ?")} · ${i.headcount ?? "?"} people · ${relativeTime(i.created_at)}</p>
    </a>`).join("") : empty("No incidents yet", "Submit a report from Intake.");
}

// ------------------------------------------------------------ intake

function renderScenarios() {
  const grid = document.getElementById("scenario-grid");
  grid.innerHTML = Object.entries(scenarios).map(([key, s]) => `<button type="button" class="scenario ${s.tone}" data-scenario="${key}"><b>${escapeHtml(s.title)}</b><span>${escapeHtml(s.hint)}</span></button>`).join("");
  grid.querySelectorAll("[data-scenario]").forEach((button) => button.addEventListener("click", () => {
    const s = scenarios[button.dataset.scenario];
    document.getElementById("sos-text").value = s.text;
    document.getElementById("gps-lat").value = s.lat ?? "";
    document.getElementById("gps-lon").value = s.lon ?? "";
    document.getElementById("gps-acc").value = s.acc ?? "";
  }));
}

async function submitReport(event) {
  event.preventDefault();
  const button = document.getElementById("intake-btn");
  const lat = numberOrNull("gps-lat"), lon = numberOrNull("gps-lon"), acc = numberOrNull("gps-acc");
  const body = {
    request_id: uuid(),
    device_id: document.getElementById("device-id").value.trim() || `console-${uuid().slice(0, 8)}`,
    raw_text: document.getElementById("sos-text").value.trim(),
    channel: "operations_console",
  };
  if (lat !== null && lon !== null) { body.latitude = lat; body.longitude = lon; }
  if (acc !== null) body.accuracy_m = acc;
  button.disabled = true;
  try {
    const { data } = await api("/reports", { method: "POST", body });
    state.tracked.unshift({ report_id: data.report_id, text: body.raw_text, submitted_at: new Date().toISOString(), report_status: data.status, incident: null });
    state.tracked = state.tracked.slice(0, 8);
    saveSession(TRACK_KEY, state.tracked);
    renderTracked();
    showToast("Report accepted", "Extraction runs on the triage role; this can take about a minute on CPU.");
  } catch (error) {
    showToast("Report rejected", error.message, true);
  } finally {
    button.disabled = false;
  }
}

async function pollTracked() {
  const active = state.tracked.filter((t) => !["COMPLETED", "REJECTED", "MERGED"].includes(t.incident?.display_status));
  if (!active.length) { renderTracked(); return; }
  await Promise.allSettled(active.map(async (t) => {
    const { data } = await api(`/reports/${encodeURIComponent(t.report_id)}`);
    t.report_status = data.status;
    if (data.incident_id) {
      const detail = (await api(`/incidents/${encodeURIComponent(data.incident_id)}`)).data;
      if (!t.incident || detail.version >= t.incident.version) t.incident = detail;
      if (detail.display_status === "ASSIGNED" && !t.route) {
        try { t.route = (await api(`/incidents/${encodeURIComponent(data.incident_id)}/route`)).data; } catch (_) { /* not yet */ }
      }
    }
  }));
  saveSession(TRACK_KEY, state.tracked.map(({ route, ...rest }) => ({ ...rest, incident: rest.incident ? { incident_id: rest.incident.incident_id, version: rest.incident.version, display_status: rest.incident.display_status } : null })));
  renderTracked();
}

function renderTracked() {
  const target = document.getElementById("tracked-reports");
  if (!state.tracked.length) return;
  target.innerHTML = state.tracked.map((t) => {
    const i = t.incident;
    const task = i?.tasks?.[0];
    const shown = i?.display_status;
    const extracted = t.report_status === "EXTRACTED" || (i && t.report_status !== "PENDING");
    const failed = t.report_status === "FAILED";
    const steps = [
      step("Received", "done", formatTime(t.submitted_at)),
      step("Extracted", failed ? "stop" : extracted ? "done" : "active", failed ? "Model unavailable: manual triage" : extracted ? "Candidate facts saved" : "Model running on the triage role"),
      step("Incident", i ? "done" : extracted || failed ? "active" : "", i ? `${i.urgency || "UNKNOWN"} · ${i.location_source || "no location"}` : "Rust urgency + location policy"),
      shown === "REVIEW_REQUIRED" ? step("Review", "stop", (i.review_reasons || [])[0] || "Dispatcher decision needed")
        : shown === "REJECTED" ? step("Rejected", "stop", "Dispatcher rejected")
        : step("Queued", task ? "done" : i ? "active" : "", task ? `Tier rank ${task.priority_rank ?? "?"}` : "Priority queue"),
      shown === "COMPLETED" ? step("Completed", "done", "Vehicle free again")
        : step("Assigned", task?.status === "ASSIGNED" ? "done" : task?.status === "QUEUED" ? "active" : "", task?.assigned_vehicle_id ? `${task.assigned_vehicle_id} · ETA ${duration(task.eta_s)}` : "Matcher: hold → verify → commit"),
    ].join("");
    const link = i ? `<a class="link" href="#" data-open-incident="${escapeHtml(i.incident_id)}">Open incident</a>` : `<span>report ${escapeHtml(shortId(t.report_id))}</span>`;
    return `<div class="tracked"><div class="tracked-head"><div><code>${escapeHtml(shortId(t.report_id))}</code><p>${escapeHtml(t.text)}</p></div>${shown ? statusBadge(shown) : '<span class="status progress">IN PROGRESS</span>'}</div><div class="steps">${steps}</div><div class="tracked-foot"><span>${relativeTime(t.submitted_at)}</span>${link}</div></div>`;
  }).join("");
  const latest = state.tracked.find((t) => t.route);
  const layer = state.layers["intake-map"]?.routes;
  if (layer) {
    layer.clearLayers();
    if (latest) {
      const drawn = drawRoute(layer, latest.route);
      try { state.maps["intake-map"].fitBounds(drawn.getBounds(), { padding: [30, 30], maxZoom: 16 }); } catch (_) { /* empty */ }
      const tag = document.getElementById("intake-map-state");
      tag.textContent = "Route verified live";
      tag.className = "tag";
    }
  }
}

function step(title, cls, text) {
  return `<div class="step ${cls}"><b>${escapeHtml(title)}</b>${escapeHtml(text || "")}</div>`;
}

// ------------------------------------------------------------ incidents

function renderIncidentList() {
  const target = document.getElementById("incident-list");
  const query = document.getElementById("incident-search").value.trim().toLowerCase();
  const filter = document.getElementById("incident-filter").value;
  const rows = state.incidents.filter((i) => {
    if (filter === "DUPLICATES" && !i.possible_duplicates) return false;
    if (!["ALL", "DUPLICATES"].includes(filter) && i.display_status !== filter) return false;
    if (!query) return true;
    return [i.incident_id, i.urgency, i.required_asset, i.display_status, ...(i.assigned_vehicles || [])].join(" ").toLowerCase().includes(query);
  });
  target.innerHTML = rows.length ? rows.map((i) => `
    <div class="incident-row ${state.selected === i.incident_id ? "selected" : ""}" data-incident-id="${escapeHtml(i.incident_id)}">
      <code>${escapeHtml(shortId(i.incident_id))}</code>${statusBadge(i.display_status)}
      <p>${urgencyBadge(i.urgency)} ${escapeHtml(i.required_asset || (i.suggested_asset ? `suggested ${i.suggested_asset}` : "asset unknown"))} · ${i.headcount ?? "?"} people${i.possible_duplicates ? ' · <span class="flag warn">possible duplicate</span>' : ""}</p>
      <small>${relativeTime(i.created_at)} · ${i.report_count} report(s)${(i.assigned_vehicles || []).length ? " · " + escapeHtml(i.assigned_vehicles.join(", ")) : ""}</small>
    </div>`).join("") : empty("No incidents", "Nothing matches this filter.");
  target.querySelectorAll("[data-incident-id]").forEach((row) => row.addEventListener("click", () => selectIncident(row.dataset.incidentId)));
}

async function selectIncident(id) {
  state.selected = id;
  state.detail = null;
  renderIncidentList();
  document.getElementById("incident-detail").innerHTML = '<div class="skeleton">Loading incident…</div>';
  await loadDetail(id);
}

async function loadDetail(id) {
  try {
    const { data } = await api(`/incidents/${encodeURIComponent(id)}`);
    if (state.selected !== id) return;
    // Never move backwards: ignore an answer older than what is shown.
    if (state.detail && state.detail.incident_id === id && data.version < state.detail.version) return;
    const changed = !state.detail || state.detail.version !== data.version || state.detail.incident_id !== id;
    state.detail = data;
    if (changed) renderDetail(data);
  } catch (error) {
    if (state.selected === id) renderFailure("incident-detail", error.message);
  }
}

function renderDetail(i) {
  const target = document.getElementById("incident-detail");
  const reports = i.reports || [];
  const reviewing = i.display_status === "REVIEW_REQUIRED";
  target.innerHTML = `
    <div class="detail-title"><div><code>${escapeHtml(i.incident_id)}</code><p>Created ${formatTime(i.created_at)} · updated ${relativeTime(i.updated_at)} · version ${i.version}</p></div><div class="badge-row">${urgencyBadge(i.urgency)}${statusBadge(i.display_status)}</div></div>
    ${i.merged_into ? `<div class="notice" style="margin-top:14px"><b>Merged</b><span>This incident was merged into <a class="link" href="#" data-open-incident="${escapeHtml(i.merged_into)}">${escapeHtml(shortId(i.merged_into))}</a>. Its reports and any vehicle moved there.</span></div>` : ""}
    ${i.split_from ? `<div class="notice warning" style="margin-top:14px"><b>Split</b><span>Split off <a class="link" href="#" data-open-incident="${escapeHtml(i.split_from)}">${escapeHtml(shortId(i.split_from))}</a> by a dispatcher.</span></div>` : ""}
    ${(i.urgency_flags || []).length ? `<div class="badge-row" style="margin-top:10px">${i.urgency_flags.map((f) => `<span class="flag">${escapeHtml(f)}</span>`).join("")}</div>` : ""}
    <div class="facts" style="margin-top:14px">
      ${fact("People", i.headcount)}${fact("Required asset", i.required_asset)}${fact("Suggested asset (F-2)", i.suggested_asset)}
      ${fact("Location source (F-1)", i.location_source)}${fact("Victim location", i.location ? `${i.location.lat.toFixed(5)}, ${i.location.lon.toFixed(5)}` : null)}${fact("Caller location", i.caller_location ? `${i.caller_location.lat.toFixed(5)}, ${i.caller_location.lon.toFixed(5)}` : null)}
      ${fact("Rule urgency", i.rule_urgency)}${fact("Review priority", i.review_priority)}${fact("Callback needed", i.callback_required ? "yes (UNKNOWN urgency)" : "no")}
    </div>
    ${(i.review_reasons || []).length ? `<div class="notice warning" style="margin-top:14px"><b>Review reasons</b><span>${i.review_reasons.map(escapeHtml).join(" · ")}</span></div>` : ""}
    ${reviewing ? reviewFormHtml(i) : ""}
    ${missionHtml(i)}
    <h3 class="section-title">Tasks</h3>${(i.tasks || []).length ? i.tasks.map(taskHtml).join("") : '<p class="supporting small">No task yet.</p>'}
    <div class="route-record" id="detail-route" hidden><div class="route-record-head"><h3>Route (verified live before commit)</h3><span class="tag" id="detail-route-meta"></span></div><div class="incident-route-map" id="detail-route-map"></div></div>
    ${(i.duplicates || []).length ? `<h3 class="section-title">Possible duplicates (rule D-3)</h3>${i.duplicates.map((d) => duplicateHtml(i, d)).join("")}<div class="duplicate-note">Flagged for a person, never merged automatically. Merging keeps exactly one vehicle sent: the survivor's own, or the absorbed one's if only it has one.</div><div id="merge-form-slot"></div>` : ""}
    <h3 class="section-title">Reports (${reports.length})</h3>${reports.map((r) => reportHtml(r, i)).join("")}
    ${(i.reservations || []).length ? `<h3 class="section-title">Reservations</h3><div class="table-wrap"><table><thead><tr><th>Kind</th><th>Resource</th><th>Status</th><th>Created</th><th>Released</th></tr></thead><tbody>${i.reservations.map((r) => `<tr><td>${escapeHtml(r.kind)}</td><td><code>${escapeHtml(r.vehicle_id || `${r.shelter_id} · ${r.beds} beds`)}</code></td><td>${escapeHtml(r.status)}</td><td>${formatTime(r.created_at)}</td><td>${r.released_at ? formatTime(r.released_at) + " · " + escapeHtml(r.release_reason || "") : "—"}</td></tr>`).join("")}</tbody></table></div>` : ""}
    <div class="timeline"><h3>Audit trail</h3>${(i.audit || []).slice().sort((a, b) => String(a.occurred_at).localeCompare(String(b.occurred_at))).map((a) => `<div class="timeline-row"><b>${escapeHtml(a.action)}</b><p>${escapeHtml(a.reason || "")}${a.from_state ? ` (${escapeHtml(a.from_state)} → ${escapeHtml(a.to_state || "")})` : ""}</p><small>${escapeHtml(a.actor_id || "")} · ${formatTime(a.occurred_at)}</small></div>`).join("") || '<p class="supporting small">No audit events.</p>'}</div>`;
  if (reviewing) bindReviewForm(i);
  target.querySelectorAll("[data-trigger]").forEach((button) => button.addEventListener("click", () => triggerRoadLeg(button.dataset.trigger, Number(button.dataset.version), button.dataset.reason)));
  document.getElementById("cancel-mission")?.addEventListener("click", () => openCancelMission(i));
  target.querySelectorAll("[data-merge]").forEach((button) => button.addEventListener("click", () => openMergeForm(i, button.dataset.merge)));
  target.querySelectorAll("[data-split]").forEach((button) => button.addEventListener("click", () => openSplitForm(i, button.dataset.split)));
  target.querySelectorAll("[data-complete-task]").forEach((button) => button.addEventListener("click", () => completeTask(button.dataset.completeTask, Number(button.dataset.version))));
  if ((i.tasks || []).some((t) => t.status === "ASSIGNED" || t.status === "COMPLETED") || i.mission) showDetailRoute(i.incident_id);
}

function missionHtml(i) {
  if (!i.mission) return "";
  const tasks = i.tasks || [];
  const boat = tasks.find((t) => t.kind === "WATER_EXTRACTION" && t.status !== "CANCELLED");
  const road = tasks.find((t) => t.kind === "ROAD_EVACUATION" && t.status !== "CANCELLED");
  const disabled = token() ? "" : 'disabled title="Paste an operator token to act"';
  if (i.mission !== "BOAT_THEN_ROAD") {
    return `<div class="notice" style="margin-top:14px"><b>Boat plan cancelled</b><span>A dispatcher cancelled the boat-then-road plan; the road pickup waits for the water to recede and is not converted again.</span></div>`;
  }
  const pending = road?.status === "PENDING_TRIGGER";
  const actions = [];
  if (boat && pending && ["QUEUED", "ASSIGNED"].includes(boat.status)) actions.push(`<button class="button primary compact" data-trigger="${escapeHtml(boat.task_id)}" data-version="${boat.version}" data-reason="VICTIM_ABOARD" ${disabled}>Victim aboard: send road vehicle</button>`);
  if (road && pending) actions.push(`<button class="button ghost compact" data-trigger="${escapeHtml(road.task_id)}" data-version="${road.version}" data-reason="EARLY" ${disabled}>Send road vehicle now</button>`);
  if (boat && boat.status !== "COMPLETED") actions.push(`<button class="button ghost compact" id="cancel-mission" ${disabled}>Cancel boat plan</button>`);
  const leg = (title, t, note) => `<div class="mission-leg ${t ? t.status.toLowerCase() : ""}"><b>${escapeHtml(title)}</b><span>${t ? statusBadge(t.status) : ""} ${t?.assigned_vehicle_id ? escapeHtml(t.assigned_vehicle_id) : ""}</span><small>${note}</small></div>`;
  return `<div class="mission-panel"><div class="panel-head"><div><p class="kicker">Cut off by flood water</p><h3>Boat-then-road mission</h3></div><span class="tag">staging ${escapeHtml(shortId(i.staging_junction_id || ""))}</span></div>
    <div class="mission-legs">
      ${leg("1 · Boat by road to staging", boat, boat?.eta_s != null ? `ETA ${duration(boat.eta_s)} incl. water` : "towed on a trailer, road verified live")}
      ${leg("2 · Water to the victims", boat, `unverified estimate ${duration(boat?.water_eta_s)}: crew confirms depth and current`)}
      ${leg("3 · Road from staging", road, road?.status === "PENDING_TRIGGER" ? "waits for 'victim aboard'" : road?.trigger ? `sent: ${escapeHtml(road.trigger)}` : "to the drop-off shelter")}
    </div>
    ${actions.length ? `<div class="review-actions" style="margin-top:10px">${actions.join("")}</div>` : ""}<div id="cancel-mission-slot"></div></div>`;
}

async function triggerRoadLeg(taskId, version, reason) {
  try {
    await api(`/tasks/${encodeURIComponent(taskId)}/trigger`, { method: "POST", auth: true, body: { request_id: uuid(), expected_version: version, reason } });
    showToast("Road leg queued", "The matcher sends a road vehicle to the staging point.");
  } catch (error) {
    showToast("Not sent", commandError(error), true);
  }
  state.detail = null;
  refresh(["selected", "incidents", "queue"]);
}

function openCancelMission(i) {
  const slot = document.getElementById("cancel-mission-slot");
  slot.innerHTML = `<form class="review-form"><h3>Cancel the boat plan</h3><p class="supporting small">The boat is released and the incident goes back to a road pickup, which waits for the water to recede (it is not converted again).</p>
    <label>Reason (audited)</label><textarea class="cm-reason" rows="2" maxlength="500" placeholder="e.g. Current too strong for the boat; fire service helicopter requested"></textarea>
    <div class="review-actions"><button type="button" class="button danger cm-go">Cancel boat plan</button><button type="button" class="button ghost cm-back">Keep the plan</button></div></form>`;
  slot.querySelector(".cm-back").addEventListener("click", () => { slot.innerHTML = ""; });
  slot.querySelector(".cm-go").addEventListener("click", async () => {
    const reason = slot.querySelector(".cm-reason").value.trim();
    if (!reason) { showToast("Reason required", "Write why the plan is cancelled.", true); return; }
    try {
      await api(`/incidents/${encodeURIComponent(i.incident_id)}/mission/cancel`, { method: "POST", auth: true, body: { request_id: uuid(), expected_version: i.version, reason } });
      showToast("Boat plan cancelled", "The boat is free; the road pickup waits.");
    } catch (error) {
      showToast("Not cancelled", commandError(error), true);
    }
    state.detail = null;
    refresh(["selected", "incidents", "vehicles", "queue"]);
  });
}

function taskHtml(t) {
  const canComplete = t.status === "ASSIGNED";
  const action = canComplete ? `<button class="button primary compact" data-complete-task="${escapeHtml(t.task_id)}" data-version="${t.version}" ${token() ? "" : 'disabled title="Paste an operator token to act"'}>Mark complete</button>` : statusBadge(t.status);
  return `<div class="task-row"><div><b>${escapeHtml(t.kind)}</b> <code class="report-meta">${escapeHtml(shortId(t.task_id))} · v${t.version}</code>
    <dl><dt>Asset</dt><dt>Vehicle</dt><dt>ETA</dt><dt>Overlay</dt>
        <dd>${escapeHtml(t.required_asset || "—")}</dd><dd>${escapeHtml(t.assigned_vehicle_id || "—")}</dd><dd>${t.eta_s != null ? duration(t.eta_s) : "—"}</dd><dd>${t.overlay_version ?? "—"}</dd></dl>
    ${(t.escalations || []).length ? `<div class="badge-row" style="margin-top:6px">${t.escalations.map((e) => `<span class="flag warn">${escapeHtml(e)}</span>`).join("")}</div>` : ""}
    </div>${action}</div>`;
}

function reportHtml(r, incident) {
  let extraction = null;
  try { extraction = r.extraction_json ? JSON.parse(r.extraction_json) : null; } catch (_) { /* unreadable */ }
  const triage = extraction?.triage;
  const profile = triage?.victim_profile || {};
  const facts = triage ? [
    ["Headcount", triage.headcount], ["Asset", triage.required_asset], ["Place said", triage.victim_location_raw],
    ["On scene", profile.reporter_on_scene], ["Relation", profile.reporter_relation], ["Water", [profile.water_level, profile.water_trend].filter(Boolean).join(" · ")],
    ["Medical", (profile.medical_conditions || []).join(", ")], ["Vulnerable", (profile.vulnerable_groups || []).join(", ")], ["Model urgency (raise only)", profile.suggested_urgency],
  ] : [];
  const splittable = incident && ["FOLLOW_UP", "MERGED"].includes(r.kind) && !["MERGED", "REJECTED"].includes(incident.display_status);
  const split = splittable ? `<div class="report-actions"><button class="button ghost compact" data-split="${escapeHtml(r.report_id)}" ${token() ? "" : 'disabled title="Paste an operator token to act"'}>Split into new incident</button></div><div id="split-slot-${escapeHtml(r.report_id)}"></div>` : "";
  return `<div class="report-card"><div class="report-meta"><span>${escapeHtml(r.kind || "")}</span><span>${escapeHtml(r.rule || "")}</span><span>${escapeHtml(r.device_id || "")}</span><span>${escapeHtml(r.channel || "")}</span><span>${formatTime(r.received_at)}</span><span>extraction ${escapeHtml(r.extraction_status || "")}</span></div>
    <div class="raw-message">${escapeHtml(r.raw_text)}</div>
    ${extraction?.error ? `<div class="notice warning"><b>Extraction failed</b><span>${escapeHtml(extraction.error)}</span></div>` : ""}
    ${facts.length ? `<div class="facts">${facts.map(([k, v]) => fact(k, v === true ? "yes" : v === false ? "no" : v)).join("")}</div>` : ""}
    ${triage?.inference ? `<p class="route-proof">Model ${escapeHtml(triage.inference.model)}${triage.inference.adapter_id ? " + " + escapeHtml(triage.inference.adapter_id) : ""} · ${escapeHtml(triage.inference.schema_version)} · ${number(triage.inference.latency_ms)} ms</p>` : ""}${split}</div>`;
}

async function showDetailRoute(incidentId) {
  try {
    const { data } = await api(`/incidents/${encodeURIComponent(incidentId)}/route`);
    if (state.selected !== incidentId) return;
    const box = document.getElementById("detail-route");
    box.hidden = false;
    const props = data.features.find((f) => f.properties.kind === "route")?.properties || {};
    setText("detail-route-meta", `${(props.distance_km || 0).toFixed(2)} km · ${duration(props.travel_time_s)} · ${props.segments || 0} segments`);
    const container = document.getElementById("detail-route-map");
    const map = L.map(container, { minZoom: 11, maxZoom: 18, maxBounds: BOUNDS }).setView(ALUVA, 13);
    L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", { maxZoom: 19, attribution: "© OpenStreetMap contributors" }).addTo(map);
    const drawn = drawRoute(map, data);
    setTimeout(() => { map.invalidateSize(); try { map.fitBounds(drawn.getBounds(), { padding: [25, 25], maxZoom: 16 }); } catch (_) { /* empty */ } }, 50);
  } catch (_) { /* no route */ }
}

function reviewFormHtml(i) {
  const disabled = token() ? "" : "disabled";
  const asset = i.required_asset || i.suggested_asset || "AMBULANCE";
  return `<form class="review-form" id="review-form">
    <h3>Dispatcher review</h3><p class="supporting small">Confirm the facts (call the reporter back if needed), click the victims' location on the map, then approve to queue a task, or reject.${token() ? "" : " <b>Paste an operator token to act.</b>"}</p>
    <div class="form-row"><div><label for="rv-asset">Vehicle type</label><select id="rv-asset">${["AMBULANCE", "EVAC_TRUCK", "RESCUE_BOAT"].map((a) => `<option ${a === asset ? "selected" : ""}>${a}</option>`).join("")}</select></div>
    <div><label for="rv-headcount">People</label><input id="rv-headcount" type="number" min="1" max="500" value="${i.headcount || 1}"></div></div>
    <div class="form-row"><div><label for="rv-urgency">Urgency</label><select id="rv-urgency"><option value="">Keep ${escapeHtml(i.urgency || "UNKNOWN")}</option>${["CRITICAL", "URGENT", "UNKNOWN", "STABLE"].map((u) => `<option>${u}</option>`).join("")}</select></div>
    <div><label>Victim location</label><div class="locked-field" id="rv-location">Click the map</div></div></div>
    <div class="review-map" id="rv-map"></div>
    <label for="rv-reason">Reason (audited)</label><textarea id="rv-reason" rows="2" maxlength="500" placeholder="e.g. Called back: 3 people, ground floor, near Bank Junction"></textarea>
    <div class="review-actions"><button type="button" class="button primary" id="rv-approve" ${disabled}>Approve and queue</button><button type="button" class="button danger" id="rv-reject" ${disabled}>Reject</button></div>
  </form>`;
}

function bindReviewForm(i) {
  const start = i.location || i.caller_location || (i.reports || []).filter((r) => r.latitude != null).map((r) => ({ lat: r.latitude, lon: r.longitude }))[0] || null;
  const map = L.map("rv-map", { minZoom: 11, maxZoom: 18, maxBounds: BOUNDS }).setView(start ? [start.lat, start.lon] : ALUVA, start ? 15 : 13);
  L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", { maxZoom: 19, attribution: "© OpenStreetMap contributors" }).addTo(map);
  let marker = null;
  let point = null;
  const place = (lat, lon) => {
    point = { lat, lon };
    if (marker) marker.setLatLng([lat, lon]); else marker = L.marker([lat, lon], { draggable: true }).addTo(map).on("dragend", (e) => place(e.target.getLatLng().lat, e.target.getLatLng().lng));
    setText("rv-location", `${lat.toFixed(5)}, ${lon.toFixed(5)} (snapped to nearest junction)`);
  };
  if (start && start.lat >= BOUNDS[0][0] && start.lat <= BOUNDS[1][0]) place(start.lat, start.lon);
  map.on("click", (e) => place(e.latlng.lat, e.latlng.lng));
  setTimeout(() => map.invalidateSize(), 50);
  const send = async (decision) => {
    const reason = document.getElementById("rv-reason").value.trim();
    if (!reason) { showToast("Reason required", "Write why you approve or reject; it is audited.", true); return; }
    const body = { request_id: uuid(), expected_version: i.version, decision, reason };
    if (decision === "APPROVE") {
      if (!point) { showToast("Location required", "Click the victims' location on the map.", true); return; }
      Object.assign(body, {
        required_asset: document.getElementById("rv-asset").value,
        headcount: Number(document.getElementById("rv-headcount").value),
        latitude: point.lat, longitude: point.lon,
        urgency: document.getElementById("rv-urgency").value || undefined,
      });
    }
    try {
      await api(`/incidents/${encodeURIComponent(i.incident_id)}/review`, { method: "POST", body, auth: true });
      showToast(decision === "APPROVE" ? "Approved" : "Rejected", decision === "APPROVE" ? "Task queued; the matcher assigns a vehicle within seconds." : "The incident was closed.");
    } catch (error) {
      showToast("Review not applied", commandError(error), true);
    }
    state.detail = null;
    refresh(["selected", "incidents", "queue", "alerts"]);
  };
  document.getElementById("rv-approve").addEventListener("click", () => send("APPROVE"));
  document.getElementById("rv-reject").addEventListener("click", () => send("REJECT"));
}

function duplicateHtml(incident, d) {
  const open = !d.resolution && !["MERGED", "REJECTED", "COMPLETED"].includes(d.status) && !["MERGED", "REJECTED", "COMPLETED"].includes(incident.display_status);
  const action = d.resolution ? `<span class="flag">${escapeHtml(d.resolution)}</span>`
    : open ? `<button class="button ghost compact" data-merge="${escapeHtml(d.incident_id)}" ${token() ? "" : 'disabled title="Paste an operator token to act"'}>Merge…</button>` : "";
  return `<div class="compact-row hazard-row"><div><b><a class="link" href="#" data-open-incident="${escapeHtml(d.incident_id)}">${escapeHtml(shortId(d.incident_id))}</a> · ${escapeHtml(d.status)}</b><small>${d.distance_m != null ? Math.round(d.distance_m) + " m apart" : "distance unknown"}</small></div>${action}</div>`;
}

/** Rank for choosing the survivor: an incident further along keeps its vehicle. */
function progressRank(status) {
  return { ASSIGNED: 3, QUEUED: 2, NEEDS_TASK: 1, TRIAGE_APPROVED: 1, REVIEW_REQUIRED: 0 }[status] ?? 1;
}

async function openMergeForm(incident, otherId) {
  const slot = document.getElementById("merge-form-slot");
  slot.innerHTML = '<div class="skeleton">Loading the other incident…</div>';
  let other;
  try { other = (await api(`/incidents/${encodeURIComponent(otherId)}`)).data; } catch (error) { slot.innerHTML = ""; showToast("Cannot merge", error.message, true); return; }
  const pair = [incident, other].sort((a, b) => progressRank(b.display_status) - progressRank(a.display_status));
  let survivor = pair[0], absorbed = pair[1];
  const effect = () => {
    const s = survivor.display_status, a = absorbed.display_status;
    if (s === "ASSIGNED" && a === "ASSIGNED") return `${shortId(absorbed.incident_id)}'s vehicle is released; ${shortId(survivor.incident_id)} keeps its own.`;
    if (a === "ASSIGNED") return `${shortId(absorbed.incident_id)}'s vehicle moves to ${shortId(survivor.incident_id)} (already on its way, never recalled).`;
    if (s === "QUEUED") return `${shortId(absorbed.incident_id)}'s queued task is cancelled; one task remains.`;
    if (a === "QUEUED") return `${shortId(absorbed.incident_id)}'s queued task moves to ${shortId(survivor.incident_id)}.`;
    return "No vehicle or task is affected.";
  };
  const render = () => {
    slot.innerHTML = `<form class="review-form" id="merge-form"><h3>Merge duplicates (D-4)</h3>
      <div class="facts">${fact("Keeps (survivor)", `${shortId(survivor.incident_id)} · ${survivor.display_status}`)}${fact("Absorbed", `${shortId(absorbed.incident_id)} · ${absorbed.display_status}`)}${fact("People after merge", Math.max(survivor.headcount || 0, absorbed.headcount || 0) || "?")}</div>
      <p class="supporting small"><b>Vehicle:</b> ${escapeHtml(effect())} Urgency only goes up; all reports are kept.</p>
      <label for="mg-reason">Reason (audited)</label><textarea id="mg-reason" rows="2" maxlength="500" placeholder="e.g. Same family: called back, both numbers belong to the Varghese household"></textarea>
      <div class="review-actions"><button type="button" class="button primary" id="mg-go">Merge</button><button type="button" class="button ghost" id="mg-swap">Swap survivor</button><button type="button" class="button ghost" id="mg-cancel">Cancel</button></div></form>`;
    document.getElementById("mg-swap").addEventListener("click", () => { [survivor, absorbed] = [absorbed, survivor]; render(); });
    document.getElementById("mg-cancel").addEventListener("click", () => { slot.innerHTML = ""; });
    document.getElementById("mg-go").addEventListener("click", async () => {
      const reason = document.getElementById("mg-reason").value.trim();
      if (!reason) { showToast("Reason required", "Every merge records why.", true); return; }
      try {
        const { data } = await api(`/incidents/${encodeURIComponent(survivor.incident_id)}/merge`, { method: "POST", auth: true,
          body: { request_id: uuid(), absorbed_id: absorbed.incident_id, survivor_version: survivor.version, absorbed_version: absorbed.version, reason } });
        showToast("Merged", `${shortId(absorbed.incident_id)} → ${shortId(survivor.incident_id)} · ${data.released_vehicles.length ? "vehicle released" : data.moved_tasks.length ? "task moved" : "tasks combined"}`);
        state.detail = null;
        selectIncident(survivor.incident_id);
        queueRefresh("incidents", "vehicles", "queue");
      } catch (error) {
        showToast("Not merged", commandError(error), true);
      }
    });
  };
  render();
}

function openSplitForm(incident, reportId) {
  const slot = document.getElementById(`split-slot-${reportId}`);
  slot.innerHTML = `<form class="review-form"><h3>Split this report off</h3><p class="supporting small">It becomes its own incident for review (nothing is dispatched until a dispatcher approves). This incident keeps its other reports; its headcount is recalculated, its urgency is not lowered.</p>
    <label>Reason (audited)</label><textarea class="sp-reason" rows="2" maxlength="500" placeholder="e.g. Shared phone at the relief camp: a different family"></textarea>
    <div class="review-actions"><button type="button" class="button warning sp-go">Split</button><button type="button" class="button ghost sp-cancel">Cancel</button></div></form>`;
  slot.querySelector(".sp-cancel").addEventListener("click", () => { slot.innerHTML = ""; });
  slot.querySelector(".sp-go").addEventListener("click", async () => {
    const reason = slot.querySelector(".sp-reason").value.trim();
    if (!reason) { showToast("Reason required", "Every split records why.", true); return; }
    try {
      const { data } = await api(`/incidents/${encodeURIComponent(incident.incident_id)}/split`, { method: "POST", auth: true,
        body: { request_id: uuid(), report_id: reportId, expected_version: incident.version, reason } });
      showToast("Split", `New review incident ${shortId(data.new_incident_id)}.`);
      state.detail = null;
      selectIncident(data.new_incident_id);
      queueRefresh("incidents");
    } catch (error) {
      showToast("Not split", commandError(error), true);
    }
  });
}

async function setVehicleStatus(vehicleId, status, version, reason) {
  try {
    const { data } = await api(`/vehicles/${encodeURIComponent(vehicleId)}/status`, { method: "POST", auth: true,
      body: { request_id: uuid(), status, expected_version: version, reason } });
    showToast(status === "AVAILABLE" ? "Back in service" : "Out of service", data.requeued_task ? `Its task ${shortId(data.requeued_task)} is back in the queue; another vehicle will be sent.` : vehicleId);
  } catch (error) {
    showToast("Status not changed", commandError(error), true);
  }
  queueRefresh("vehicles", "queue", "incidents");
}

async function completeTask(taskId, version) {
  if (!confirm("Mark this task complete? The vehicle becomes available at the drop-off shelter.")) return;
  try {
    await api(`/tasks/${encodeURIComponent(taskId)}/complete`, { method: "POST", body: { request_id: uuid(), expected_version: version, note: "completed from the console" }, auth: true });
    showToast("Task complete", "The vehicle is available again.");
  } catch (error) {
    showToast("Not completed", commandError(error), true);
  }
  state.detail = null;
  refresh(["selected", "incidents", "vehicles", "queue"]);
}

// ------------------------------------------------------------ hazards

async function pickSegments(latlng) {
  const layer = state.layers["hazard-map"].pick;
  layer.clearLayers();
  L.circleMarker(latlng, { radius: 5, color: "#a73b35" }).addTo(layer);
  try {
    const { data } = await api(`/segments/near?lat=${latlng.lat}&lon=${latlng.lng}`);
    state.segments = data.segments;
    state.picked = null;
    renderSegmentChoices();
  } catch (error) {
    state.segments = [];
    renderSegmentChoices(error.message);
  }
}

function renderSegmentChoices(message) {
  const target = document.getElementById("segment-choices");
  if (message) { target.innerHTML = `<p class="supporting small">${escapeHtml(message)}</p>`; return; }
  if (!state.segments.length) { target.innerHTML = ""; return; }
  const chosen = state.segments.find((s) => s.segment_id === state.picked);
  target.innerHTML = state.segments.map((s) => `<div class="segment-choice ${s.segment_id === state.picked ? "selected" : ""}" data-segment="${escapeHtml(s.segment_id)}"><input type="radio" name="segment" ${s.segment_id === state.picked ? "checked" : ""}><div><b>${escapeHtml(s.road_name || s.road_class || "Unnamed road")}</b>${s.hazard_status ? ` <span class="flag warn">${escapeHtml(s.hazard_status)}</span>` : ""}<small>${escapeHtml(s.segment_id)} · ${Math.round(s.length_m || 0)} m</small></div></div>`).join("")
    + (chosen ? `<form class="hazard-form" id="hazard-form"><div class="form-row"><div><label for="hz-status">Status</label><select id="hz-status"><option>FLOODED</option><option>BLOCKED</option></select></div><div><label for="hz-hours">Duration (hours, empty = until cleared)</label><input id="hz-hours" type="number" min="1" max="168" value="6"></div></div><label for="hz-reason">Reason (audited)</label><textarea id="hz-reason" rows="2" maxlength="500" placeholder="e.g. Field team: road under 60 cm of water"></textarea><button class="button danger full" type="submit" ${token() ? "" : 'disabled title="Paste an operator token to act"'}>Apply hazard</button></form>` : "");
  target.querySelectorAll("[data-segment]").forEach((row) => row.addEventListener("click", () => {
    state.picked = row.dataset.segment;
    const s = state.segments.find((x) => x.segment_id === state.picked);
    const layer = state.layers["hazard-map"].pick;
    layer.clearLayers();
    L.polyline([[s.from.lat, s.from.lon], [s.to.lat, s.to.lon]], { color: "#a73b35", weight: 8, opacity: .8 }).addTo(layer);
    renderSegmentChoices();
  }));
  document.getElementById("hazard-form")?.addEventListener("submit", applyHazard);
}

async function applyHazard(event) {
  event.preventDefault();
  const reason = document.getElementById("hz-reason").value.trim();
  if (!reason) { showToast("Reason required", "Every hazard records why it was applied.", true); return; }
  const hours = document.getElementById("hz-hours").value;
  const body = { request_id: uuid(), segment_id: state.picked, status: document.getElementById("hz-status").value, reason };
  if (hours) body.duration_hours = Number(hours);
  try {
    const { data } = await api("/hazards", { method: "POST", body, auth: true });
    showToast("Hazard applied", `Overlay version ${data.overlay_version}. New routes avoid this segment.`);
    state.segments = [];
    state.picked = null;
    state.layers["hazard-map"].pick.clearLayers();
    renderSegmentChoices();
  } catch (error) {
    showToast("Hazard not applied", commandError(error), true);
  }
  queueRefresh("hazards", "system");
}

function renderHazardList() {
  const target = document.getElementById("hazard-list");
  const features = state.hazards?.features || [];
  setText("hazard-count", `${features.length} active`);
  target.innerHTML = features.length ? features.map((f) => {
    const p = f.properties;
    return `<div class="compact-row hazard-row"><div><b>${escapeHtml(p.status)} · ${escapeHtml(p.road_name || "road")}</b><small>${escapeHtml(p.reason)} · ${escapeHtml(p.actor_id)} · until ${p.valid_until ? formatTime(p.valid_until) : "cleared"}</small><small>${escapeHtml(p.segment_id)}</small></div><button class="button ghost compact" data-clear="${escapeHtml(p.segment_id)}" ${token() ? "" : "disabled"}>Clear</button></div>`;
  }).join("") : empty("No active hazards", "The overlay is clear. Click a road on the map to close it.");
  target.querySelectorAll("[data-clear]").forEach((button) => button.addEventListener("click", () => clearHazard(button.dataset.clear)));
}

async function clearHazard(segmentId) {
  const reason = prompt("Why is this road open again? (audited)", "Field team: water receded, road passable");
  if (!reason) return;
  try {
    await api("/hazards/clear", { method: "POST", body: { segment_id: segmentId, reason }, auth: true });
    showToast("Hazard cleared", "The segment is routable again.");
  } catch (error) {
    showToast("Not cleared", commandError(error), true);
  }
  queueRefresh("hazards", "system");
}

// ------------------------------------------------------------ fleet & beds

function renderShelterCards() {
  const target = document.getElementById("shelter-cards");
  target.innerHTML = state.shelters.map((s) => {
    const pct = s.capacity ? Math.min(100, Math.round((s.occupancy / s.capacity) * 100)) : 0;
    return `<article class="resource-card shelter-card"><h3>${escapeHtml(s.name || s.shelter_id)}</h3><code>${escapeHtml(s.shelter_id)}</code>
      <div class="capacity"><div class="capacity-head"><span>Beds used</span><b>${s.occupancy}/${s.capacity}</b></div><div class="bar"><i style="width:${pct}%"></i></div></div>
      <div class="capacity-head"><span>Vehicles based here</span><b>${s.available}/${s.fleet} available</b></div></article>`;
  }).join("") || empty("No shelters", "Run migrate_ops to seed the pilot.");
}

function renderQueueTable() {
  const target = document.getElementById("queue-table");
  const rows = ["CRITICAL", "URGENT", "STABLE"].flatMap((tier) => (state.queue?.tiers?.[tier] || []).map((t) => ({ tier, ...t })));
  setText("queue-total", `${state.queue?.total ?? 0} waiting`);
  target.innerHTML = rows.length ? rows.map((t) => `<tr><td>${urgencyBadge(t.tier)}</td><td><a class="link" href="#" data-open-incident="${escapeHtml(t.incident_id)}"><code>${escapeHtml(shortId(t.task_id))}</code></a></td><td>${escapeHtml(t.required_asset)}</td><td>${t.persons}</td><td>${duration(t.waited_s)}</td><td>${t.verify_failures}</td><td>${(t.escalations || []).map((e) => `<span class="flag warn">${escapeHtml(e)}</span>`).join(" ") || "—"}</td></tr>`).join("")
    : '<tr><td colspan="7" class="muted-cell">Nothing waiting: every queued task has a vehicle.</td></tr>';
}

function renderVehicleTable() {
  const target = document.getElementById("vehicle-table");
  const filter = document.getElementById("vehicle-filter").value;
  const rows = state.vehicles.filter((v) => filter === "ALL" || v.status === filter);
  const disabled = token() ? "" : 'disabled title="Paste an operator token to act"';
  target.innerHTML = rows.length ? rows.map((v) => {
    const action = v.status === "OUT_OF_SERVICE"
      ? `<button class="button ghost compact" data-vehicle-status="AVAILABLE" data-vehicle="${escapeHtml(v.vehicle_id)}" data-version="${v.version}" ${disabled}>Return to service</button>`
      : `<button class="button ghost compact" data-vehicle-status="OUT_OF_SERVICE" data-vehicle="${escapeHtml(v.vehicle_id)}" data-version="${v.version}" data-busy="${v.task_id ? "1" : ""}" ${disabled}>Out of service</button>`;
    return `<tr><td><code>${escapeHtml(v.vehicle_id)}</code></td><td>${escapeHtml(v.asset_type)}</td><td>${vehicleStatus(v.status)}</td><td>${v.capacity_persons}</td><td><code>${escapeHtml(v.home_shelter_id || "")}</code></td><td>${v.position ? `${v.position.lat.toFixed(4)}, ${v.position.lon.toFixed(4)}` : "—"}</td><td>${v.incident_id ? `<a class="link" href="#" data-open-incident="${escapeHtml(v.incident_id)}">${escapeHtml(shortId(v.task_id))}</a>` : "—"}</td><td>${action}</td></tr>`;
  }).join("") : '<tr><td colspan="8" class="muted-cell">No vehicles match.</td></tr>';
  target.querySelectorAll("[data-vehicle-status]").forEach((button) => button.addEventListener("click", () => {
    const row = button.closest("tr");
    const next = row.nextElementSibling;
    if (next?.classList.contains("inline-confirm")) { next.remove(); return; }
    const busy = button.dataset.busy === "1";
    const status = button.dataset.vehicleStatus;
    const confirmRow = document.createElement("tr");
    confirmRow.className = "inline-confirm";
    confirmRow.innerHTML = `<td colspan="8"><div class="inline-form"><span>${status === "AVAILABLE" ? "Return to service" : busy ? "<b>Breakdown:</b> its task goes back to the queue and another vehicle is sent." : "Take out of service"}</span><input placeholder="Reason (audited)" maxlength="500"><button class="button ${busy ? "danger" : "primary"} compact">Confirm</button></div></td>`;
    row.after(confirmRow);
    const input = confirmRow.querySelector("input");
    input.focus();
    confirmRow.querySelector("button").addEventListener("click", () => {
      if (!input.value.trim()) { showToast("Reason required", "Write why the status changes.", true); return; }
      setVehicleStatus(button.dataset.vehicle, status, Number(button.dataset.version), input.value.trim());
    });
  }));
}

// ------------------------------------------------------------ alerts

function renderAlerts() {
  const a = state.alerts || { escalations: [], events: [], callbacks: [], dead_letters: [] };
  badge("nav-alerts", a.escalations.length + a.dead_letters.length);
  document.getElementById("escalation-table").innerHTML = a.escalations.length ? a.escalations.map((t) => `<tr><td><a class="link" href="#" data-open-incident="${escapeHtml(t.incident_id)}"><code>${escapeHtml(shortId(t.task_id))}</code></a></td><td>${t.escalations.map((e) => `<span class="flag warn">${escapeHtml(e)}</span>`).join(" ")}</td><td>${escapeHtml(t.required_asset || "")}</td><td>${t.verify_failures}</td><td>${formatTime(t.escalated_at)}</td></tr>`).join("")
    : '<tr><td colspan="5" class="muted-cell">No escalated tasks.</td></tr>';
  document.getElementById("callback-list").innerHTML = a.callbacks.length ? a.callbacks.map((c) => `<div class="alert-row"><b><a class="link" href="#" data-open-incident="${escapeHtml(c.incident_id)}">${escapeHtml(shortId(c.incident_id))}</a> · ${escapeHtml(c.status)}</b><small>review priority ${escapeHtml(c.review_priority || "—")} · ${relativeTime(c.created_at)}</small></div>`).join("") : empty("No callbacks", "No open incident with UNKNOWN urgency.");
  const eventRow = (e) => {
    let payload = {};
    try { payload = JSON.parse(e.payload_json); } catch (_) { /* raw */ }
    const incident = payload.incident_id;
    const danger = ["INCIDENT_CRITICAL", "DROPOFF_UNRESOLVED"].includes(e.type);
    return `<div class="alert-row ${danger ? "danger" : ""}"><b>${escapeHtml(e.type)}${incident ? ` · <a class="link" href="#" data-open-incident="${escapeHtml(incident)}">${escapeHtml(shortId(incident))}</a>` : ""}</b><small>${payload.reason ? escapeHtml(payload.reason) + " · " : ""}${relativeTime(e.created_at)}</small></div>`;
  };
  document.getElementById("alert-events").innerHTML = a.events.length ? a.events.map(eventRow).join("") : empty("No alert events", "Critical incidents, escalations and unresolved drop-offs appear here.");
  document.getElementById("dead-letters").innerHTML = a.dead_letters.length ? a.dead_letters.map((d) => `<div class="alert-row danger"><b>${escapeHtml(d.fields?.reason || d.fields?.error || "dead letter")}</b><small>${escapeHtml(d.stream_id)} · ${escapeHtml(d.fields?.aggregate_id || d.fields?.original_id || "")}</small></div>`).join("") : empty("No dead letters", "Every intake message was processed.");
  const overview = [...a.escalations.map((t) => ({ text: `Task ${shortId(t.task_id)} escalated: ${t.escalations.join(", ")}`, incident: t.incident_id, danger: true })),
    ...a.callbacks.slice(0, 3).map((c) => ({ text: `Callback needed: ${shortId(c.incident_id)}`, incident: c.incident_id }))].slice(0, 6);
  document.getElementById("overview-alerts").innerHTML = overview.length ? overview.map((o) => `<div class="alert-row ${o.danger ? "danger" : ""}"><b><a class="link" href="#" data-open-incident="${escapeHtml(o.incident)}">${escapeHtml(o.text)}</a></b></div>`).join("") : empty("All clear", "No escalations or callbacks.");
}

// ------------------------------------------------------------ token & status

function renderToken() {
  const label = document.getElementById("token-state");
  if (token()) { label.textContent = "Token set for this tab · actions enabled"; label.className = "ok"; }
  else { label.textContent = "No token: read-only"; label.className = ""; }
}

function dot(kind, online, label) {
  const el = document.getElementById(`${kind}-dot`);
  el.className = online === true ? "online" : online === null ? "warn" : "";
  setText(`${kind}-label`, label);
}

function badge(id, count) {
  const el = document.getElementById(id);
  el.hidden = !count;
  el.textContent = count;
}

// ------------------------------------------------------------ helpers

function statusBadge(status) {
  const cls = { PENDING_TRIGGER: "needs_task", REVIEW_REQUIRED: "review", QUEUED: "queued", NEEDS_TASK: "needs_task", ASSIGNED: "assigned", COMPLETED: "completed", REJECTED: "rejected", MERGED: "rejected", CANCELLED: "rejected" }[status] || "progress";
  return `<span class="status ${cls}">${escapeHtml(String(status || "").replace(/_/g, " "))}</span>`;
}
function urgencyBadge(urgency) { return urgency ? `<span class="urgency ${escapeHtml(urgency)}">${escapeHtml(urgency)}</span>` : ""; }
function vehicleStatus(status) { const cls = { AVAILABLE: "completed", ASSIGNED: "assigned", HELD: "queued", OUT_OF_SERVICE: "review" }[status] || "rejected"; return `<span class="status ${cls}">${escapeHtml(status.replace(/_/g, " "))}</span>`; }
function shortAsset(type) { return { AMBULANCE: "amb", EVAC_TRUCK: "truck", RESCUE_BOAT: "boat" }[type] || type; }
function shortId(id) { const s = String(id || ""); const tail = s.split("/").pop(); return tail.length > 13 ? tail.slice(0, 8) + "…" : tail; }
function fact(label, content) { return `<div class="fact"><span>${escapeHtml(label)}</span><b>${value(content)}</b></div>`; }
function value(content) { return content === null || content === undefined || content === "" ? "—" : escapeHtml(String(content)); }
function number(content) { return new Intl.NumberFormat("en-IN").format(content || 0); }
function duration(seconds) { if (seconds == null || Number.isNaN(Number(seconds))) return "—"; const s = Math.round(Number(seconds)); return s < 60 ? `${s} s` : s < 3600 ? `${Math.floor(s / 60)} min ${s % 60} s` : `${Math.floor(s / 3600)} h ${Math.round((s % 3600) / 60)} min`; }
function numberOrNull(id) { const v = document.getElementById(id).value.trim(); return v === "" ? null : Number(v); }
function setText(id, content) { const node = document.getElementById(id); if (node) node.textContent = content; }
function empty(title, text) { return `<div class="empty-state"><h3>${escapeHtml(title)}</h3><p>${escapeHtml(text)}</p></div>`; }
function formatTime(input) { if (!input) return "—"; const date = new Date(input); return Number.isNaN(date.valueOf()) ? escapeHtml(String(input)) : new Intl.DateTimeFormat("en-IN", { dateStyle: "medium", timeStyle: "short", timeZone: "Asia/Kolkata" }).format(date); }
function relativeTime(input) { if (!input) return "—"; const seconds = Math.round((new Date(input).valueOf() - Date.now()) / 1000); const abs = Math.abs(seconds); const [amount, unit] = abs < 60 ? [seconds, "second"] : abs < 3600 ? [Math.round(seconds / 60), "minute"] : abs < 86400 ? [Math.round(seconds / 3600), "hour"] : [Math.round(seconds / 86400), "day"]; return new Intl.RelativeTimeFormat("en", { numeric: "auto" }).format(amount, unit); }
function escapeHtml(input) { return String(input ?? "").replace(/[&<>'"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" }[c])); }
function renderFailure(id, message) { const target = document.getElementById(id); if (target) target.innerHTML = `<div class="empty-state"><h3>Data unavailable</h3><p>${escapeHtml(message)}</p></div>`; }
function showToast(title, message, error = false) { const region = document.getElementById("toast-region"); const toast = document.createElement("div"); toast.className = `toast${error ? " error" : ""}`; toast.innerHTML = `<b>${escapeHtml(title)}</b><span>${escapeHtml(message)}</span>`; region.appendChild(toast); setTimeout(() => toast.remove(), 6500); }
function uuid() { if (window.crypto?.randomUUID) return crypto.randomUUID(); return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => { const r = (Math.random() * 16) | 0; return (c === "x" ? r : (r & 0x3) | 0x8).toString(16); }); }
function loadSession(key, fallback) { try { const raw = sessionStorage.getItem(key); return raw ? JSON.parse(raw) : fallback; } catch (_) { return fallback; } }
function saveSession(key, data) { try { sessionStorage.setItem(key, JSON.stringify(data)); } catch (_) { /* storage unavailable */ } }
