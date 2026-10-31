use neo4rs::{query, Row};

use super::{parse_asset, parse_enum, OpsRepoError, OpsRepository};
use crate::models::{AssetType, Vehicle, VehicleStatus};

/// Baseline fleet per pilot shelter: (shelter id, ID code, ambulances,
/// trucks, boats). Mirrors `StateManager::ensure_aluva_shelters`.
pub const BASELINE_FLEET: &[(&str, &str, u32, u32, u32)] = &[
    ("S_ALUVA_TOWNHALL", "TH", 6, 4, 0),
    ("S_UC_COLLEGE", "UC", 3, 8, 0),
    ("S_MANAPPURAM_DEPOT", "MP", 2, 2, 10),
    ("S_TALUK_HOSPITAL", "TK", 8, 0, 0),
];

/// Persons each vehicle type can carry.
#[derive(Debug, Clone, Copy)]
pub struct FleetCapacities {
    pub ambulance: u32,
    pub evac_truck: u32,
    pub rescue_boat: u32,
}

impl Default for FleetCapacities {
    fn default() -> Self {
        Self {
            ambulance: 4,
            evac_truck: 20,
            rescue_boat: 8,
        }
    }
}

const VEHICLE_FIELDS: &str = "v.vehicle_id AS vehicle_id, v.asset_type AS asset_type, \
     v.org AS org, v.capacity_persons AS capacity_persons, v.status AS status, \
     v.version AS version, v.home_shelter_id AS home_shelter_id, v.junction_id AS junction_id";

impl OpsRepository {
    /// Create vehicle records for the baseline fleet if missing. Existing
    /// vehicles are left exactly as they are, so this is safe to re-run.
    /// Returns the number of vehicles created.
    pub async fn seed_baseline_fleet(&self, capacities: FleetCapacities) -> Result<u64, OpsRepoError> {
        let mut vehicles = Vec::new();
        for (shelter_id, code, ambulances, trucks, boats) in BASELINE_FLEET {
            for (prefix, asset, count, capacity) in [
                ("AMB", AssetType::Ambulance, *ambulances, capacities.ambulance),
                ("TRK", AssetType::EvacTruck, *trucks, capacities.evac_truck),
                ("BOAT", AssetType::RescueBoat, *boats, capacities.rescue_boat),
            ] {
                for n in 1..=count {
                    vehicles.push(neo4rs::BoltType::from(std::collections::HashMap::from([
                        ("vehicle_id".to_string(), neo4rs::BoltType::from(format!("{prefix}-{code}-{n:02}"))),
                        ("asset_type".to_string(), neo4rs::BoltType::from(asset.to_string())),
                        ("capacity".to_string(), neo4rs::BoltType::from(capacity as i64)),
                        ("shelter_id".to_string(), neo4rs::BoltType::from(shelter_id.to_string())),
                    ])));
                }
            }
        }

        let q = query(
            "UNWIND $vehicles AS row \
             MATCH (s:Shelter {id: row.shelter_id}) \
             OPTIONAL MATCH (j:Junction {id: s.junction_id}) \
             MERGE (v:Vehicle {vehicle_id: row.vehicle_id}) \
             ON CREATE SET v.asset_type = row.asset_type, v.org = 'pilot-fleet', \
                 v.capacity_persons = row.capacity, v.status = 'AVAILABLE', v.version = 0, \
                 v.home_shelter_id = s.id, v.junction_id = s.junction_id, \
                 v.position = CASE WHEN j IS NULL THEN null \
                     ELSE point({latitude: j.lat, longitude: j.lon}) END, \
                 v.created_at = datetime(), v.updated_at = datetime(), v._created = true \
             MERGE (v)-[:BASED_AT]->(s) \
             WITH v, coalesce(v._created, false) AS created \
             REMOVE v._created \
             RETURN sum(CASE WHEN created THEN 1 ELSE 0 END) AS created",
        )
        .param("vehicles", vehicles);

        let created = match self.write_first_row(q).await? {
            Some(row) => row.get::<i64>("created")? as u64,
            None => 0,
        };
        Ok(created)
    }

    pub async fn get_vehicle(&self, vehicle_id: &str) -> Result<Option<Vehicle>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (v:Vehicle {{vehicle_id: $vehicle_id}}) RETURN {VEHICLE_FIELDS}"
        ))
        .param("vehicle_id", vehicle_id);
        let mut result = self.neo4j.graph.execute(q).await?;
        match result.next().await? {
            Some(row) => Ok(Some(vehicle_from_row(&row)?)),
            None => Ok(None),
        }
    }

    /// All vehicles currently `AVAILABLE` (the matcher's candidate set).
    pub async fn list_available_vehicles(&self) -> Result<Vec<Vehicle>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (v:Vehicle {{status: 'AVAILABLE'}}) RETURN {VEHICLE_FIELDS} ORDER BY v.vehicle_id"
        ));
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut vehicles = Vec::new();
        while let Some(row) = result.next().await? {
            vehicles.push(vehicle_from_row(&row)?);
        }
        Ok(vehicles)
    }

    /// `AVAILABLE` vehicles based at a shelter in `dataset` (the matcher's
    /// candidates; other datasets' fleets are never offered).
    pub async fn list_available_vehicles_in(&self, dataset: &str) -> Result<Vec<Vehicle>, OpsRepoError> {
        let q = query(&format!(
            "MATCH (v:Vehicle {{status: 'AVAILABLE'}})-[:BASED_AT]->(:Shelter {{dataset: $dataset}}) \
             WHERE v.junction_id IS NOT NULL \
             RETURN {VEHICLE_FIELDS} ORDER BY v.vehicle_id"
        ))
        .param("dataset", dataset);
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut vehicles = Vec::new();
        while let Some(row) = result.next().await? {
            vehicles.push(vehicle_from_row(&row)?);
        }
        Ok(vehicles)
    }
}

fn vehicle_from_row(row: &Row) -> Result<Vehicle, OpsRepoError> {
    Ok(Vehicle {
        vehicle_id: row.get("vehicle_id")?,
        asset_type: parse_asset(&row.get::<String>("asset_type")?)?,
        org: row.get("org")?,
        capacity_persons: row.get::<i64>("capacity_persons")? as u32,
        status: parse_enum(&row.get::<String>("status")?, VehicleStatus::parse, "vehicle status")?,
        version: row.get::<i64>("version")? as u64,
        home_shelter_id: row.get("home_shelter_id")?,
        junction_id: row.get("junction_id")?,
    })
}
