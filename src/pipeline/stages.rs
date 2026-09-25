use crate::db::{RedisClient, StateManager};
use crate::ingestion::{SpatialResolver, TriageExtractor};
use crate::models::{CrisisState, DispatchStatus, TacticalBrief};
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
                    // 1. Check for explicit or embedded GPS coordinates
                    if let Some((lat, lon)) =
                        SpatialResolver::parse_coordinates(&state.sos.raw_text)
                    {
                        info!("Found GPS coordinates ({lat}, {lon}) in SOS text. Snapping to nearest junction...");
                        match self
                            .state_mgr
                            .find_nearest_junction("aluva-periyar-pilot", lat, lon, 1000.0)
                            .await
                        {
                            Ok(Some((junction_id, dist))) => {
                                info!("Snapped GPS ({lat}, {lon}) -> {junction_id} ({dist:.1} m away)");
                                triage.resolved_junction_id = Some(junction_id);
                            }
                            Ok(None) => {
                                warn!("No road junction found within 1000m of ({lat}, {lon})")
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

                if triage.resolved_junction_id.is_none() {
                    state.errors.push(format!(
                        "TriageLocationResolutionFailed: {:?}",
                        triage.victim_location_raw
                    ));
                }

                if triage.needs_human_review {
                    state.errors.push(format!(
                        "TriageRequiresHumanReview: {}",
                        triage.uncertainty_reasons.join("; ")
                    ));
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

        let headcount = match triage.headcount {
            Some(value) => value,
            None => {
                state
                    .errors
                    .push("Cannot allocate assets: missing triage headcount".to_string());
                return state;
            }
        };
        let required_asset = match triage.required_asset.as_ref() {
            Some(value) => value,
            None => {
                state
                    .errors
                    .push("Cannot allocate assets: missing required asset".to_string());
                return state;
            }
        };

        // Query candidate shelters
        match self
            .state_mgr
            .find_candidate_shelters(required_asset, headcount)
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
                                    required_asset,
                                    headcount,
                                    &state.sos.alert_id,
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
                    state
                        .errors
                        .push("Failed to atomically reserve any eligible shelter".to_string());
                }
            }
            Ok(_) => {
                state
                    .errors
                    .push("No shelters have sufficient capacity or required assets".to_string());
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

        let required_asset = match triage.required_asset.clone() {
            Some(value) => value,
            None => {
                state
                    .errors
                    .push("Missing required asset for path calculation".to_string());
                return state;
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
            // Query active passable directed subgraph from Neo4j (Aluva pilot dataset)
            let edges = match self
                .state_mgr
                .get_passable_directed_subgraph("aluva-periyar-pilot")
                .await
            {
                Ok(edges) => edges,
                Err(e) => {
                    state.errors.push(format!(
                        "Failed to retrieve passable directed graph from Neo4j: {}",
                        e
                    ));
                    return state;
                }
            };

            let query =
                RoutingQuery::new(start_junction, target_junction).with_asset_type(required_asset);

            match DeterministicPathfinder::find_directed_route(&edges, &query) {
                Ok(route) => {
                    info!(
                        "Directed route computed: {} waypoints, {} segments, {:.2} km, {:.1} s",
                        route.path.len(),
                        route.segment_path.len(),
                        route.total_distance_km,
                        route.total_travel_time_s
                    );
                    state.computed_path = Some(route.path);
                    state.segment_path = Some(route.segment_path);
                    state.total_distance_km = Some(route.total_distance_km);
                    state.total_travel_time_s = Some(route.total_travel_time_s);
                    state.detour_reason = route.detour_reason;
                }
                Err(e) => {
                    error!("Directed routing failed: {}", e);
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
        let headcount = state.triage.as_ref().and_then(|t| t.headcount).unwrap_or(0);
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

        // Assertion 3: Independent live safety verification of every traversed road segment against operational overlay
        if let Some(ref segment_ids) = state.segment_path {
            match self.state_mgr.verify_path_segments(segment_ids).await {
                Ok(hazardous) if !hazardous.is_empty() => {
                    verified = false;
                    verification_notes.push(format!(
                        "CRITICAL SAFETY VIOLATION: Route traverses {} actively compromised/flooded segments: {:?}",
                        hazardous.len(), hazardous
                    ));
                }
                Ok(_) => {
                    info!(
                        "Independent safety check: 100% of {} traversed segments verified safe.",
                        segment_ids.len()
                    );
                }
                Err(e) => {
                    verified = false;
                    verification_notes.push(format!("SafetyVerificationFailed: {}", e));
                }
            }
        }

        let dispatch_status = if verified && !state.has_errors() {
            DispatchStatus::RoutedVerified
        } else {
            DispatchStatus::EscalateHumanDispatcher
        };

        let primary_reason = state
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
            .unwrap_or_else(|| "Human dispatcher review was requested".to_string());

        let action_required = if dispatch_status == DispatchStatus::RoutedVerified {
            "None".to_string()
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
        } else if path.is_empty() {
            "NOT RUN (route unavailable)".to_string()
        } else if verification_notes.is_empty() {
            "FAILED (pipeline reported an unresolved error)".to_string()
        } else {
            verification_notes.join("; ")
        };
        let headcount_text = state
            .triage
            .as_ref()
            .and_then(|triage| triage.headcount)
            .map(|value| format!("{value} Persons"))
            .unwrap_or_else(|| "UNKNOWN".to_string());

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
            verification_summary
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
}
