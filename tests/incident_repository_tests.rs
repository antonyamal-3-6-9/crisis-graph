use chrono::Utc;
use crisis_graph::config::Config;
use crisis_graph::db::{IncidentRepository, IncidentRepositoryError, Neo4jClient};
use crisis_graph::models::{
    ActorContext, ActorRole, AssetType, Incident, IncidentStatus, ReviewDecisionKind, SosAlert,
    TriageInferenceMetadata, TriageReport,
};
use neo4rs::query;
use uuid::Uuid;

fn system() -> ActorContext {
    ActorContext::new("repository-test-pipeline", [ActorRole::SystemService])
}

fn dispatcher() -> ActorContext {
    ActorContext::new("repository-test-dispatcher", [ActorRole::Dispatcher])
}

fn triage(needs_review: bool) -> TriageReport {
    TriageReport {
        victim_location_raw: Some("Aluva Railway Station".to_string()),
        resolved_junction_id: Some("node/4664235699".to_string()),
        headcount: Some(5),
        required_asset: Some(AssetType::Ambulance),
        hazards: vec![],
        confidence_score: if needs_review { 0.55 } else { 0.95 },
        needs_human_review: needs_review,
        uncertainty_reasons: if needs_review {
            vec!["Caller confirmation required".to_string()]
        } else {
            vec![]
        },
        inference: TriageInferenceMetadata {
            backend: "test".to_string(),
            model: "test".to_string(),
            adapter_id: None,
            adapter_sha256: None,
            prompt_version: "test".to_string(),
            schema_version: "triage-extraction-v2".to_string(),
            latency_ms: 1,
        },
    }
}

async fn cleanup(neo4j: &Neo4jClient, incident_id: &str) {
    let _ = neo4j
        .graph
        .run(
            query(
                "MATCH (:Incident {incident_id: $incident_id})-[:HAS_AUDIT_EVENT]->(e) \
                 DETACH DELETE e",
            )
            .param("incident_id", incident_id),
        )
        .await;
    let _ = neo4j
        .graph
        .run(
            query("MATCH (i:Incident {incident_id: $incident_id}) DETACH DELETE i")
                .param("incident_id", incident_id),
        )
        .await;
}

#[tokio::test]
async fn neo4j_repository_persists_audit_history_and_rejects_stale_writes() {
    dotenvy::dotenv().ok();
    let config = Config::from_env();
    let neo4j = match Neo4jClient::connect(&config).await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Neo4j unavailable; skipping live incident repository test: {error}");
            return;
        }
    };
    let repository = IncidentRepository::new(neo4j.clone());
    repository.setup_schema().await.unwrap();

    let incident_id = format!("TEST-INCIDENT-{}", Uuid::new_v4());
    cleanup(&neo4j, &incident_id).await;

    let result = async {
        let now = Utc::now();
        let alert = SosAlert {
            alert_id: incident_id.clone(),
            raw_text: "Five people at Aluva Railway Station need an ambulance.".to_string(),
            timestamp: now,
            source_channel: Some("repository_test".to_string()),
        };
        let (mut incident, receipt) = Incident::receive(alert, &system(), now).unwrap();
        repository.create(&incident, &receipt).await.unwrap();
        let duplicate = repository.create(&incident, &receipt).await.unwrap_err();
        assert!(
            matches!(
                duplicate,
                IncidentRepositoryError::AlreadyExists(ref id) if id == &incident_id
            ),
            "unexpected duplicate-create error: {duplicate:?}"
        );

        let extraction = incident
            .append_model_extraction(&system(), 1, triage(true), Utc::now())
            .unwrap();
        repository
            .save_transition(&incident, &extraction)
            .await
            .unwrap();

        let review_gate = incident
            .transition(
                &system(),
                2,
                IncidentStatus::ReviewRequired,
                "Deterministic review gate",
                Utc::now(),
            )
            .unwrap();
        repository
            .save_transition(&incident, &review_gate)
            .await
            .unwrap();

        let pending = repository
            .list_by_status(IncidentStatus::ReviewRequired, 50)
            .await
            .unwrap();
        assert!(pending.iter().any(|item| item.incident_id == incident_id));

        let mut stale = repository.get(&incident_id).await.unwrap().unwrap();
        let approval = incident
            .apply_review(
                &dispatcher(),
                3,
                ReviewDecisionKind::Approve,
                "Caller confirmed the extracted fields",
                Some(triage(false)),
                Utc::now(),
            )
            .unwrap();
        repository
            .save_transition(&incident, &approval)
            .await
            .unwrap();

        let stale_approval = stale
            .apply_review(
                &dispatcher(),
                3,
                ReviewDecisionKind::Approve,
                "Stale competing decision",
                Some(triage(false)),
                Utc::now(),
            )
            .unwrap();
        let conflict = repository
            .save_transition(&stale, &stale_approval)
            .await
            .unwrap_err();
        assert!(matches!(
            conflict,
            IncidentRepositoryError::VersionConflict {
                expected: 3,
                actual: 4,
                ..
            }
        ));

        let stored = repository.get(&incident_id).await.unwrap().unwrap();
        assert_eq!(stored.status, IncidentStatus::TriageApproved);
        assert_eq!(stored.version, 4);
        assert_eq!(stored.triage_revisions.len(), 2);

        let audit = repository.audit_history(&incident_id).await.unwrap();
        assert_eq!(audit.len(), 4);
        assert_eq!(audit.first().unwrap().action, "INCIDENT_RECEIVED");
        assert_eq!(audit.last().unwrap().new_version, 4);
    }
    .await;

    cleanup(&neo4j, &incident_id).await;
    result
}
