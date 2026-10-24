use crisis_graph::config::Config;
use crisis_graph::db::{Neo4jClient, StateManager};
use crisis_graph::models::{AssetType, ReservationRole};
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
            .reserve_shelter_asset(shelter_id, &AssetType::Ambulance, 2, &incident_id, ReservationRole::RoadPrimary)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("first reservation unexpectedly failed")?;
        if reserved.current_occupancy != before.current_occupancy + 2
            || reserved.ambulances_available + 1 != before.ambulances_available
        {
            return Err("first reservation did not mutate the expected counters".to_string());
        }

        let duplicate = state
            .reserve_shelter_asset(shelter_id, &AssetType::Ambulance, 2, &incident_id, ReservationRole::RoadPrimary)
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

#[tokio::test]
async fn incident_holds_one_reservation_per_role_and_release_frees_all() {
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

    let incident_id = format!("TEST-RESERVATION-ROLES-{}", Uuid::new_v4());
    let road_shelter = "S_TALUK_HOSPITAL";
    let boat_depot = "S_MANAPPURAM_DEPOT";
    let snapshot = |shelters: Vec<crisis_graph::db::ShelterWithLocation>, id: &str| {
        shelters.into_iter().find(|s| s.id == id).expect("seeded shelter must exist")
    };
    let all_before = state.get_shelters_with_locations().await.unwrap();
    let road_before = snapshot(all_before.clone(), road_shelter);
    let boat_before = snapshot(all_before, boat_depot);

    let test_result: Result<(), String> = async {
        state
            .reserve_shelter_asset(road_shelter, &AssetType::Ambulance, 3, &incident_id, ReservationRole::RoadPrimary)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("road reservation unexpectedly failed")?;

        // Previously refused because the incident already had an active reservation.
        let boat = state
            .reserve_shelter_asset(boat_depot, &AssetType::RescueBoat, 0, &incident_id, ReservationRole::WaterMitigation)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("water mitigation reservation was refused for an incident with a road reservation")?;
        if boat.boats_available + 1 != boat_before.boats_available
            || boat.current_occupancy != boat_before.current_occupancy
        {
            return Err("boat reservation did not mutate only the boat counter".to_string());
        }

        let duplicate_boat = state
            .reserve_shelter_asset(boat_depot, &AssetType::RescueBoat, 0, &incident_id, ReservationRole::WaterMitigation)
            .await
            .map_err(|e| e.to_string())?;
        if duplicate_boat.is_some() {
            return Err("second boat reservation for the same role was applied".to_string());
        }

        let active_roles: Vec<String> = state
            .get_resource_reservations(200)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|r| r.incident_id == incident_id && r.status == "RESERVED")
            .map(|r| r.role)
            .collect();
        if active_roles.len() != 2 {
            return Err(format!("expected two active reservations, found {active_roles:?}"));
        }

        if !state
            .release_incident_reservation(&incident_id, "integration test release")
            .await
            .map_err(|e| e.to_string())?
        {
            return Err("release did not report freed reservations".to_string());
        }

        let all_after = state.get_shelters_with_locations().await.map_err(|e| e.to_string())?;
        let road_after = snapshot(all_after.clone(), road_shelter);
        let boat_after = snapshot(all_after, boat_depot);
        if road_after.ambulances_available != road_before.ambulances_available
            || road_after.current_occupancy != road_before.current_occupancy
            || boat_after.boats_available != boat_before.boats_available
        {
            return Err("release did not restore both the ambulance and the boat".to_string());
        }
        Ok(())
    }
    .await;

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
