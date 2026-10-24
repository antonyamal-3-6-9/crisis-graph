use crate::db::{RedisClient, StateManager, TraversalProfile};
use crate::ingestion::{SpatialResolver, TriageExtractor};
use crate::models::{CrisisState, DispatchStatus, ReservationRole, TacticalBrief};
use crate::solver::pathfinder::TransportMode;
use crate::solver::{DeterministicPathfinder, RoutingQuery};
use chrono::Utc;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct PipelineStages {
    extractor: TriageExtractor,
    state_mgr: StateManager,
    redis: RedisClient,
}

impl PipelineStages {
    pub fn new(extractor: TriageExtractor, state_mgr: StateManager, redis: RedisClient) -> Self {
        Self {
            extractor,
            state_mgr,
            redis,
        }
    }

    /// Stage 1: AI Listener (Triage & Hazard Extraction)
    pub async fn stage_1_triage_extraction(&self, mut state: CrisisState) -> CrisisState {
        info!("--- [Stage 1: AI Listener] Extracting SOS Triage Data ---");

        match self.extractor.extract(&state.sos).await {
            Ok(triage) => {
                let mut triage = triage;

                // Attempt GPS coordinate snapping or lexical landmark resolution if junction is not yet resolved
                if triage.resolved_junction_id.is_none() {
                    // 1. Check for structured device telemetry or embedded GPS coordinates
                    let coords = match (state.sos.latitude, state.sos.longitude) {
                        (Some(lat), Some(lon)) => {
                            info!("Device telemetry coordinates ({lat}, {lon}, accuracy: {:?}m) provided with SOS.", state.sos.accuracy_m);
                            Some((lat, lon, state.sos.accuracy_m.unwrap_or(1000.0).max(50.0)))
                        }
                        _ => SpatialResolver::parse_coordinates(&state.sos.raw_text)
                            .map(|(lat, lon)| (lat, lon, 1000.0)),
                    };

                    if let Some((lat, lon, search_radius)) = coords {
                        info!("Snapping coordinates ({lat}, {lon}, radius: {search_radius:.0}m) to nearest junction...");
                        match self
                            .state_mgr
                            .find_nearest_junction("aluva-periyar-pilot", lat, lon, search_radius)
                            .await
                        {
                            Ok(Some((junction_id, dist))) => {
                                info!("Snapped coordinates ({lat}, {lon}) -> {junction_id} ({dist:.1} m away)");
                                triage.resolved_junction_id = Some(junction_id);
                            }
                            Ok(None) => {
                                warn!("No road junction found within {search_radius:.0}m of ({lat}, {lon})")
                            }
                            Err(e) => warn!("Spatial snapper query error: {e}"),
                        }
                    }

                    // 2. Check for lexical landmark matching in raw location or message body
                    if triage.resolved_junction_id.is_none() {
                        let resolver = SpatialResolver::new();
                        if let Some(location) = triage.victim_location_raw.as_deref() {
                            if let Some(junction_id) = resolver.resolve(location) {
                                info!("Resolved location '{}' -> {}", location, junction_id);
                                triage.resolved_junction_id = Some(junction_id);
                            }
                        }
                        if triage.resolved_junction_id.is_none() {
                            if let Some(junction_id) = resolver.resolve(&state.sos.raw_text) {
                                info!("Resolved SOS message landmark -> {}", junction_id);
                                triage.resolved_junction_id = Some(junction_id);
                            }
                        }
                    }
                }

                info!(
                    "Extracted candidate: victim_junction={:?}, headcount={:?}, asset={:?}, hazards={}, review={}, latency_ms={}",
                    triage.resolved_junction_id, triage.headcount, triage.required_asset,
                    triage.hazards.len(), triage.needs_human_review, triage.inference.latency_ms
                );

                // LLM/citizen extractions are candidate reports only. They must not mutate
                // the authoritative operational overlay without dispatcher/field verification.
                for hazard in &triage.hazards {
                    info!("Recorded candidate hazard extraction for verification: segment={:?}, status={:?}, duration_hours={:?}", hazard.road_segment, hazard.status, hazard.duration_hours);
                }

                // Execute Uncertainty Waterfall Resolution
                let waterfall = crate::models::waterfall::UncertaintyWaterfall::resolve(&state.sos, &triage);
                let location_missing = waterfall.location_missing;
                state.uncertainty_waterfall = Some(waterfall.clone());

                if location_missing {
                    state.errors.push(format!(
                        "TriageLocationResolutionFailed: {:?}",
                        triage.victim_location_raw
                    ));
                    if let Some(ref dial_alert) = waterfall.dispatcher_dial_alert {
                        warn!("{}", dial_alert);
                    }
                    state.errors.push("TriageRequiresHumanReview: Victim location was not provided".to_string());
                } else {
                    // Location is known. If the only uncertainties are missing headcount or asset,
                    // the Uncertainty Waterfall provides a safe, standard operational baseline.
                    let non_blocking_reasons = ["Headcount was not provided", "Required asset was not specified"];
                    let remaining_blocking_reasons: Vec<String> = triage
                        .uncertainty_reasons
                        .iter()
                        .filter(|r| !non_blocking_reasons.contains(&r.as_str()))
                        .cloned()
                        .collect();

                    if !remaining_blocking_reasons.is_empty() {
                        state.errors.push(format!(
                            "TriageRequiresHumanReview: {}",
                            remaining_blocking_reasons.join("; ")
                        ));
                    }
                }

                state.triage = Some(triage);
            }
            Err(e) => {
                error!("Stage 1 Triage extraction failed: {}", e);
                state.errors.push(format!("TriageExtractionError: {}", e));
            }
        }

        state
    }

    /// Stage 2: Database Clerk (Asset & Shelter Allocation with Redis Mutex)
    pub async fn stage_2_asset_allocation(&self, mut state: CrisisState) -> CrisisState {
        info!("--- [Stage 2: Database Clerk] Reserving Shelter & Assets ---");

        if state.has_errors() {
            info!("Skipping resource allocation because triage did not pass validation");
            return state;
        }

        let triage = match &state.triage {
            Some(t) => t,
            None => {
                state
                    .errors
                    .push("Cannot allocate assets: missing triage report".to_string());
                return state;
            }
        };

        // Resolve headcount and required asset (using extracted values or Uncertainty Waterfall fallbacks)
        let (headcount, required_asset) = if let Some(ref wf) = state.uncertainty_waterfall {
            (wf.resolved_headcount, wf.resolved_asset.clone())
        } else {
            let hc = match triage.headcount {
                Some(value) => value,
                None => {
                    state
                        .errors
                        .push("Cannot allocate assets: missing triage headcount".to_string());
                    return state;
                }
            };
            let asset = match triage.required_asset.as_ref() {
                Some(value) => value.clone(),
                None => {
                    state
                        .errors
                        .push("Cannot allocate assets: missing required asset".to_string());
                    return state;
                }
            };
            (hc, asset)
        };

        // Query candidate shelters
        match self
            .state_mgr
            .find_candidate_shelters(&required_asset, headcount)
            .await
        {
            Ok(candidates) if !candidates.is_empty() => {
                let mut allocated = false;

                for shelter in candidates {
                    let lock_token = Uuid::new_v4().to_string();

                    // Acquire distributed lock on the shelter
                    match self.redis.acquire_lock(&shelter.id, &lock_token, 10).await {
                        Ok(true) => {
                            info!("Acquired Redis lock for shelter: {}", shelter.id);

                            // Execute atomic Cypher capacity reservation
                            match self
                                .state_mgr
                                .reserve_shelter_asset(
                                    &shelter.id,
                                    &required_asset,
                                    headcount,
                                    &state.sos.alert_id,
                                    ReservationRole::RoadPrimary,
                                )
                                .await
                            {
                                Ok(Some(reserved_shelter)) => {
                                    info!(
                                        "Allocated shelter {} (Junction: {}). Remaining occupancy: {}/{}",
                                        reserved_shelter.name, reserved_shelter.junction_id,
                                        reserved_shelter.current_occupancy, reserved_shelter.capacity
                                    );
                                    state.assigned_shelter = Some(reserved_shelter);
                                    state.assigned_asset =
                                        Some(format!("{}_UNIT_1", required_asset));
                                    allocated = true;
                                }
                                Ok(None) => {
                                    warn!(
                                        "Shelter {} capacity full during atomic commit",
                                        shelter.id
                                    );
                                }
                                Err(e) => {
                                    warn!("Shelter reservation error: {}", e);
                                }
                            }

                            // Release distributed lock
                            let _ = self.redis.release_lock(&shelter.id, &lock_token).await;

                            if allocated {
                                let _ = self.redis.remove_waiting_incident(&state.sos.alert_id).await;
                                break;
                            }
                        }
                        Ok(false) => {
                            warn!(
                                "Shelter {} is currently locked by another concurrent worker",
                                shelter.id
                            );
                        }
                        Err(e) => {
                            warn!("Redis lock error for {}: {}", shelter.id, e);
                        }
                    }
                }

                if !allocated {
                    let _ = self.redis.push_waiting_incident(&state.sos.alert_id).await;
                    state
                        .errors
                        .push("Failed to atomically reserve any eligible shelter (Enqueued in FIFO waiting queue)".to_string());
                }
            }
            Ok(_) => {
                let _ = self.redis.push_waiting_incident(&state.sos.alert_id).await;
                state
                    .errors
                    .push("No shelters have sufficient capacity or required assets (Enqueued in FIFO waiting queue)".to_string());
            }
            Err(e) => {
                state.errors.push(format!("ShelterQueryError: {}", e));
            }
        }

        state
    }

    /// Compensate an incident-owned shelter/asset reservation after a later
    /// control-plane, routing, or verification failure. The database operation
    /// is idempotent, so retries cannot increment fleet counts twice.
    pub async fn compensate_resource_allocation(
        &self,
        mut state: CrisisState,
        reason: &str,
    ) -> CrisisState {
        if state.assigned_shelter.is_none() && state.assigned_asset.is_none() {
            return state;
        }

        match self
            .state_mgr
            .release_incident_reservation(&state.sos.alert_id, reason)
            .await
        {
            Ok(true) => {
                info!(
                    incident_id = %state.sos.alert_id,
                    "Released incident resource reservation"
                );
                state.assigned_shelter = None;
                state.assigned_asset = None;
            }
            Ok(false) => state.errors.push(
                "ReservationCompensationError: no active incident reservation was found"
                    .to_string(),
            ),
            Err(error) => state
                .errors
                .push(format!("ReservationCompensationError: {error}")),
        }
        state
    }

    /// Stage 3: Route Calculator (Deterministic Petgraph Pathfinder)
    pub async fn stage_3_deterministic_pathfinder(&self, mut state: CrisisState) -> CrisisState {
        info!("--- [Stage 3: Route Calculator] Computing Shortest Path via Petgraph ---");

        if state.has_errors() {
            info!("Skipping route calculation because an earlier stage failed closed");
            return state;
        }

        let triage = match &state.triage {
            Some(t) => t,
            None => {
                state
                    .errors
                    .push("Missing triage data for path calculation".to_string());
                return state;
            }
        };

        let target_junction = match &triage.resolved_junction_id {
            Some(j) => j,
            None => {
                state.errors.push(format!(
                    "Cannot resolve victim junction for location {:?}",
                    triage.victim_location_raw
                ));
                return state;
            }
        };

        let required_asset = if let Some(ref wf) = state.uncertainty_waterfall {
            wf.resolved_asset.clone()
        } else {
            match triage.required_asset.clone() {
                Some(value) => value,
                None => {
                    state
                        .errors
                        .push("Missing required asset for path calculation".to_string());
                    return state;
                }
            }
        };

        let start_junction = match &state.assigned_shelter {
            Some(s) => &s.junction_id,
            None => {
                state
                    .errors
                    .push("Cannot compute path: no assigned shelter junction".to_string());
                return state;
            }
        };

        let is_directed_pilot =
            start_junction.starts_with("node/") || target_junction.starts_with("node/");

        if is_directed_pilot {
            // Query directed topology and active hazard segments from Neo4j (Aluva pilot dataset)
            let (edges, blocked_segments) = match self
                .state_mgr
                .get_directed_topology_with_hazards("aluva-periyar-pilot")
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    state.errors.push(format!(
                        "Failed to retrieve directed graph topology from Neo4j: {}",
                        e
                    ));
                    return state;
                }
            };

            let query = RoutingQuery::new(start_junction, target_junction)
                .with_asset_type(required_asset)
                .with_blocked_segments(blocked_segments);

            match DeterministicPathfinder::find_multimodal_route(&edges, &query) {
                Ok(plan) => {
                    let all_waypoints: Vec<String> = plan
                        .legs
                        .iter()
                        .flat_map(|leg| leg.waypoints.clone())
                        .collect();
                    let all_segments: Vec<String> = plan
                        .legs
                        .iter()
                        .flat_map(|leg| leg.segment_ids.clone())
                        .collect();

                    info!(
                        "Multi-modal route computed: {} legs, {} waypoints, {} segments, {:.2} km, {:.1} s, mitigation_required={}",
                        plan.legs.len(),
                        all_waypoints.len(),
                        all_segments.len(),
                        plan.total_distance_km,
                        plan.total_travel_time_s,
                        plan.requires_mitigation
                    );

                    state.computed_path = Some(all_waypoints);
                    state.segment_path = Some(all_segments);
                    state.total_distance_km = Some(plan.total_distance_km);
                    state.total_travel_time_s = Some(plan.total_travel_time_s);
                    state.detour_reason = Some(plan.recommendation_summary.clone());
                    state.multimodal_plan = Some(plan);
                }
                Err(e) => {
                    error!("Multi-modal routing failed: {}", e);
                    state.errors.push(format!("RoutingError: {}", e));
                }
            }
        } else {
            // Backward-compatible fallback for prototype toy graph (J1-J8)
            let passable_edges = match self.state_mgr.get_passable_subgraph().await {
                Ok(edges) => edges,
                Err(e) => {
                    state.errors.push(format!(
                        "Failed to retrieve passable subgraph from Neo4j: {}",
                        e
                    ));
                    return state;
                }
            };

            match DeterministicPathfinder::find_shortest_path(
                &passable_edges,
                start_junction,
                target_junction,
            ) {
                Ok(route) => {
                    info!(
                        "Path computed successfully: {:?} ({} km)",
                        route.path, route.total_distance_km
                    );
                    state.computed_path = Some(route.path);
                    state.total_distance_km = Some(route.total_distance_km);
                    state.detour_reason = route.detour_reason;
                }
                Err(e) => {
                    error!("Routing failed: {}", e);
                    state.errors.push(format!("RoutingError: {}", e));
                }
            }
        }

        state
    }

    /// Stage 4: Safety Verifier & Tactical Brief Generator
    pub async fn stage_4_verifier_and_brief(&self, mut state: CrisisState) -> CrisisState {
        info!("--- [Stage 4: Safety Verifier] Guardrail Assertions & Tactical Brief ---");

        let incident_id = state.sos.alert_id.clone();
        let target_junction = state
            .triage
            .as_ref()
            .and_then(|t| t.resolved_junction_id.clone())
            .unwrap_or_else(|| "UNKNOWN".to_string());
        let shelter = state.assigned_shelter.clone();
        let headcount = state
            .uncertainty_waterfall
            .as_ref()
            .map(|wf| wf.resolved_headcount)
            .or_else(|| state.triage.as_ref().and_then(|t| t.headcount))
            .unwrap_or(0);
        let path = state.computed_path.clone().unwrap_or_default();
        let distance = state.total_distance_km.unwrap_or(0.0);
        let asset = state
            .assigned_asset
            .clone()
            .unwrap_or_else(|| "UNASSIGNED".to_string());

        let mut verified = true;
        let mut verification_notes = Vec::new();

        // Assertion 1: Check path existence and contiguity
        if path.is_empty() {
            verified = false;
            verification_notes
                .push("FAILURE: No route could be generated to victim junction.".to_string());
        } else {
            if let Some(ref s) = shelter {
                if path.first() != Some(&s.junction_id) {
                    verified = false;
                    verification_notes.push(
                        "FAILURE: Route does not start at assigned shelter junction.".to_string(),
                    );
                }
            }
            if path.last() != Some(&target_junction) {
                verified = false;
                verification_notes.push(
                    "FAILURE: Route does not terminate at target victim junction.".to_string(),
                );
            }
        }

        // Assertion 2: Check resource assignment
        if shelter.is_none() || state.assigned_asset.is_none() {
            verified = false;
            verification_notes.push("FAILURE: Resource allocation incomplete.".to_string());
        }

        // Assertion 3: Independent live safety verification of every land leg, using the
        // access profile of whoever traverses it. Water legs cannot be verified from
        // graph state and are reported as such, never as confirmed.
        let requires_mitigation = state
            .multimodal_plan
            .as_ref()
            .map(|p| p.requires_mitigation)
            .unwrap_or(false);

        let mut land_checks: Vec<(String, Vec<String>, TraversalProfile)> = Vec::new();
        let mut unverified_water_km = 0.0;
        if let Some(ref plan) = state.multimodal_plan {
            for leg in &plan.legs {
                match leg.mode {
                    TransportMode::RoadVehicle => land_checks.push((
                        format!("Road leg {}", leg.leg_index),
                        leg.segment_ids.clone(),
                        TraversalProfile::Vehicle(plan.primary_asset.clone()),
                    )),
                    TransportMode::FootWading => land_checks.push((
                        format!("Foot leg {}", leg.leg_index),
                        leg.segment_ids.clone(),
                        TraversalProfile::OnFoot,
                    )),
                    TransportMode::Watercraft => unverified_water_km += leg.distance_km,
                    TransportMode::Aerial => {}
                }
            }
        } else if let Some(ref segment_ids) = state.segment_path {
            let route_asset = state
                .uncertainty_waterfall
                .as_ref()
                .map(|wf| wf.resolved_asset.clone())
                .or_else(|| state.triage.as_ref().and_then(|t| t.required_asset.clone()));
            match route_asset {
                Some(asset) => land_checks.push((
                    "Route".to_string(),
                    segment_ids.clone(),
                    TraversalProfile::Vehicle(asset),
                )),
                None => {
                    verified = false;
                    verification_notes.push(
                        "FAILURE: Asset type unknown; vehicle access cannot be verified.".to_string(),
                    );
                }
            }
        }

        for (label, segment_ids, profile) in &land_checks {
            match self.state_mgr.verify_path_segments(segment_ids, profile).await {
                Ok(hazardous) if !hazardous.is_empty() => {
                    verified = false;
                    verification_notes.push(format!(
                        "CRITICAL SAFETY VIOLATION: {} traverses {} compromised or access-restricted segments: {:?}",
                        label, hazardous.len(), hazardous
                    ));
                }
                Ok(_) => {
                    info!(
                        "Independent safety check: {} — 100% of {} segments verified safe for {:?}.",
                        label, segment_ids.len(), profile
                    );
                }
                Err(e) => {
                    verified = false;
                    verification_notes.push(format!("SafetyVerificationFailed: {}", e));
                }
            }
        }

        if unverified_water_km > 0.0 {
            warn!(
                "Water corridor of {:.2} km cannot be verified from graph state; requires field confirmation.",
                unverified_water_km
            );
        }

        let dispatch_status = if verified && !state.has_errors() {
            if requires_mitigation {
                DispatchStatus::MitigationRequired
            } else {
                DispatchStatus::RoutedVerified
            }
        } else {
            DispatchStatus::EscalateHumanDispatcher
        };

        let primary_reason = Self::primary_dispatch_reason(
            &state,
            &verification_notes,
            dispatch_status == DispatchStatus::RoutedVerified || dispatch_status == DispatchStatus::MitigationRequired,
        );

        let action_required = if dispatch_status == DispatchStatus::RoutedVerified {
            "None".to_string()
        } else if dispatch_status == DispatchStatus::MitigationRequired {
            "Deploy primary road vehicle to Staging Junction and arrange Rescue Boat for the water corridor; confirm corridor depth, current and obstructions with field teams before launch.".to_string()
        } else if state
            .errors
            .iter()
            .any(|error| error.starts_with("TriageRequiresHumanReview:"))
        {
            "Confirm the missing or conflicting incident details, then resubmit.".to_string()
        } else if state
            .errors
            .iter()
            .any(|error| error.starts_with("TriageLocationResolutionFailed:"))
        {
            "Confirm an exact victim or pickup location, then resubmit.".to_string()
        } else if state
            .errors
            .iter()
            .any(|error| error.starts_with("RoutingError:"))
        {
            "Review route constraints and select a safe contingency.".to_string()
        } else {
            "Human dispatcher review is required before dispatch.".to_string()
        };

        let shelter_id = shelter
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| "NONE".to_string());
        let shelter_junction = shelter
            .as_ref()
            .map(|s| s.junction_id.clone())
            .unwrap_or_else(|| "NONE".to_string());

        let travel_time_text = if let Some(s) = state.total_travel_time_s {
            format!("{:.1} min ({:.0} s)", s / 60.0, s)
        } else {
            "N/A".to_string()
        };

        let waypoints_summary = if path.is_empty() {
            "N/A".to_string()
        } else if path.len() <= 6 {
            path.join(" -> ")
        } else {
            format!(
                "{} -> ... ({} hops) -> {}",
                path[0],
                path.len() - 2,
                path[path.len() - 1]
            )
        };

        let allocation_summary = if shelter.is_some() && state.assigned_asset.is_some() {
            "COMPLETED"
        } else if state.has_errors() {
            "SKIPPED"
        } else {
            "INCOMPLETE"
        };
        let route_summary = if path.is_empty() {
            if state.has_errors() {
                "SKIPPED"
            } else {
                "FAILED"
            }
        } else {
            "COMPUTED"
        };
        let detour_summary = if path.is_empty() {
            "N/A (route not computed)"
        } else {
            state
                .detour_reason
                .as_deref()
                .unwrap_or("None (Direct Clear Path)")
        };
        let verification_summary = if dispatch_status == DispatchStatus::RoutedVerified {
            "PASSED (100% Segments Independently Verified Safe)".to_string()
        } else if dispatch_status == DispatchStatus::MitigationRequired {
            format!(
                "PASSED FOR LAND LEGS (100% of road/foot segments verified); {:.2} km WATER CORRIDOR UNVERIFIED — requires field confirmation",
                unverified_water_km
            )
        } else if path.is_empty() {
            "NOT RUN (route unavailable)".to_string()
        } else if verification_notes.is_empty() {
            "FAILED (pipeline reported an unresolved error)".to_string()
        } else {
            verification_notes.join("; ")
        };
        let headcount_text = if let Some(ref wf) = state.uncertainty_waterfall {
            if wf.headcount_is_fallback {
                format!("{} Persons (STANDARD BASELINE FALLBACK)", wf.resolved_headcount)
            } else {
                format!("{} Persons", wf.resolved_headcount)
            }
        } else {
            state
                .triage
                .as_ref()
                .and_then(|triage| triage.headcount)
                .map(|value| format!("{value} Persons"))
                .unwrap_or_else(|| "UNKNOWN".to_string())
        };

        let mut waterfall_text = String::new();
        if let Some(ref wf) = state.uncertainty_waterfall {
            if wf.headcount_is_fallback || wf.asset_is_fallback {
                waterfall_text.push_str("\nUNCERTAINTY WATERFALL DIRECTIVES:\n");
                for d in &wf.field_operational_directives {
                    waterfall_text.push_str(&format!("  * {}\n", d));
                }
            }
            if let Some(ref cb) = wf.automated_callback_inquiry {
                waterfall_text.push_str(&format!("AUTOMATED CALLBACK INQUIRY: {}\n", cb));
            }
            if let Some(ref alert) = wf.dispatcher_dial_alert {
                waterfall_text.push_str(&format!("OPERATOR ALERT: {}\n", alert));
            }
        }

        let raw_brief = format!(
            "====================================================\n\
             TACTICAL DISPATCH ORDER [CRISISGRAPH ENGINE]\n\
             INCIDENT ID : {}\n\
             TIMESTAMP   : {}\n\
             STATUS      : {:?}\n\
             PRIMARY REASON : {}\n\
             ACTION REQUIRED: {}\n\
             ----------------------------------------------------\n\
             VICTIM LOC  : {} (Junction: {})\n\
             HEADCOUNT   : {}\n\
             DISPATCH    : {}\n\
             FROM SHELTER: {} (Junction: {})\n\
             ----------------------------------------------------\n\
             ALLOCATION  : {}\n\
             ROUTE       : {}\n\
             DISTANCE    : {:.2} km\n\
             EST. TIME   : {}\n\
             WAYPOINTS   : {}\n\
             DETOUR INFO : {}\n\
             VERIFICATION: {}\n\
             {}{}\
             ====================================================",
            incident_id,
            Utc::now().to_rfc3339(),
            dispatch_status,
            primary_reason,
            action_required,
            state
                .triage
                .as_ref()
                .and_then(|t| t.victim_location_raw.as_deref())
                .unwrap_or("Unknown"),
            target_junction,
            headcount_text,
            asset,
            shelter.as_ref().map(|s| s.name.as_str()).unwrap_or("None"),
            shelter_junction,
            allocation_summary,
            route_summary,
            distance,
            travel_time_text,
            waypoints_summary,
            detour_summary,
            verification_summary,
            if waterfall_text.is_empty() { "" } else { "----------------------------------------------------\n" },
            waterfall_text
        );

        let brief = TacticalBrief {
            incident_id,
            status: dispatch_status,
            target_victim_junction: target_junction,
            assigned_shelter_id: shelter_id,
            assigned_shelter_junction: shelter_junction,
            assigned_asset: asset,
            headcount,
            computed_route: path,
            segment_path: state.segment_path.clone().unwrap_or_default(),
            total_distance_km: distance,
            total_travel_time_s: state.total_travel_time_s.unwrap_or(0.0),
            detour_reason: state.detour_reason.clone(),
            timestamp: Utc::now(),
            raw_brief_text: raw_brief,
            multimodal_plan: state.multimodal_plan.clone(),
            uncertainty_waterfall: state.uncertainty_waterfall.clone(),
        };

        state.brief = Some(brief);
        state
    }

    fn humanize_pipeline_error(error: &str) -> String {
        let detail = error
            .split_once(": ")
            .map(|(_, detail)| detail)
            .unwrap_or(error)
            .trim()
            .trim_end_matches('.')
            .to_string();

        if detail.is_empty() {
            "Human dispatcher review was requested".to_string()
        } else if detail == "None" {
            String::new()
        } else {
            detail
        }
    }

    fn primary_dispatch_reason(
        state: &CrisisState,
        verification_notes: &[String],
        route_verified: bool,
    ) -> String {
        state
            .triage
            .as_ref()
            .filter(|triage| triage.needs_human_review)
            .and_then(|triage| triage.uncertainty_reasons.first().cloned())
            .or_else(|| {
                state
                    .errors
                    .iter()
                    .map(|reason| Self::humanize_pipeline_error(reason))
                    .find(|reason| !reason.is_empty() && reason != "None")
            })
            .or_else(|| verification_notes.first().cloned())
            .unwrap_or_else(|| {
                if route_verified {
                    "All dispatch-critical facts were complete and the route passed independent safety verification".to_string()
                } else {
                    "Human dispatcher review was requested".to_string()
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::PipelineStages;

    #[test]
    fn humanizes_typed_pipeline_error_for_dispatcher_brief() {
        assert_eq!(
            PipelineStages::humanize_pipeline_error(
                "TriageRequiresHumanReview: Required asset was not specified"
            ),
            "Required asset was not specified"
        );
    }

    #[test]
    fn preserves_untyped_pipeline_error_text() {
        assert_eq!(
            PipelineStages::humanize_pipeline_error("No shelters have sufficient capacity."),
            "No shelters have sufficient capacity"
        );
    }

    #[test]
    fn filters_meaningless_none_diagnostic() {
        assert_eq!(
            PipelineStages::humanize_pipeline_error("TriageLocationResolutionFailed: None"),
            ""
        );
    }

    #[test]
    fn replaces_empty_typed_error_with_review_reason() {
        assert_eq!(
            PipelineStages::humanize_pipeline_error("TriageRequiresHumanReview: "),
            "Human dispatcher review was requested"
        );
    }

    #[test]
    fn verified_dispatch_does_not_use_human_review_fallback_reason() {
        let state = crate::models::CrisisState::new(crate::models::SosAlert::new("SOS-TEST", "Test dispatch"));

        assert_eq!(
            PipelineStages::primary_dispatch_reason(&state, &[], true),
            "All dispatch-critical facts were complete and the route passed independent safety verification"
        );
        assert_eq!(
            PipelineStages::primary_dispatch_reason(&state, &[], false),
            "Human dispatcher review was requested"
        );
    }
}
