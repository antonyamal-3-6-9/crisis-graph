"use strict";

const API = "/api/v1";
const ALUVA = [10.10816, 76.35651];
const state = { incidents: [], shelters: [], hazards: null, reservations: [], waitingQueue: [], graph: null, maps: {}, layers: {}, selectedIncident: null, incidentRouteMap: null };

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
  await Promise.allSettled([loadHealth(), loadGraphSummary(), loadShelters(), loadHazards(), loadIncidents(), loadReservations(), loadWaitingQueue()]);
  invalidateVisibleMaps();
});

function navigateTo(path) {
  if (location.pathname !== path) {
    history.pushState(null, "", path);
  }
  configurePage();
  invalidateVisibleMaps();
}

function configurePage() {
  const path = pageMeta[location.pathname] ? location.pathname : "/";
  document.querySelectorAll(".view").forEach((el) => el.classList.toggle("active", el.dataset.view === path));
  document.querySelectorAll(".nav a").forEach((el) => el.classList.toggle("active", el.dataset.route === path));
  document.getElementById("page-eyebrow").textContent = pageMeta[path][0];
  document.getElementById("page-title").textContent = pageMeta[path][1];
}

function bindEvents() {
  window.addEventListener("popstate", () => {
    configurePage();
    invalidateVisibleMaps();
  });
  document.querySelectorAll(".nav a, .top-actions a[href], .brand").forEach((link) => {
    link.addEventListener("click", (e) => {
      const href = link.getAttribute("href");
      if (href && href.startsWith("/") && pageMeta[href]) {
        e.preventDefault();
        navigateTo(href);
      }
    });
  });
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

async function fetchJson(url, options = {}) {
  const method = options.method || "GET";
  const start = performance.now();
  console.groupCollapsed(`%c[API] %c${method} %c${url}`, "color:#3b82f6;font-weight:bold;", "color:#10b981;font-weight:bold;", "color:#6b7280;");
  if (options.body) {
    try { console.log("Request Body:", JSON.parse(options.body)); } catch (_) { console.log("Request Body:", options.body); }
  }
  try {
    const response = await fetch(url, options);
    const elapsed = (performance.now() - start).toFixed(1);
    if (!response.ok) {
      const errText = await response.text();
      console.error(`Status: ${response.status} ${response.statusText} (${elapsed}ms)`, errText);
      console.groupEnd();
      throw new Error(`${response.status} ${response.statusText}: ${errText}`);
    }
    const data = await response.json();
    console.log(`Response (${elapsed}ms):`, data);
    console.groupEnd();
    return data;
  } catch (err) {
    console.error("API Error:", err);
    console.groupEnd();
    throw err;
  }
}

async function fetchOptionalJson(url) {
  const start = performance.now();
  console.groupCollapsed(`%c[API Optional] %cGET %c${url}`, "color:#8b5cf6;font-weight:bold;", "color:#10b981;font-weight:bold;", "color:#6b7280;");
  try {
    const response = await fetch(url);
    const elapsed = (performance.now() - start).toFixed(1);
    if (response.status === 404) {
      console.log(`Status 404 (Not Found) in ${elapsed}ms`);
      console.groupEnd();
      return null;
    }
    if (!response.ok) {
      const errText = await response.text();
      console.error(`Status: ${response.status} ${response.statusText} (${elapsed}ms)`, errText);
      console.groupEnd();
      throw new Error(`${response.status} ${response.statusText}: ${errText}`);
    }
    const data = await response.json();
    console.log(`Response (${elapsed}ms):`, data);
    console.groupEnd();
    return data;
  } catch (err) {
    console.error("Optional fetch error:", err);
    console.groupEnd();
    throw err;
  }
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
    if (!state.selectedIncident && state.incidents.length > 0) {
      const reviewItem = state.incidents.find((i) => i.status === "REVIEW_REQUIRED");
      selectIncident((reviewItem || state.incidents[0]).incident_id);
    }
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
  target.innerHTML = incidents.length ? incidents.map((incident) => `<a class="feed-row" href="/incidents" data-incident-id="${escapeHtml(incident.incident_id)}"><b>${escapeHtml(incident.incident_id)}</b>${statusBadge(incident.status)}<p>${escapeHtml(incident.original_sos.raw_text)}</p></a>`).join("") : '<div class="empty-state"><h3>No persisted incidents</h3><p>Use Dispatch Lab to begin the demonstration.</p></div>';
  target.querySelectorAll("[data-incident-id]").forEach((link) => {
    link.addEventListener("click", (e) => {
      e.preventDefault();
      navigateTo("/incidents");
      selectIncident(link.dataset.incidentId);
    });
  });
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

function renderReviewCardHtml(prefix, incidentId, version, triage) {
  let suggestedJunction = triage?.resolved_junction_id || "";
  if (!suggestedJunction && triage?.victim_location_raw) {
    const loc = triage.victim_location_raw.toLowerCase();
    if (loc.includes("bank")) suggestedJunction = "node/4742583964";
    else if (loc.includes("railway") || loc.includes("station")) suggestedJunction = "node/4664235699";
    else if (loc.includes("manappuram") || loc.includes("temple")) suggestedJunction = "node/7048449098";
    else if (loc.includes("pump")) suggestedJunction = "node/7992454789";
    else if (loc.includes("town hall")) suggestedJunction = "node/1085737525";
    else if (loc.includes("hospital")) suggestedJunction = "node/343716109";
    else if (loc.includes("uc") || loc.includes("college")) suggestedJunction = "node/9903123560";
  }
  if (!suggestedJunction) suggestedJunction = "node/4742583964";

  const suggestedHeadcount = triage?.headcount || 4;
  const suggestedAsset = triage?.required_asset || "AMBULANCE";
  const suggestedNotes = triage?.uncertainty_reasons?.length
    ? "Resolved ambiguous report: verified victim count, vehicle class, and road access."
    : "Dispatcher verified and authorized dispatch.";

  return `
    <section class="review-action-card" id="${prefix}-card">
      <div class="review-action-head">
        <div>
          <p class="kicker">Human-in-the-loop governance</p>
          <h3>Dispatcher Review & Authorization</h3>
        </div>
        <span class="status review">Awaiting Dispatcher Decision</span>
      </div>
      <p class="supporting" style="margin: 4px 0 14px; font-size: 11px;">
        The candidate extraction flagged uncertainties. Automated dispatch was safely halted. Review candidate facts below to authorize allocation and routing:
      </p>
      <form id="${prefix}-form" onsubmit="return false;">
        <div class="form-row" style="margin-bottom: 8px;">
          <div>
            <label for="${prefix}-headcount">Headcount (Persons)</label>
            <input type="number" id="${prefix}-headcount" min="1" max="100" value="${suggestedHeadcount}" required />
          </div>
          <div>
            <label for="${prefix}-asset">Authorized Asset Class</label>
            <select id="${prefix}-asset" required>
              <option value="AMBULANCE" ${suggestedAsset === "AMBULANCE" ? "selected" : ""}>Ambulance (Road)</option>
              <option value="EVAC_TRUCK" ${suggestedAsset === "EVAC_TRUCK" ? "selected" : ""}>Evacuation Truck (Heavy/Flood)</option>
              <option value="RESCUE_BOAT" ${suggestedAsset === "RESCUE_BOAT" ? "selected" : ""}>Rescue Boat (Water)</option>
              <option value="HELICOPTER" ${suggestedAsset === "HELICOPTER" ? "selected" : ""}>Helicopter (Air)</option>
            </select>
          </div>
        </div>
        <div class="form-row" style="margin-bottom: 8px;">
          <div>
            <label for="${prefix}-junction">Resolved OSM Junction</label>
            <input type="text" id="${prefix}-junction" list="landmark-junctions" value="${escapeHtml(suggestedJunction)}" placeholder="e.g. node/4742583964" required />
            <datalist id="landmark-junctions">
              <option value="node/4742583964">Bank Junction (Aluva)</option>
              <option value="node/4664235699">Aluva Railway Station</option>
              <option value="node/7992454789">Pump Junction</option>
              <option value="node/7048449098">Manappuram Shiva Temple</option>
              <option value="node/1085737525">Aluva Town Hall</option>
              <option value="node/9903123560">UC College</option>
              <option value="node/343716109">Taluk Hospital</option>
            </datalist>
          </div>
          <div>
            <label for="${prefix}-callsign">Dispatcher Call-sign</label>
            <input type="text" id="${prefix}-callsign" value="dispatcher_station_1" required />
          </div>
        </div>
        <div style="margin-bottom: 8px;">
          <label for="${prefix}-notes">Audit Log / Justification</label>
          <input type="text" id="${prefix}-notes" value="${escapeHtml(suggestedNotes)}" placeholder="Enter reason for audit record..." required />
        </div>
        <div class="review-btn-row">
          <button type="button" class="button primary" id="${prefix}-approve-btn">
            <span>✓ Authorize & Dispatch</span>
          </button>
          <button type="button" class="button danger" id="${prefix}-reject-btn">
            <span>✕ Reject Incident</span>
          </button>
        </div>
      </form>
    </section>
  `;
}

function bindReviewCardActions(prefix, incidentId, version, onComplete) {
  const approveBtn = document.getElementById(`${prefix}-approve-btn`);
  const rejectBtn = document.getElementById(`${prefix}-reject-btn`);
  if (!approveBtn || !rejectBtn) return;

  const encodedId = encodeURIComponent(incidentId);

  approveBtn.addEventListener("click", async () => {
    const headcount = parseInt(document.getElementById(`${prefix}-headcount`)?.value, 10);
    const required_asset = document.getElementById(`${prefix}-asset`)?.value;
    const resolved_junction_id = document.getElementById(`${prefix}-junction`)?.value.trim();
    const reviewer_id = document.getElementById(`${prefix}-callsign`)?.value.trim() || "dispatcher_station_1";
    const reason = document.getElementById(`${prefix}-notes`)?.value.trim() || "Dispatcher verified and authorized dispatch";

    if (!headcount || isNaN(headcount) || headcount < 1) {
      showToast("Validation Error", "Please provide a valid headcount (>= 1).", true);
      return;
    }
    if (!resolved_junction_id) {
      showToast("Validation Error", "Please provide a valid victim junction ID.", true);
      return;
    }

    approveBtn.disabled = true;
    rejectBtn.disabled = true;
    approveBtn.innerHTML = '<i class="button-spinner"></i><span>Authorizing dispatch…</span>';

    try {
      const res = await fetchJson(`${API}/incidents/${encodedId}/review`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          decision: "APPROVE",
          expected_version: version,
          reason,
          reviewer_id,
          headcount,
          required_asset,
          resolved_junction_id,
        }),
      });

      showToast("Dispatch Authorized", `Incident ${incidentId} routed and verified!`);
      await Promise.allSettled([loadIncidents(), loadShelters(), loadReservations(), loadGraphSummary()]);
      if (onComplete) onComplete(res);
    } catch (err) {
      showToast("Authorization Failed", err.message, true);
      approveBtn.disabled = false;
      rejectBtn.disabled = false;
      approveBtn.innerHTML = '<span>✓ Authorize & Dispatch</span>';
    }
  });

  rejectBtn.addEventListener("click", async () => {
    const reviewer_id = document.getElementById(`${prefix}-callsign`)?.value.trim() || "dispatcher_station_1";
    const reason = document.getElementById(`${prefix}-notes`)?.value.trim() || "Rejected by dispatcher";

    approveBtn.disabled = true;
    rejectBtn.disabled = true;
    rejectBtn.innerHTML = '<i class="button-spinner"></i><span>Rejecting…</span>';

    try {
      const res = await fetchJson(`${API}/incidents/${encodedId}/review`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          decision: "REJECT",
          expected_version: version,
          reason,
          reviewer_id,
        }),
      });

      showToast("Incident Rejected", `Incident ${incidentId} marked as rejected.`);
      await Promise.allSettled([loadIncidents(), loadShelters(), loadReservations(), loadGraphSummary()]);
      if (onComplete) onComplete(res);
    } catch (err) {
      showToast("Rejection Failed", err.message, true);
      approveBtn.disabled = false;
      rejectBtn.disabled = false;
      rejectBtn.innerHTML = '<span>✕ Reject Incident</span>';
    }
  });
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
    const needsReview = incident.status === "REVIEW_REQUIRED";

    target.innerHTML = `<div class="detail-title"><div><code>${escapeHtml(incident.incident_id)}</code><p>Version ${incident.version} · ${escapeHtml(incident.original_sos.source_channel || "unknown source")} · ${formatTime(incident.created_at)}</p></div>${statusBadge(incident.status)}</div>
      <div class="raw-message">${escapeHtml(incident.original_sos.raw_text)}</div>
      <div class="facts">
        ${fact("Victim location", triage?.victim_location_raw)}${fact("Resolved junction", triage?.resolved_junction_id)}${fact("Headcount", triage?.headcount)}
        ${fact("Requested asset", triage?.required_asset)}${fact("Confidence label", triage?.confidence_score)}${fact("Human review", triage?.needs_human_review == null ? null : triage.needs_human_review ? "Required" : "Not required")}
      </div>
      ${reasons.length ? `<div class="notice warning detail-notice"><b>Uncertainty</b><span>${reasons.map(escapeHtml).join(" · ")}</span></div>` : ""}
      ${needsReview ? renderReviewCardHtml("incident-review", incident.incident_id, incident.version, triage) : ""}
      ${routeIsActive ? `
        <div class="mission-actions">
          <div>
            <b>Active Dispatch Allocation</b>
            <p>Vehicle and capacity currently locked for this incident. Release manually or allow 2.5m mission TTL sweeper to auto-recycle back to depot.</p>
          </div>
          <button class="button-release" onclick="releaseIncident('${escapeHtml(incident.incident_id)}')">Complete Mission & Release Fleet</button>
        </div>` : ""}
      ${route ? `<section class="route-record"><div class="route-record-head"><div><p class="kicker">Immutable route evidence</p><h3>${routeIsInvalidated ? "Invalidated" : routeIsActive ? "Verified" : "Historical"} route v${route.route_version}</h3></div><span class="tag ${routeIsInvalidated ? "danger" : routeIsActive ? "success" : "muted"}">${routeIsInvalidated ? "Do not use" : routeIsActive ? `${route.verified_segment_count} segments certified` : "Audit evidence"}</span></div>
        <div class="incident-route-map" id="incident-route-map"></div>
        <div class="facts route-facts">${fact("Origin", route.origin_shelter_name)}${fact("Assigned unit", route.assigned_asset_id)}${fact("Triage revision", `v${route.triage_revision}`)}${fact("Distance", `${Number(route.total_distance_km).toFixed(2)} km`)}${fact("Travel time", `${(Number(route.total_travel_time_s) / 60).toFixed(1)} min`)}${fact("Objective", route.cost_objective)}${fact("Verified at", formatTime(route.verified_at))}</div>
        <p class="route-proof"><code>${escapeHtml(route.route_id)}</code> · ${escapeHtml(route.dataset)} · ${escapeHtml(route.detour_reason || "Direct clear path")} · current incident state ${escapeHtml(incident.status)}</p>
      </section>` : persistedRoute?.projection_error ? `<div class="notice warning detail-notice"><b>Route projection unavailable</b><span>The incident record remains readable, but its stored junction path could not be projected onto the current baseline map.</span></div>` : !needsReview ? `<div class="notice detail-notice"><b>No persisted verified route</b><span>This incident stopped before route certification, or predates immutable route evidence storage.</span></div>` : ""}
      <div class="timeline"><h3>Immutable audit timeline</h3>${audit.length ? audit.map((event) => `<div class="timeline-row"><b>${escapeHtml(event.action)}</b><p>${escapeHtml(event.from_status)} → ${escapeHtml(event.to_status)} · ${escapeHtml(event.reason)}</p><small>v${event.previous_version} → v${event.new_version} · ${formatTime(event.occurred_at)} · ${escapeHtml(event.actor_id)}</small></div>`).join("") : '<p class="supporting">No audit events found.</p>'}</div>`;

    if (needsReview) {
      bindReviewCardActions("incident-review", incident.incident_id, incident.version, () => {
        selectIncident(incident.incident_id);
      });
    }

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
    style: (feature) => {
      if (feature?.properties?.stroke) {
        return {
          color: isInvalidated ? "#b13e36" : feature.properties.stroke,
          weight: 6,
          opacity: 0.9,
          dashArray: feature.properties.dash_style === "dashed" ? "8, 8" : (feature.properties.dash_style === "dotted" ? "3, 6" : null)
        };
      }
      return {
        color: isInvalidated ? "#b13e36" : isActive ? "#1f8a62" : "#647873",
        weight: 6,
        opacity: .9,
        dashArray: isActive ? null : "7 6"
      };
    },
    pointToLayer: (feature, latlng) => {
      if (feature?.properties?.type === "staging_hub") {
        return L.circleMarker(latlng, {
          radius: 11,
          color: "#ffffff",
          weight: 3,
          fillColor: "#06b6d4",
          fillOpacity: 1
        }).bindPopup(`<b>⚓ Staging Transfer Hub</b><br><small>${escapeHtml(feature.properties.name || "Boat Launch & Ambulance Handover")}</small><br>Junction: <code>${escapeHtml(feature.properties.junction_id)}</code>`);
      }
      if (feature?.properties?.type === "shelter" || feature?.properties?.role === "origin") {
        return L.circleMarker(latlng, {
          radius: 9,
          color: "#ffffff",
          weight: 3,
          fillColor: "#2563eb",
          fillOpacity: 1
        }).bindPopup(`<b>🚩 Base Depot (Origin)</b><br><b>${escapeHtml(feature.properties.name || "Relief Hub")}</b><br>Assigned Fleet: <code>${escapeHtml(feature.properties.asset || "Vehicle Unit")}</code>`);
      }
      return L.circleMarker(latlng, {
        radius: 9,
        color: "#ffffff",
        weight: 3,
        fillColor: "#dc2626",
        fillOpacity: 1
      }).bindPopup(`<b>📍 Distress Location (Destination)</b><br>Victim Junction: <code>${escapeHtml(feature.properties.junction_id || "")}</code>${feature.properties.headcount ? `<br>Headcount: <b>${feature.properties.headcount} Persons</b>` : ""}`);
    },
    onEachFeature: (feature, featureLayer) => {
      if (feature?.properties?.type === "route_leg") {
        featureLayer.bindPopup(`<b>${escapeHtml(feature.properties.mode_label || "Transit Leg")}</b><br>Distance: <b>${feature.properties.distance_km?.toFixed(2)} km</b><br>Est. Time: <b>${(feature.properties.travel_time_s / 60).toFixed(1)} min</b>${feature.properties.is_hazard ? "<br><span style='color:#dc2626;font-weight:600;'>⚠ Submerged Hazard Corridor</span>" : "<br><span style='color:#10b981;font-weight:600;'>✔ Certified Dry Road</span>"}`);
      }
    }
  }).addTo(map);

  const bounds = layer.getBounds();
  if (bounds.isValid()) map.fitBounds(bounds, { padding: [35, 35], maxZoom: 16 });
  state.incidentRouteMap = map;
  setTimeout(() => map.invalidateSize(), 30);
}

let dispatchTimer = null;

async function submitDispatch(event) {
  event.preventDefault();
  const input = document.getElementById("sos-text");
  const button = document.getElementById("dispatch-btn");
  const resultPanel = document.getElementById("dispatch-result");
  const rawText = input.value.trim();
  if (!rawText) return;

  button.disabled = true;
  button.innerHTML = '<i class="button-spinner"></i><span>Running safety pipeline…</span>';
  document.getElementById("dispatch-map-state").textContent = "Processing";

  const startTime = performance.now();
  if (resultPanel) {
    resultPanel.innerHTML = `
      <div class="processing-card">
        <div class="circular-progress">
          <svg viewBox="0 0 50 50">
            <circle class="bg" cx="25" cy="25" r="20" fill="none" stroke-width="4"></circle>
            <circle class="meter" cx="25" cy="25" r="20" fill="none" stroke-width="4"></circle>
          </svg>
        </div>
        <h3>Processing emergency dispatch</h3>
        <p class="processing-stage" id="stage-desc">Stage 1: Evaluating report via local Qwen3-4B-Instruct...</p>
        <div class="processing-timer" id="pipeline-timer">Elapsed: 0.0s</div>
      </div>
    `;
  }

  if (dispatchTimer) clearInterval(dispatchTimer);
  dispatchTimer = setInterval(() => {
    const elapsedSec = ((performance.now() - startTime) / 1000).toFixed(1);
    const timerEl = document.getElementById("pipeline-timer");
    const descEl = document.getElementById("stage-desc");
    if (timerEl) timerEl.textContent = `Elapsed: ${elapsedSec}s`;
    if (descEl) {
      if (elapsedSec < 2.5) {
        descEl.textContent = "Stage 1: Evaluating report via local Qwen3-4B-Instruct...";
      } else if (elapsedSec < 8.0) {
        descEl.textContent = "Stage 1: Generating typed triage JSON schema (4B CPU inference)...";
      } else {
        descEl.textContent = "Stage 2–4: Reserving resources with Redis lock & verifying road topology in Neo4j...";
      }
    }
  }, 100);

  try {
    const latVal = parseFloat(document.getElementById("gps-lat")?.value);
    const lonVal = parseFloat(document.getElementById("gps-lon")?.value);
    const result = await fetchJson(`${API}/dispatch`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        raw_text: rawText,
        source_channel: document.getElementById("channel-select").value,
        latitude: isNaN(latVal) ? null : latVal,
        longitude: isNaN(lonVal) ? null : lonVal,
      })
    });
    if (dispatchTimer) clearInterval(dispatchTimer);
    renderDispatchResult(result);
    await Promise.allSettled([loadIncidents(), loadShelters(), loadReservations(), loadGraphSummary()]);
  } catch (error) {
    if (dispatchTimer) clearInterval(dispatchTimer);
    showToast("Dispatch failed", error.message, true);
    if (resultPanel) {
      resultPanel.innerHTML = `<div class="empty-state"><h3>Pipeline unavailable</h3><p>${escapeHtml(error.message)}</p></div>`;
    }
  } finally {
    if (dispatchTimer) clearInterval(dispatchTimer);
    button.disabled = false;
    button.innerHTML = '<span>Run triage, allocation and routing</span>';
  }
}

function renderDispatchResult(data, fromStream = false) {
  const verified = data.status === "RoutedVerified" && data.incident_status === "ROUTE_VERIFIED";
  const triage = data.triage || {};
  const inference = triage.inference || {};
  const latencySec = inference.latency_ms ? (inference.latency_ms / 1000).toFixed(2) + "s" : null;
  const speed = inference.tokens_per_second ? inference.tokens_per_second.toFixed(1) + " tok/s" : null;
  const ttft = inference.prompt_eval_ms ? (inference.prompt_eval_ms / 1000).toFixed(2) + "s" : null;
  const tokens = inference.completion_tokens ? `${inference.completion_tokens} tokens` : null;
  const reviewRequired = data.review_required || data.incident_status === "REVIEW_REQUIRED";

  const target = document.getElementById("dispatch-result");
  if (target) {
    target.innerHTML = `<div class="result-head"><div><p class="kicker">${verified ? "Certified deterministic result" : "Fail-closed outcome"}</p><h2>${verified ? "Route verified for dispatch proposal" : "Human escalation required"}</h2><code>${escapeHtml(data.alert_id)} · incident v${data.incident_version}</code></div>${statusBadge(data.incident_status)}</div>
    <div class="result-metrics"><div><span>Distance</span><b>${data.distance_km ? data.distance_km.toFixed(2) + " km" : "Not computed"}</b></div><div><span>Travel time</span><b>${data.travel_time_s ? (data.travel_time_s / 60).toFixed(1) + " min" : "Not computed"}</b></div><div><span>Asset</span><b>${escapeHtml(data.assigned_asset || "Unassigned")}</b></div><div><span>Segments verified</span><b>${number(data.segment_count || 0)}</b></div></div>
    <div class="evidence-grid"><div class="evidence"><h3>AI candidate extraction</h3><dl><dt>Location</dt><dd>${value(triage.victim_location_raw)}</dd><dt>Headcount</dt><dd>${value(triage.headcount)}</dd><dt>Requested asset</dt><dd>${value(triage.required_asset)}</dd><dt>Confidence</dt><dd>${value(triage.confidence_score)}</dd><dt>Review flag</dt><dd>${data.review_required ? "Required" : "Clear"}</dd></dl></div><div class="evidence"><h3>Deterministic controls</h3><dl><dt>Persistence</dt><dd>${data.control_plane_persisted ? "Neo4j committed" : "Failed"}</dd><dt>Duplicate</dt><dd>${data.duplicate_suppressed ? "Suppressed" : "No"}</dd><dt>Junction</dt><dd>${value(data.victim_junction)}</dd><dt>Origin</dt><dd>${value(data.assigned_shelter?.name)}</dd><dt>Route</dt><dd>${verified ? "100% segments passed" : "Unavailable / rejected"}</dd></dl></div></div>
    <div class="inference-telemetry">
      <div><span>Inference Latency</span><b>${latencySec || "N/A"}</b></div>
      <div><span>Prompt Eval (TTFT)</span><b>${ttft || "N/A"}</b></div>
      <div><span>Generation Speed</span><b>${speed || "N/A"}</b></div>
      <div><span>Output Tokens</span><b>${tokens || "N/A"}</b></div>
    </div>
    ${(triage.uncertainty_reasons || []).length ? `<ul class="error-list">${triage.uncertainty_reasons.map((reason) => `<li>${escapeHtml(reason)}</li>`).join("")}</ul>` : ""}${data.errors?.length ? `<ul class="error-list">${data.errors.map((reason) => `<li>${escapeHtml(reason)}</li>`).join("")}</ul>` : ""}
    ${reviewRequired ? renderReviewCardHtml("dispatch-review", data.alert_id, data.incident_version, data.triage) : ""}`;

    if (reviewRequired) {
      bindReviewCardActions("dispatch-review", data.alert_id, data.incident_version, (res) => {
        renderDispatchResult(res);
      });
    }
  }
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
      style: (feature) => {
        if (feature?.properties?.stroke) {
          return {
            color: feature.properties.stroke,
            weight: 6,
            opacity: 0.9,
            dashArray: feature.properties.dash_style === "dashed" ? "8, 8" : (feature.properties.dash_style === "dotted" ? "3, 6" : null)
          };
        }
        return { color: verified ? "#1f8a62" : "#b57418", weight: 6, opacity: .9 };
      },
      pointToLayer: (feature, latlng) => {
        if (feature?.properties?.type === "staging_hub") {
          return L.circleMarker(latlng, {
            radius: 11,
            color: "#ffffff",
            weight: 3,
            fillColor: "#06b6d4",
            fillOpacity: 1
          }).bindPopup(`<b>⚓ Staging Transfer Hub</b><br><small>${escapeHtml(feature.properties.name || "Boat Launch & Ambulance Handover")}</small><br>Junction: <code>${escapeHtml(feature.properties.junction_id)}</code>`);
        }
        if (feature?.properties?.type === "shelter" || feature?.properties?.role === "origin") {
          return L.circleMarker(latlng, {
            radius: 9,
            color: "#ffffff",
            weight: 3,
            fillColor: "#2563eb",
            fillOpacity: 1
          }).bindPopup(`<b>🚩 Base Depot (Origin)</b><br><b>${escapeHtml(feature.properties.name || "Relief Hub")}</b><br>Assigned Fleet: <code>${escapeHtml(feature.properties.asset || "Vehicle Unit")}</code>`);
        }
        return L.circleMarker(latlng, {
          radius: 9,
          color: "#ffffff",
          weight: 3,
          fillColor: "#dc2626",
          fillOpacity: 1
        }).bindPopup(`<b>📍 Distress Location (Destination)</b><br>Victim Junction: <code>${escapeHtml(feature.properties.junction_id || "")}</code>${feature.properties.headcount ? `<br>Headcount: <b>${feature.properties.headcount} Persons</b>` : ""}${feature.properties.needed_asset ? `<br>Required Asset: <b>${escapeHtml(feature.properties.needed_asset)}</b>` : ""}`);
      },
      onEachFeature: (feature, featureLayer) => {
        if (feature?.properties?.type === "route_leg") {
          featureLayer.bindPopup(`<b>${escapeHtml(feature.properties.mode_label || "Transit Leg")}</b><br>Distance: ${feature.properties.distance_km?.toFixed(2)} km<br>Est. Time: ${Math.round(feature.properties.travel_time_s || 0)} s${feature.properties.is_hazard ? "<br><b style='color:#b13e36'>⚠ Navigating Submerged Corridor</b>" : ""}`);
        }
      }
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
    return `
      <article class="resource-card">
        <h3>${escapeHtml(hub.name)}</h3>
        <code>${escapeHtml(hub.id)}</code>
        <div class="capacity">
          <div class="capacity-head"><span>Occupancy</span><b>${hub.current_occupancy} / ${hub.capacity}</b></div>
          <div class="bar"><i style="width:${ratio}%"></i></div>
        </div>
        <div class="fleet">
          <div class="fleet-unit">
            <span>🚑 ${hub.ambulances_available}</span>
            <div class="fleet-ctrls">
              <button class="fleet-ctrl-btn" onclick="adjustShelterFleet('${escapeHtml(hub.id)}', 'ambulances', -1)" title="Remove Ambulance">-</button>
              <button class="fleet-ctrl-btn" onclick="adjustShelterFleet('${escapeHtml(hub.id)}', 'ambulances', 1)" title="Add Ambulance">+</button>
            </div>
          </div>
          <div class="fleet-unit">
            <span>🚚 ${hub.trucks_available}</span>
            <div class="fleet-ctrls">
              <button class="fleet-ctrl-btn" onclick="adjustShelterFleet('${escapeHtml(hub.id)}', 'trucks', -1)" title="Remove Truck">-</button>
              <button class="fleet-ctrl-btn" onclick="adjustShelterFleet('${escapeHtml(hub.id)}', 'trucks', 1)" title="Add Truck">+</button>
            </div>
          </div>
          <div class="fleet-unit">
            <span>🚤 ${hub.boats_available}</span>
            <div class="fleet-ctrls">
              <button class="fleet-ctrl-btn" onclick="adjustShelterFleet('${escapeHtml(hub.id)}', 'boats', -1)" title="Remove Boat">-</button>
              <button class="fleet-ctrl-btn" onclick="adjustShelterFleet('${escapeHtml(hub.id)}', 'boats', 1)" title="Add Boat">+</button>
            </div>
          </div>
        </div>
      </article>`;
  }).join("");
}

async function adjustShelterFleet(shelterId, assetKind, delta) {
  const payload = {};
  if (assetKind === "ambulances") payload.delta_ambulances = delta;
  if (assetKind === "trucks") payload.delta_trucks = delta;
  if (assetKind === "boats") payload.delta_boats = delta;

  try {
    await fetchJson(`${API}/shelters/${encodeURIComponent(shelterId)}/fleet`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    showToast("Fleet Updated", `${delta > 0 ? "Added" : "Removed"} ${assetKind} for ${shelterId}`);
    await Promise.allSettled([loadShelters(), loadWaitingQueue(), loadGraphSummary()]);
  } catch (error) {
    showToast("Fleet Update Failed", error.message, true);
  }
}

async function loadWaitingQueue() {
  try {
    state.waitingQueue = await fetchJson(`${API}/queue/waiting`);
    renderWaitingQueue();
  } catch (e) {
    console.error("Failed to load waiting queue", e);
  }
}

function renderWaitingQueue() {
  const table = document.getElementById("waiting-queue-table");
  const badge = document.getElementById("queue-count-badge");
  if (badge) {
    badge.textContent = `${state.waitingQueue.length} in queue`;
    badge.className = `tag ${state.waitingQueue.length > 0 ? "warning" : "muted"}`;
  }
  if (!table) return;

  if (!state.waitingQueue.length) {
    table.innerHTML = '<tr><td colspan="6" class="muted-cell">No incidents currently waiting in queue.</td></tr>';
    return;
  }

  table.innerHTML = state.waitingQueue.map((item) => `
    <tr>
      <td><b>#${item.position}</b></td>
      <td><code>${escapeHtml(item.incident_id)}</code></td>
      <td>${statusBadge(item.status)}</td>
      <td><b>${escapeHtml(item.requested_asset || "Any")}</b></td>
      <td>${item.headcount || "Default (4)"}</td>
      <td>${escapeHtml(item.location_raw || "Unknown")}</td>
    </tr>
  `).join("");
}

async function releaseIncident(incidentId) {
  try {
    await fetchJson(`${API}/incidents/${encodeURIComponent(incidentId)}/release`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ reason: "Dispatcher completed mission & released fleet" }),
    });
    showToast("Mission Completed", `Fleet released for incident ${incidentId}`);
    await Promise.allSettled([loadIncidents(), loadShelters(), loadReservations(), loadWaitingQueue(), loadGraphSummary()]);
    selectIncident(incidentId);
  } catch (error) {
    showToast("Release Failed", error.message, true);
  }
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
  await Promise.allSettled([loadShelters(), loadReservations(), loadWaitingQueue(), loadGraphSummary()]);
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
      await Promise.allSettled([loadIncidents(), loadShelters(), loadReservations(), loadWaitingQueue(), loadGraphSummary()]);
    } catch (error) { console.error("Invalid SSE dispatch event", error); }
  };
}

function currentTriage(incident) { return incident.triage_revisions?.at(-1)?.triage || null; }
function statusBadge(status) {
  const kind = status === "REVIEW_REQUIRED" || status === "RELEASED" ? "review" :
               status === "REJECTED" || status === "ROUTE_INVALIDATED" ? "danger" :
               status === "ROUTE_VERIFIED" || status === "RESERVED" || status === "COMPLETED" ? "verified" : "progress";
  return `<span class="status ${kind}">${escapeHtml(status || "UNKNOWN")}</span>`;
}
function fact(label, content) { return `<div class="fact"><span>${escapeHtml(label)}</span><b>${value(content)}</b></div>`; }
function value(content) { return content === null || content === undefined || content === "" ? "Not supplied" : escapeHtml(String(content)); }
function number(content) { return new Intl.NumberFormat("en-IN").format(content || 0); }
function setText(id, content) { const node = document.getElementById(id); if (node) node.textContent = content; }
function formatTime(input) { if (!input) return "—"; const date = new Date(input); return Number.isNaN(date.valueOf()) ? escapeHtml(String(input)) : new Intl.DateTimeFormat("en-IN", { dateStyle: "medium", timeStyle: "short", timeZone: "Asia/Kolkata" }).format(date); }
function relativeTime(input) { const seconds = Math.round((new Date(input).valueOf() - Date.now()) / 1000); const absolute = Math.abs(seconds); const [amount, unit] = absolute < 60 ? [seconds, "second"] : absolute < 3600 ? [Math.round(seconds / 60), "minute"] : absolute < 86400 ? [Math.round(seconds / 3600), "hour"] : [Math.round(seconds / 86400), "day"]; return new Intl.RelativeTimeFormat("en", { numeric: "auto" }).format(amount, unit); }
function escapeHtml(input) { return String(input ?? "").replace(/[&<>'"]/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" }[char])); }
function renderFailure(id, message) { const target = document.getElementById(id); if (target) target.innerHTML = `<div class="empty-state"><h3>Data unavailable</h3><p>${escapeHtml(message)}</p></div>`; }
function showToast(title, message, error = false) { const region = document.getElementById("toast-region"); const toast = document.createElement("div"); toast.className = `toast${error ? " error" : ""}`; toast.innerHTML = `<b>${escapeHtml(title)}</b><span>${escapeHtml(message)}</span>`; region.appendChild(toast); setTimeout(() => toast.remove(), 6500); }
