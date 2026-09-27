use chrono::{TimeZone, Utc};
use crisis_graph::models::{
    ActorContext, ActorRole, AssetType, CandidateHazardReport, ControlPlaneError, Incident,
    IncidentStatus, ReviewDecisionKind, RoadStatus, RouteCostObjective, SosAlert,
    TriageInferenceMetadata, TriageReport, TriageRevisionSource, VerifiedRouteInput,
};

fn at(second: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 24, 10, 0, second)
        .single()
        .unwrap()
}

fn alert() -> SosAlert {
    SosAlert {
        alert_id: "SOS-CONTROL-1".to_string(),
        raw_text: "Five people at Aluva Railway Station need an ambulance.".to_string(),
        timestamp: at(0),
        source_channel: Some("test".to_string()),
    }
}

fn triage(review: bool) -> TriageReport {
    TriageReport {
        victim_location_raw: Some("Aluva Railway Station".to_string()),
        resolved_junction_id: Some("node/4664235699".to_string()),
        headcount: Some(5),
        required_asset: Some(AssetType::Ambulance),
        hazards: vec![CandidateHazardReport {
            road_segment: Some("way/1/seg/0/fwd".to_string()),
            status: Some(RoadStatus::Open),
            duration_hours: Some(2),
        }],
        confidence_score: if review { 0.55 } else { 0.95 },
        needs_human_review: review,
        uncertainty_reasons: if review {
            vec!["Caller requested confirmation".to_string()]
        } else {
            vec![]
        },
        inference: TriageInferenceMetadata {
            backend: "llama-cpp".to_string(),
            model: "test-model".to_string(),
            adapter_id: Some("test-adapter".to_string()),
            adapter_sha256: None,
            prompt_version: "triage-extraction-v2.1.1".to_string(),
            schema_version: "triage-extraction-v2".to_string(),
            latency_ms: 10,
        },
    }
}

fn system() -> ActorContext {
    ActorContext::new("pipeline", [ActorRole::SystemService])
}

fn dispatcher() -> ActorContext {
    ActorContext::new("dispatcher-7", [ActorRole::Dispatcher])
}

fn responder() -> ActorContext {
    ActorContext::new("driver-3", [ActorRole::Responder])
}

fn incident() -> Incident {
    Incident::receive(alert(), &system(), at(0)).unwrap().0
}

fn verifying_incident() -> Incident {
    let mut incident = incident();
    incident
        .append_model_extraction(&system(), 1, triage(false), at(1))
        .unwrap();
    incident.status = IncidentStatus::Verifying;
    incident.version = 7;
    incident
}

fn verified_route() -> VerifiedRouteInput {
    VerifiedRouteInput {
        dataset: "aluva-periyar-pilot".to_string(),
        cost_objective: RouteCostObjective::FastestTime,
        origin_shelter_id: "S_TALUK_HOSPITAL".to_string(),
        origin_shelter_name: "Aluva Taluk Hospital".to_string(),
        origin_junction_id: "node/origin".to_string(),
        victim_junction_id: "node/victim".to_string(),
        assigned_asset_id: "AMBULANCE_UNIT_1".to_string(),
        junction_path: vec![
            "node/origin".to_string(),
            "node/middle".to_string(),
            "node/victim".to_string(),
        ],
        segment_path: vec!["segment/1".to_string(), "segment/2".to_string()],
        total_distance_km: 1.25,
        total_travel_time_s: 210.0,
        detour_reason: None,
    }
}

#[test]
fn receiving_incident_requires_ingest_permission_and_creates_first_audit_event() {
    let error = Incident::receive(alert(), &dispatcher(), at(0)).unwrap_err();
    assert!(matches!(error, ControlPlaneError::Forbidden { .. }));

    let (incident, audit) = Incident::receive(alert(), &system(), at(0)).unwrap();
    assert_eq!(incident.status, IncidentStatus::Received);
    assert_eq!(incident.version, 1);
    assert_eq!(audit.action, "INCIDENT_RECEIVED");
    assert_eq!(audit.previous_version, 0);
    assert_eq!(audit.new_version, 1);
}

#[test]
fn status_storage_name_matches_its_json_contract() {
    for status in [
        IncidentStatus::Received,
        IncidentStatus::ReviewRequired,
        IncidentStatus::RouteVerified,
        IncidentStatus::EnRoute,
        IncidentStatus::RouteInvalidated,
    ] {
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            format!("\"{}\"", status.as_str())
        );
    }
}

#[test]
fn records_model_extraction_as_first_immutable_revision() {
    let mut incident = incident();
    let audit = incident
        .append_model_extraction(&system(), 1, triage(true), at(1))
        .unwrap();

    assert_eq!(incident.status, IncidentStatus::TriageExtracted);
    assert_eq!(incident.version, 2);
    assert_eq!(incident.triage_revisions.len(), 1);
    assert_eq!(incident.triage_revisions[0].revision, 1);
    assert_eq!(
        incident.triage_revisions[0].source,
        TriageRevisionSource::Model
    );
    assert_eq!(audit.previous_version, 1);
    assert_eq!(audit.new_version, 2);
}

#[test]
fn dispatcher_approval_appends_revision_and_preserves_model_output() {
    let mut incident = incident();
    incident
        .append_model_extraction(&system(), 1, triage(true), at(1))
        .unwrap();
    incident
        .transition(
            &system(),
            2,
            IncidentStatus::ReviewRequired,
            "Deterministic review gate",
            at(2),
        )
        .unwrap();

    let audit = incident
        .apply_review(
            &dispatcher(),
            3,
            ReviewDecisionKind::Approve,
            "Caller confirmed all fields",
            Some(triage(false)),
            at(3),
        )
        .unwrap();

    assert_eq!(incident.status, IncidentStatus::TriageApproved);
    assert_eq!(incident.version, 4);
    assert_eq!(incident.triage_revisions.len(), 2);
    assert!(incident.triage_revisions[0].triage.needs_human_review);
    assert!(!incident.triage_revisions[1].triage.needs_human_review);
    assert_eq!(
        incident.triage_revisions[1].source,
        TriageRevisionSource::Dispatcher
    );
    assert_eq!(
        incident.review_decisions[0].resulting_triage_revision,
        Some(2)
    );
    assert_eq!(audit.to_status, IncidentStatus::TriageApproved);
}

#[test]
fn stale_review_is_rejected_without_mutating_incident() {
    let mut incident = incident();
    incident
        .append_model_extraction(&system(), 1, triage(true), at(1))
        .unwrap();
    incident
        .transition(
            &system(),
            2,
            IncidentStatus::ReviewRequired,
            "Review required",
            at(2),
        )
        .unwrap();

    let error = incident
        .apply_review(
            &dispatcher(),
            2,
            ReviewDecisionKind::Approve,
            "Stale browser tab",
            Some(triage(false)),
            at(3),
        )
        .unwrap_err();

    assert_eq!(
        error,
        ControlPlaneError::VersionConflict {
            expected: 2,
            actual: 3
        }
    );
    assert_eq!(incident.status, IncidentStatus::ReviewRequired);
    assert_eq!(incident.triage_revisions.len(), 1);
    assert!(incident.review_decisions.is_empty());
}

#[test]
fn responder_cannot_approve_an_incident() {
    let mut incident = incident();
    incident
        .append_model_extraction(&system(), 1, triage(true), at(1))
        .unwrap();
    incident
        .transition(
            &system(),
            2,
            IncidentStatus::ReviewRequired,
            "Review required",
            at(2),
        )
        .unwrap();

    let error = incident
        .apply_review(
            &responder(),
            3,
            ReviewDecisionKind::Approve,
            "Not authorized",
            Some(triage(false)),
            at(3),
        )
        .unwrap_err();

    assert!(matches!(error, ControlPlaneError::Forbidden { .. }));
    assert_eq!(incident.status, IncidentStatus::ReviewRequired);
}

#[test]
fn approval_rejects_triage_that_still_requires_review() {
    let mut incident = incident();
    incident
        .append_model_extraction(&system(), 1, triage(true), at(1))
        .unwrap();
    incident
        .transition(
            &system(),
            2,
            IncidentStatus::ReviewRequired,
            "Review required",
            at(2),
        )
        .unwrap();

    let error = incident
        .apply_review(
            &dispatcher(),
            3,
            ReviewDecisionKind::Approve,
            "Incomplete correction",
            Some(triage(true)),
            at(3),
        )
        .unwrap_err();

    assert_eq!(error, ControlPlaneError::ReviewedTriageNotDispatchReady);
    assert_eq!(incident.status, IncidentStatus::ReviewRequired);
    assert_eq!(incident.version, 3);
}

#[test]
fn generic_transition_cannot_bypass_review_revision_and_audit_contract() {
    let mut incident = incident();
    incident
        .append_model_extraction(&system(), 1, triage(true), at(1))
        .unwrap();
    incident
        .transition(
            &system(),
            2,
            IncidentStatus::ReviewRequired,
            "Review required",
            at(2),
        )
        .unwrap();

    let error = incident
        .transition(
            &dispatcher(),
            3,
            IncidentStatus::TriageApproved,
            "Attempted approval without a reviewed revision",
            at(3),
        )
        .unwrap_err();

    assert_eq!(error, ControlPlaneError::DedicatedReviewActionRequired);
    assert_eq!(incident.status, IncidentStatus::ReviewRequired);
    assert_eq!(incident.version, 3);
}

#[test]
fn state_machine_prevents_skipping_safety_stages() {
    let mut incident = incident();
    let error = incident
        .transition(
            &system(),
            1,
            IncidentStatus::RouteVerified,
            "Attempted shortcut",
            at(1),
        )
        .unwrap_err();

    assert_eq!(
        error,
        ControlPlaneError::InvalidTransition {
            from: IncidentStatus::Received,
            to: IncidentStatus::RouteVerified,
        }
    );
    assert_eq!(incident.status, IncidentStatus::Received);
    assert_eq!(incident.version, 1);
}

#[test]
fn verified_route_is_recorded_as_immutable_evidence_with_the_transition() {
    let mut incident = verifying_incident();

    let audit = incident
        .record_verified_route(&system(), 7, verified_route(), at(1))
        .unwrap();

    assert_eq!(incident.status, IncidentStatus::RouteVerified);
    assert_eq!(incident.version, 8);
    assert_eq!(incident.route_decisions.len(), 1);
    let decision = incident.current_route().unwrap();
    assert_eq!(decision.route_version, 1);
    assert_eq!(decision.triage_revision, 1);
    assert_eq!(decision.segment_path, ["segment/1", "segment/2"]);
    assert_eq!(decision.verified_segment_count, 2);
    assert_eq!(decision.verified_by, "pipeline");
    assert_eq!(audit.to_status, IncidentStatus::RouteVerified);
    assert_eq!(audit.new_version, 8);
}

#[test]
fn generic_transition_cannot_mark_route_verified_without_route_evidence() {
    let mut incident = verifying_incident();

    let error = incident
        .transition(
            &system(),
            7,
            IncidentStatus::RouteVerified,
            "Attempted verification without evidence",
            at(1),
        )
        .unwrap_err();

    assert_eq!(
        error,
        ControlPlaneError::DedicatedRouteVerificationActionRequired
    );
    assert!(incident.route_decisions.is_empty());
    assert_eq!(incident.status, IncidentStatus::Verifying);
}

#[test]
fn verified_route_rejects_inconsistent_path_evidence_without_mutation() {
    let mut incident = verifying_incident();
    let mut route = verified_route();
    route.victim_junction_id = "node/other".to_string();

    let error = incident
        .record_verified_route(&system(), 7, route, at(1))
        .unwrap_err();

    assert!(matches!(error, ControlPlaneError::InvalidVerifiedRoute(_)));
    assert_eq!(incident.status, IncidentStatus::Verifying);
    assert_eq!(incident.version, 7);
    assert!(incident.route_decisions.is_empty());
}

#[test]
fn older_incident_snapshots_deserialize_with_no_route_decisions() {
    let incident = incident();
    let mut snapshot = serde_json::to_value(&incident).unwrap();
    snapshot.as_object_mut().unwrap().remove("route_decisions");

    let restored: Incident = serde_json::from_value(snapshot).unwrap();

    assert!(restored.route_decisions.is_empty());
}

#[test]
fn assigned_route_can_be_invalidated_and_recomputed() {
    let mut incident = incident();
    incident.status = IncidentStatus::Assigned;
    incident.version = 8;

    incident
        .transition(
            &system(),
            8,
            IncidentStatus::RouteInvalidated,
            "New verified flood intersects route version 2",
            at(1),
        )
        .unwrap();
    incident
        .transition(
            &system(),
            9,
            IncidentStatus::Routing,
            "Compute replacement route",
            at(2),
        )
        .unwrap();

    assert_eq!(incident.status, IncidentStatus::Routing);
    assert_eq!(incident.version, 10);
}
