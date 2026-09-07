// CrisisGraph Emergency Dispatch Web Console

const ALUVA_CENTRE = [10.10816, 76.35651];
let map;
let sheltersLayer;
let hazardsLayer;
let routeLayer;
let currentBriefText = "";

document.addEventListener("DOMContentLoaded", () => {
  initMap();
  loadShelters();
  loadHazards();
});

// 1. Initialize Map
function initMap() {
  map = L.map("map", {
    zoomControl: true,
  }).setView(ALUVA_CENTRE, 13);

  // Layer 1: Tactical Dark (Watermark-free high-contrast OSM)
  const tacticalDark = L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
    attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
    className: "tactical-tiles",
    maxZoom: 19,
  });

  // Layer 2: Clean Standard OpenStreetMap
  const standardOsm = L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
    attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
    maxZoom: 19,
  });

  // Layer 3: High-Resolution Satellite (Esri World Imagery)
  const satellite = L.tileLayer("https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}", {
    attribution: 'Tiles &copy; Esri, Earthstar Geographics',
    maxZoom: 18,
  });

  // Add default layer
  tacticalDark.addTo(map);

  // Layer switcher control
  const baseMaps = {
    "Tactical Dark": tacticalDark,
    "Satellite Imagery": satellite,
    "Clean Street Map": standardOsm,
  };
  L.control.layers(baseMaps, null, { position: "topright" }).addTo(map);

  // 10 km Pilot Boundary Ring
  L.circle(ALUVA_CENTRE, {
    radius: 10000,
    color: "#3b82f6",
    weight: 1.5,
    opacity: 0.6,
    fillColor: "#3b82f6",
    fillOpacity: 0.02,
    dashArray: "6, 8",
  }).addTo(map);

  // Layer groups
  sheltersLayer = L.layerGroup().addTo(map);
  hazardsLayer = L.layerGroup().addTo(map);
  routeLayer = L.layerGroup().addTo(map);
}

// 2. Fetch and render shelters
async function loadShelters() {
  try {
    const res = await fetch("/api/v1/shelters");
    if (!res.ok) throw new Error("Failed to load shelters");
    const shelters = await res.json();

    sheltersLayer.clearLayers();
    const container = document.getElementById("shelters-list");
    container.innerHTML = "";

    shelters.forEach((s) => {
      // Map Marker
      const iconHtml = getShelterIcon(s.id);
      const customIcon = L.divIcon({
        className: "custom-shelter-icon",
        html: `<div style="background:#1e293b; border:2px solid #3b82f6; border-radius:50%; width:32px; height:32px; display:flex; align-items:center; justify-content:center; box-shadow:0 0 10px rgba(59,130,246,0.6); font-size:16px;">${iconHtml}</div>`,
        iconSize: [32, 32],
        iconAnchor: [16, 16],
      });

      const marker = L.marker([s.lat, s.lon], { icon: customIcon }).addTo(sheltersLayer);
      marker.bindPopup(`
        <div style="font-family:inherit; color:#0f172a; padding:4px;">
          <strong style="font-size:13px;">${s.name}</strong><br/>
          <small style="color:#64748b;">ID: ${s.id}</small>
          <hr style="margin:6px 0; border:0; border-top:1px solid #e2e8f0;"/>
          <div>Occupancy: <strong>${s.current_occupancy} / ${s.capacity}</strong></div>
          <div style="margin-top:4px; font-size:11px; color:#2563eb;">
            🚑 Ambulances: ${s.ambulances_available} | 🚚 Trucks: ${s.trucks_available} | 🚤 Boats: ${s.boats_available}
          </div>
        </div>
      `);

      // Sidebar Card
      const card = document.createElement("div");
      card.className = "shelter-card";
      card.innerHTML = `
        <div class="shelter-header">
          <span class="shelter-name">${s.name}</span>
          <span class="shelter-occ">${s.current_occupancy}/${s.capacity}</span>
        </div>
        <div class="shelter-assets">
          ${s.ambulances_available > 0 ? `<span class="asset-badge">🚑 ${s.ambulances_available} Amb</span>` : ""}
          ${s.trucks_available > 0 ? `<span class="asset-badge">🚚 ${s.trucks_available} Trk</span>` : ""}
          ${s.boats_available > 0 ? `<span class="asset-badge">🚤 ${s.boats_available} Boat</span>` : ""}
        </div>
      `;
      container.appendChild(card);
    });
  } catch (err) {
    console.error("Error loading shelters:", err);
  }
}

function getShelterIcon(id) {
  if (id.includes("HOSPITAL")) return "🏥";
  if (id.includes("MANAPPURAM")) return "🚤";
  if (id.includes("UC_COLLEGE")) return "🏫";
  return "🏛️";
}

// 3. Fetch and render active hazards
async function loadHazards() {
  try {
    const res = await fetch("/api/v1/hazards");
    if (!res.ok) throw new Error("Failed to load hazards");
    const geojson = await res.json();

    hazardsLayer.clearLayers();
    const count = geojson.features ? geojson.features.length : 0;
    document.getElementById("hazard-status").innerText = `${count} Active Road Closure(s)`;

    if (count > 0) {
      L.geoJSON(geojson, {
        style: {
          color: "#ef4444",
          weight: 6,
          opacity: 0.85,
          dashArray: "4, 6",
        },
        onEachFeature: (feature, layer) => {
          layer.bindPopup(`
            <div style="color:#0f172a; padding:4px;">
              <strong style="color:#dc2626;">HAZARD: ${feature.properties.status}</strong><br/>
              Road: ${feature.properties.road_name}<br/>
              <small>Segment: ${feature.properties.segment_id}</small>
            </div>
          `);
        },
      }).addTo(hazardsLayer);
    }
  } catch (err) {
    console.error("Error loading hazards:", err);
  }
}

// 4. Submit SOS Alert to Dispatch Engine
async function submitDispatch(e) {
  if (e) e.preventDefault();
  const textInput = document.getElementById("sos-text");
  const channelSelect = document.getElementById("channel-select");
  const btn = document.getElementById("dispatch-btn");
  const btnText = btn.querySelector(".btn-text");
  const btnLoader = btn.querySelector(".btn-loader");

  const raw_text = textInput.value.trim();
  if (!raw_text) return;

  btn.disabled = true;
  btnText.innerText = "RUNNING SOLVER & VERIFIER...";

  try {
    const res = await fetch("/api/v1/dispatch", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        raw_text,
        source_channel: channelSelect.value,
      }),
    });

    if (!res.ok) throw new Error(`HTTP error ${res.status}`);
    const data = await res.json();
    renderDispatchResult(data);
    loadShelters(); // refresh occupancy counts
  } catch (err) {
    alert("Error processing dispatch: " + err.message);
  } finally {
    btn.disabled = false;
    btnText.innerText = "RUNNING NEURO-SYMBOLIC DISPATCH";
  }
}

// 5. Render Dispatch Result on Map & Overlay
function renderDispatchResult(data) {
  routeLayer.clearLayers();

  const isVerified = data.status === "RoutedVerified";
  const statusBadge = document.getElementById("brief-status-badge");
  statusBadge.className = isVerified ? "badge verified" : "badge escalate";
  statusBadge.innerText = isVerified ? "ROUTED & VERIFIED" : "ESCALATE TO DISPATCHER";

  document.getElementById("brief-incident-id").innerText = data.alert_id;
  document.getElementById("metric-dist").innerText = `${data.distance_km.toFixed(2)} km`;
  document.getElementById("metric-time").innerText = data.travel_time_s > 0 ? `${(data.travel_time_s / 60).toFixed(1)} min` : "N/A";
  document.getElementById("metric-asset").innerText = data.assigned_asset || "None";
  document.getElementById("metric-shelter").innerText = data.assigned_shelter ? data.assigned_shelter.name.split(" ")[0] : "None";

  currentBriefText = data.tactical_brief;
  document.getElementById("brief-content").innerText = data.tactical_brief;
  document.getElementById("brief-panel").style.display = "flex";

  // Render GeoJSON Route & Pins
  if (data.geojson && data.geojson.features && data.geojson.features.length > 0) {
    let bounds = L.latLngBounds();

    data.geojson.features.forEach((feat) => {
      if (feat.geometry.type === "LineString") {
        // Reverse coordinates from GeoJSON [lon, lat] to Leaflet [lat, lon]
        const latlngs = feat.geometry.coordinates.map((c) => [c[1], c[0]]);
        const polyline = L.polyline(latlngs, {
          color: isVerified ? "#10b981" : "#f59e0b",
          weight: 6,
          opacity: 0.9,
          lineCap: "round",
          lineJoin: "round",
        }).addTo(routeLayer);
        bounds.extend(polyline.getBounds());
      } else if (feat.geometry.type === "Point") {
        const lat = feat.geometry.coordinates[1];
        const lon = feat.geometry.coordinates[0];
        bounds.extend([lat, lon]);

        const isVictim = feat.properties.type === "victim";
        const marker = L.circleMarker([lat, lon], {
          radius: isVictim ? 9 : 8,
          fillColor: isVictim ? "#ef4444" : "#3b82f6",
          color: "#fff",
          weight: 2,
          opacity: 1,
          fillOpacity: 0.9,
        }).addTo(routeLayer);

        marker.bindPopup(`
          <div style="color:#0f172a;">
            <strong>${isVictim ? "🚨 VICTIM LOCATION" : "🏥 ORIGIN FACILITY"}</strong><br/>
            ${isVictim ? `Headcount: ${feat.properties.headcount} | Asset: ${feat.properties.needed_asset}` : `Asset: ${feat.properties.asset || "N/A"}`}
          </div>
        `);
      }
    });

    if (bounds.isValid()) {
      map.fitBounds(bounds, { padding: [60, 60] });
    }
  }
}

// 6. Quick Scenario Presets
function loadScenario(type) {
  const textInput = document.getElementById("sos-text");
  if (type === "pump_flood") {
    textInput.value = "URGENT: Flash flood at (lat: 10.1135, lon: 76.3540) near Pump Junction. 4 persons trapped, elderly patient needs ambulance immediately!";
  } else if (type === "railway_injured") {
    textInput.value = "Aluva Railway Station has 3 injured passengers, need ambulance dispatch to hospital.";
  } else if (type === "manappuram_boat") {
    textInput.value = "Water entering Aluva Manappuram temple grounds, 12 pilgrims stranded on temple steps. Urgent boat rescue needed!";
  } else if (type === "railway_truck") {
    textInput.value = "Send heavy evacuation truck to Aluva Railway Station for 10 passengers.";
  }
  submitDispatch();
}

// 7. Dynamic Road Hazard Simulation
async function simulateFlood() {
  // Flood segment on UC College route: way/1080426401/seg/0/rev
  const segment_id = "way/1080426401/seg/0/rev";
  try {
    const res = await fetch("/api/v1/hazards", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        segment_id,
        status: "FLOODED",
        duration_hours: 4,
      }),
    });
    if (!res.ok) throw new Error("Failed to flood segment");
    await loadHazards();
    alert("Segment way/1080426401/seg/0/rev marked FLOODED in Neo4j!\nNow click 'Pump Junction Flood' to observe automatic detour routing!");
  } catch (err) {
    alert("Error applying hazard: " + err.message);
  }
}

async function clearHazards() {
  try {
    const res = await fetch("/api/v1/hazards/clear", { method: "POST" });
    if (!res.ok) throw new Error("Failed to clear hazards");
    await loadHazards();
    alert("All active operational hazards cleared from Neo4j.");
  } catch (err) {
    alert("Error clearing hazards: " + err.message);
  }
}

async function resetShelters() {
  try {
    const res = await fetch("/api/v1/shelters/reset", { method: "POST" });
    if (!res.ok) throw new Error("Failed to reset shelters");
    await loadShelters();
    alert("All 4 Aluva shelters reset to initial baseline capacity and fleet assets.");
  } catch (err) {
    alert("Error resetting shelters: " + err.message);
  }
}

function closeBrief() {
  document.getElementById("brief-panel").style.display = "none";
}

function copyBriefText() {
  if (currentBriefText) {
    navigator.clipboard.writeText(currentBriefText).then(() => {
      alert("Tactical Brief copied to clipboard!");
    });
  }
}
