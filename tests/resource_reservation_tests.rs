use crisis_graph::config::Config;
use crisis_graph::db::{Neo4jClient, StateManager};
use crisis_graph::models::AssetType;
use neo4rs::query;
use uuid::Uuid;

#[tokio::test]
async fn reservation_is_incident_owned_and_compensation_is_idempotent() {
    dotenvy::dotenv().ok();
    let config = Config::from_env();
    let neo4j = match Neo4jClient::connect(&config).await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Neo4j unavailable; skipping live reservation test: {error}");
            return;
        }
    };
    let state = StateManager::new(neo4j.clone());
    state.ensure_operational_schema().await.unwrap();

    let incident_id = format!("TEST-RESERVATION-{}", Uuid::new_v4());
    let shelter_id = "S_UC_COLLEGE";
    let before = state
        .get_shelters_with_locations()
        .await
        .unwrap()
        .into_iter()
        .find(|shelter| shelter.id == shelter_id)
        .expect("UC College shelter must exist");

    let test_result: Result<(), String> = async {
        let reserved = state
            .reserve_shelter_asset(shelter_id, &AssetType::Ambulance, 2, &incident_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("first reservation unexpectedly failed")?;
        if reserved.current_occupancy != before.current_occupancy + 2
            || reserved.ambulances_available + 1 != before.ambulances_available
        {
            return Err("first reservation did not mutate the expected counters".to_string());
        }

        let duplicate = state
            .reserve_shelter_asset(shelter_id, &AssetType::Ambulance, 2, &incident_id)
            .await
            .map_err(|error| error.to_string())?;
        if duplicate.is_some() {
            return Err("duplicate incident reservation was applied twice".to_string());
        }

        if !state
            .release_incident_reservation(&incident_id, "integration test cleanup")
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("first compensation did not release the reservation".to_string());
        }
        if state
            .release_incident_reservation(&incident_id, "duplicate cleanup attempt")
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("compensation was not idempotent".to_string());
        }

        let after = state
            .get_shelters_with_locations()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|shelter| shelter.id == shelter_id)
            .ok_or("UC College shelter disappeared")?;
        if after.current_occupancy != before.current_occupancy
            || after.ambulances_available != before.ambulances_available
        {
            return Err("compensation did not restore the original counters".to_string());
        }
        Ok(())
    }
    .await;

    // Always attempt cleanup, including after an assertion-style error above.
    let _ = state
        .release_incident_reservation(&incident_id, "final test cleanup")
        .await;
    let _ = neo4j
        .graph
        .run(
            query("MATCH (r:ResourceReservation {incident_id: $incident_id}) DETACH DELETE r")
                .param("incident_id", incident_id),
        )
        .await;

    if let Err(error) = test_result {
        panic!("{error}");
    }
}
