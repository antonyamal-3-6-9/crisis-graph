use chrono::{TimeZone, Utc};
use crisis_graph::models::{
    ActorContext, ActorRole, AssetType, CandidateHazardReport, ControlPlaneError, Incident,
    IncidentStatus, ReviewDecisionKind, RoadStatus, SosAlert, TriageInferenceMetadata,
    TriageReport, TriageRevisionSource,
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
