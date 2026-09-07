use chrono::{Duration, Utc};
use neo4rs::query;
use serde::{Deserialize, Serialize};
use crate::geospatial::{AccessFlags, BaselineStatus};
use crate::models::{AssetType, HazardReport, RoadStatus, Shelter};
use crate::solver::RoutingEdge;
use super::neo4j::Neo4jClient;

#[derive(Clone)]
pub struct StateManager {
    neo4j: Neo4jClient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShelterWithLocation {
    pub id: String,
    pub name: String,
    pub junction_id: String,
    pub capacity: u32,
    pub current_occupancy: u32,
    pub boats_available: u32,
    pub ambulances_available: u32,
    pub trucks_available: u32,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveHazardInfo {
    pub segment_id: String,
    pub road_name: String,
    pub operational_status: String,
    pub from_id: String,
    pub to_id: String,
    pub from_lat: f64,
    pub from_lon: f64,
    pub to_lat: f64,
    pub to_lon: f64,
}

#[derive(Debug, Clone)]
pub struct PassableEdge {
    pub source: String,
    pub target: String,
    pub weight: f64,
}

impl StateManager {
    pub fn new(neo4j: Neo4jClient) -> Self {
        Self { neo4j }
    }

    /// Mutate road segment with hazard condition and bi-temporal expiration timestamp
    pub async fn apply_hazard(&self, hazard: &HazardReport) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let parts: Vec<&str> = hazard.road_segment.split('-').collect();
        if parts.len() != 2 {
            return Err(format!("Invalid road segment identifier: {}", hazard.road_segment).into());
        }
        let src = parts[0].trim();
        let dst = parts[1].trim();

        let status_str = match hazard.status {
            RoadStatus::Open => "OPEN",
            RoadStatus::Blocked => "BLOCKED",
            RoadStatus::Flooded => "FLOODED",
        };

        let weight = if hazard.status == RoadStatus::Open {
            1.0
        } else {
            999999.0
        };

        let valid_until = (Utc::now() + Duration::hours(hazard.duration_hours as i64)).to_rfc3339();

        let q = query(
            "MATCH (a:Junction {id: $src})-[r:CONNECTS_TO]-(b:Junction {id: $dst}) \
             SET r.status = $status, \
                 r.active_weight = $weight, \
                 r.valid_until = datetime($valid_until) \
             RETURN count(r) AS updated"
        )
        .param("src", src)
        .param("dst", dst)
        .param("status", status_str)
        .param("weight", weight)
        .param("valid_until", valid_until);

        self.neo4j.graph.run(q).await?;
        Ok(())
    }

    /// Fetch all passable edges from Neo4j (edges where status = 'OPEN' OR valid_until < datetime())
    pub async fn get_passable_subgraph(&self) -> Result<Vec<PassableEdge>, Box<dyn std::error::Error + Send + Sync>> {
        let q = query(
            "MATCH (a:Junction)-[r:CONNECTS_TO]->(b:Junction) \
             WHERE r.status = 'OPEN' OR r.valid_until < datetime() \
             RETURN a.id AS source, b.id AS target, r.base_distance_km AS weight"
        );

        let mut result = self.neo4j.graph.execute(q).await?;
        let mut edges = Vec::new();

        while let Some(row) = result.next().await? {
            let source: String = row.get("source")?;
            let target: String = row.get("target")?;
            let weight: f64 = row.get("weight")?;
            edges.push(PassableEdge { source, target, weight });
        }

        Ok(edges)
    }

    /// Fetch directed routing edges for a dataset from Neo4j, incorporating dynamic operational status
    pub async fn get_passable_directed_subgraph(
        &self,
        dataset: &str,
    ) -> Result<Vec<RoutingEdge>, Box<dyn std::error::Error + Send + Sync>> {
        let q = query(
            "MATCH (a:Junction {dataset: $dataset})-[r:CONNECTS_TO {dataset: $dataset}]->(b:Junction {dataset: $dataset}) \
             WHERE (r.baseline_status = 'OPEN' OR r.baseline_status = 'RESTRICTED') \
               AND (r.operational_status IS NULL OR r.operational_status = 'OPEN' OR r.valid_until < datetime()) \
             RETURN r.segment_id AS segment_id, \
                    a.id AS source, \
                    b.id AS target, \
                    r.length_m AS length_m, \
                    r.travel_time_s AS travel_time_s, \
                    r.road_class AS road_class, \
                    r.oneway AS oneway, \
                    r.emergency AS emergency, \
                    r.motorcar AS motorcar, \
                    r.hgv AS hgv, \
                    r.baseline_status AS baseline_status"
        )
        .param("dataset", dataset);

        let mut result = self.neo4j.graph.execute(q).await?;
        let mut edges = Vec::new();

        while let Some(row) = result.next().await? {
            let segment_id: String = row.get("segment_id")?;
            let source: String = row.get("source")?;
            let target: String = row.get("target")?;
            let length_m: f64 = row.get("length_m")?;
            let travel_time_s: f64 = row.get("travel_time_s")?;
            let road_class: String = row.get("road_class")?;
            let oneway: bool = row.get("oneway")?;
            let emergency: bool = row.get("emergency")?;
            let motorcar: bool = row.get("motorcar")?;
            let hgv: bool = row.get("hgv")?;
            let status_str: String = row.get("baseline_status")?;
            let baseline_status = match status_str.as_str() {
                "OPEN" => BaselineStatus::Open,
                "RESTRICTED" => BaselineStatus::Restricted,
                _ => BaselineStatus::Unsuitable,
            };

            edges.push(RoutingEdge {
                segment_id,
                source,
                target,
                length_m,
                travel_time_s,
                road_class,
                access: AccessFlags {
                    motorcar,
                    emergency,
                    hgv,
                },
                baseline_status,
                oneway,
            });
        }

        Ok(edges)
    }

    /// Mutate a specific road segment with an operational hazard and bi-temporal expiration timestamp
    pub async fn apply_segment_hazard(
        &self,
        segment_id: &str,
        status: RoadStatus,
        duration_hours: u32,
    ) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        let status_str = match status {
            RoadStatus::Open => "OPEN",
            RoadStatus::Blocked => "BLOCKED",
            RoadStatus::Flooded => "FLOODED",
        };
        let valid_until = (Utc::now() + Duration::hours(duration_hours as i64)).to_rfc3339();
        let q = query(
            "MATCH ()-[r:CONNECTS_TO {segment_id: $segment_id}]->() \
             SET r.operational_status = $status, \
                 r.valid_until = datetime($valid_until) \
             RETURN count(r) AS updated"
        )
        .param("segment_id", segment_id)
        .param("status", status_str)
        .param("valid_until", valid_until);

        let mut result = self.neo4j.graph.execute(q).await?;
        if let Some(row) = result.next().await? {
            Ok(row.get("updated")?)
        } else {
            Ok(0)
        }
    }

    /// Clear all operational hazards for a dataset
    pub async fn clear_operational_hazards(
        &self,
        dataset: &str,
    ) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        let q = query(
            "MATCH ()-[r:CONNECTS_TO {dataset: $dataset}]->() \
             WHERE r.operational_status IS NOT NULL \
             SET r.operational_status = NULL, \
                 r.valid_until = NULL \
             RETURN count(r) AS cleared"
        )
        .param("dataset", dataset);

        let mut result = self.neo4j.graph.execute(q).await?;
        if let Some(row) = result.next().await? {
            Ok(row.get("cleared")?)
        } else {
            Ok(0)
        }
    }

    /// Spatial Snapper: Find the nearest junction to a given (latitude, longitude) within max_radius_m
    pub async fn find_nearest_junction(
        &self,
        dataset: &str,
        lat: f64,
        lon: f64,
        max_radius_m: f64,
    ) -> Result<Option<(String, f64)>, Box<dyn std::error::Error + Send + Sync>> {
        let q = query(
            "MATCH (j:Junction {dataset: $dataset}) \
             WHERE j.location IS NOT NULL AND point.distance(j.location, point({latitude: $lat, longitude: $lon})) <= $radius \
             RETURN j.id AS id, point.distance(j.location, point({latitude: $lat, longitude: $lon})) AS dist \
             ORDER BY dist ASC LIMIT 1"
        )
        .param("dataset", dataset)
        .param("lat", lat)
        .param("lon", lon)
        .param("radius", max_radius_m);

        let mut result = self.neo4j.graph.execute(q).await?;
        if let Some(row) = result.next().await? {
            let id: String = row.get("id")?;
            let dist: f64 = row.get("dist")?;
            Ok(Some((id, dist)))
        } else {
            // Fallback for nodes where location point hasn't been precomputed
            let fallback_q = query(
                "MATCH (j:Junction {dataset: $dataset}) \
                 WITH j, point.distance(point({latitude: j.lat, longitude: j.lon}), point({latitude: $lat, longitude: $lon})) AS dist \
                 WHERE dist <= $radius \
                 RETURN j.id AS id, dist \
                 ORDER BY dist ASC LIMIT 1"
            )
            .param("dataset", dataset)
            .param("lat", lat)
            .param("lon", lon)
            .param("radius", max_radius_m);

            let mut fallback_result = self.neo4j.graph.execute(fallback_q).await?;
            if let Some(row) = fallback_result.next().await? {
                let id: String = row.get("id")?;
                let dist: f64 = row.get("dist")?;
                Ok(Some((id, dist)))
            } else {
                Ok(None)
            }
        }
    }

    /// Seed relief shelters and depot facilities for the Aluva pilot
    pub async fn seed_aluva_shelters(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cypher = r#"
            MERGE (s1:Shelter {id: 'S_ALUVA_TOWNHALL'})
            SET s1.name = 'Aluva Town Hall Relief Hub',
                s1.junction_id = 'node/4664235729',
                s1.capacity = 250,
                s1.current_occupancy = 0,
                s1.ambulances_available = 6,
                s1.trucks_available = 4,
                s1.boats_available = 0,
                s1.dataset = 'aluva-periyar-pilot',
                s1.last_updated = datetime()

            MERGE (s2:Shelter {id: 'S_UC_COLLEGE'})
            SET s2.name = 'UC College Campus Relief Camp',
                s2.junction_id = 'node/9903123560',
                s2.capacity = 600,
                s2.current_occupancy = 0,
                s2.ambulances_available = 3,
                s2.trucks_available = 8,
                s2.boats_available = 0,
                s2.dataset = 'aluva-periyar-pilot',
                s2.last_updated = datetime()

            MERGE (s3:Shelter {id: 'S_MANAPPURAM_DEPOT'})
            SET s3.name = 'Periyar Riverside Rescue Boat Depot',
                s3.junction_id = 'node/7048449098',
                s3.capacity = 50,
                s3.current_occupancy = 0,
                s3.ambulances_available = 2,
                s3.trucks_available = 2,
                s3.boats_available = 10,
                s3.dataset = 'aluva-periyar-pilot',
                s3.last_updated = datetime()

            MERGE (s4:Shelter {id: 'S_TALUK_HOSPITAL'})
            SET s4.name = 'Aluva Taluk Hospital Medical Outpost',
                s4.junction_id = 'node/343716109',
                s4.capacity = 120,
                s4.current_occupancy = 0,
                s4.ambulances_available = 8,
                s4.trucks_available = 0,
                s4.boats_available = 0,
                s4.dataset = 'aluva-periyar-pilot',
                s4.last_updated = datetime()
        "#;

        self.neo4j.graph.run(query(cypher)).await?;
        Ok(())
    }

    /// Independent post-routing safety verifier:
    /// Checks every traversed segment ID against current operational hazard timestamps in Neo4j.
    /// Returns a list of any hazardous or compromised segment IDs.
    pub async fn verify_path_segments(
        &self,
        segment_ids: &[String],
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        if segment_ids.is_empty() {
            return Ok(Vec::new());
        }

        let q = query(
            "UNWIND $ids AS seg_id \
             MATCH ()-[r:CONNECTS_TO {segment_id: seg_id}]->() \
             WHERE r.baseline_status = 'UNSUITABLE' \
                OR (r.operational_status IS NOT NULL AND r.operational_status <> 'OPEN' AND r.valid_until > datetime()) \
             RETURN seg_id"
        )
        .param("ids", segment_ids.to_vec());

        let mut result = self.neo4j.graph.execute(q).await?;
        let mut hazardous = Vec::new();
        while let Some(row) = result.next().await? {
            let seg_id: String = row.get("seg_id")?;
            hazardous.push(seg_id);
        }

        Ok(hazardous)
    }

    /// Query available shelters capable of accommodating the headcount and asset requirement
    pub async fn find_candidate_shelters(&self, asset_type: &AssetType, headcount: u32) -> Result<Vec<Shelter>, Box<dyn std::error::Error + Send + Sync>> {
        let asset_filter = match asset_type {
            AssetType::RescueBoat => "s.boats_available >= 1",
            AssetType::Ambulance => "s.ambulances_available >= 1",
            AssetType::EvacTruck => "s.trucks_available >= 1",
            AssetType::Helicopter => "s.capacity - s.current_occupancy >= $headcount",
        };

        let cypher = format!(
            "MATCH (s:Shelter) \
             WHERE (s.capacity - s.current_occupancy) >= $headcount AND {} \
             RETURN s.id AS id, s.name AS name, s.junction_id AS junction_id, \
                    s.capacity AS capacity, s.current_occupancy AS current_occupancy, \
                    s.boats_available AS boats_available, s.ambulances_available AS ambulances_available, \
                    s.trucks_available AS trucks_available \
             ORDER BY (s.capacity - s.current_occupancy) DESC",
            asset_filter
        );

        let q = query(&cypher).param("headcount", headcount as i64);
        let mut result = self.neo4j.graph.execute(q).await?;
        let mut shelters = Vec::new();

        while let Some(row) = result.next().await? {
            let id: String = row.get("id")?;
            let name: String = row.get("name")?;
            let junction_id: String = row.get("junction_id")?;
            let capacity: i64 = row.get("capacity")?;
            let current_occupancy: i64 = row.get("current_occupancy")?;
            let boats_available: i64 = row.get("boats_available")?;
            let ambulances_available: i64 = row.get("ambulances_available")?;
            let trucks_available: i64 = row.get("trucks_available")?;

            shelters.push(Shelter {
                id,
                name,
                junction_id,
                capacity: capacity as u32,
                current_occupancy: current_occupancy as u32,
                boats_available: boats_available as u32,
                ambulances_available: ambulances_available as u32,
                trucks_available: trucks_available as u32,
            });
        }

        Ok(shelters)
    }

    /// Atomically reserve capacity and asset in Neo4j
    pub async fn reserve_shelter_asset(
        &self,
        shelter_id: &str,
        asset_type: &AssetType,
        headcount: u32,
    ) -> Result<Option<Shelter>, Box<dyn std::error::Error + Send + Sync>> {
        let asset_decrement = match asset_type {
            AssetType::RescueBoat => "s.boats_available = s.boats_available - 1,",
            AssetType::Ambulance => "s.ambulances_available = s.ambulances_available - 1,",
            AssetType::EvacTruck => "s.trucks_available = s.trucks_available - 1,",
            AssetType::Helicopter => "",
        };

        let cypher = format!(
            "MATCH (s:Shelter {{id: $shelter_id}}) \
             WHERE (s.capacity - s.current_occupancy) >= $headcount \
             SET s.current_occupancy = s.current_occupancy + $headcount, \
                 {} \
                 s.last_updated = datetime() \
             RETURN s.id AS id, s.name AS name, s.junction_id AS junction_id, \
                    s.capacity AS capacity, s.current_occupancy AS current_occupancy, \
                    s.boats_available AS boats_available, s.ambulances_available AS ambulances_available, \
                    s.trucks_available AS trucks_available",
            asset_decrement
        );

        let q = query(&cypher)
            .param("shelter_id", shelter_id)
            .param("headcount", headcount as i64);

        let mut result = self.neo4j.graph.execute(q).await?;
        if let Some(row) = result.next().await? {
            let id: String = row.get("id")?;
            let name: String = row.get("name")?;
            let junction_id: String = row.get("junction_id")?;
            let capacity: i64 = row.get("capacity")?;
            let current_occupancy: i64 = row.get("current_occupancy")?;
            let boats_available: i64 = row.get("boats_available")?;
            let ambulances_available: i64 = row.get("ambulances_available")?;
            let trucks_available: i64 = row.get("trucks_available")?;

            Ok(Some(Shelter {
                id,
                name,
                junction_id,
                capacity: capacity as u32,
                current_occupancy: current_occupancy as u32,
                boats_available: boats_available as u32,
                ambulances_available: ambulances_available as u32,
                trucks_available: trucks_available as u32,
            }))
        } else {
            Ok(None)
        }
    }

    /// Retrieve ordered GPS coordinates (lon, lat) for a list of junction IDs
    pub async fn get_junction_coordinates(
        &self,
        junction_ids: &[String],
    ) -> Result<Vec<[f64; 2]>, Box<dyn std::error::Error + Send + Sync>> {
        if junction_ids.is_empty() {
            return Ok(Vec::new());
        }

        let q = query(
            "UNWIND range(0, size($ids)-1) AS idx \
             WITH idx, $ids[idx] AS jid \
             MATCH (j:Junction {id: jid}) \
             RETURN idx, j.lat AS lat, j.lon AS lon \
             ORDER BY idx"
        )
        .param("ids", junction_ids.to_vec());

        let mut result = self.neo4j.graph.execute(q).await?;
        let mut coords = Vec::new();
        while let Some(row) = result.next().await? {
            let lat: f64 = row.get("lat")?;
            let lon: f64 = row.get("lon")?;
            // GeoJSON standard: [longitude, latitude]
            coords.push([lon, lat]);
        }

        Ok(coords)
    }

    /// Get all shelters enriched with their physical WGS-84 coordinates
    pub async fn get_shelters_with_locations(
        &self,
    ) -> Result<Vec<ShelterWithLocation>, Box<dyn std::error::Error + Send + Sync>> {
        let q = query(
            "MATCH (s:Shelter) \
             OPTIONAL MATCH (j:Junction {id: s.junction_id}) \
             RETURN s.id AS id, s.name AS name, s.junction_id AS junction_id, \
                    s.capacity AS capacity, s.current_occupancy AS current_occupancy, \
                    s.boats_available AS boats_available, s.ambulances_available AS ambulances_available, \
                    s.trucks_available AS trucks_available, \
                    coalesce(j.lat, 10.108) AS lat, coalesce(j.lon, 76.356) AS lon \
             ORDER BY s.id"
        );

        let mut result = self.neo4j.graph.execute(q).await?;
        let mut list = Vec::new();
        while let Some(row) = result.next().await? {
            list.push(ShelterWithLocation {
                id: row.get("id")?,
                name: row.get("name")?,
                junction_id: row.get("junction_id")?,
                capacity: row.get::<i64>("capacity")? as u32,
                current_occupancy: row.get::<i64>("current_occupancy")? as u32,
                boats_available: row.get::<i64>("boats_available")? as u32,
                ambulances_available: row.get::<i64>("ambulances_available")? as u32,
                trucks_available: row.get::<i64>("trucks_available")? as u32,
                lat: row.get("lat")?,
                lon: row.get("lon")?,
            });
        }

        Ok(list)
    }

    /// Query all active operational hazards / road closures
    pub async fn get_active_hazards(
        &self,
        dataset: &str,
    ) -> Result<Vec<ActiveHazardInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let q = query(
            "MATCH (a:Junction)-[r:CONNECTS_TO {dataset: $dataset}]->(b:Junction) \
             WHERE (r.operational_status IS NOT NULL AND r.operational_status <> 'OPEN' AND r.valid_until > datetime()) \
                OR r.baseline_status = 'FLOODED' \
             RETURN r.segment_id AS segment_id, \
                    coalesce(r.road_name, 'Unnamed Road') AS road_name, \
                    coalesce(r.operational_status, r.baseline_status) AS operational_status, \
                    a.id AS from_id, a.lat AS from_lat, a.lon AS from_lon, \
                    b.id AS to_id, b.lat AS to_lat, b.lon AS to_lon \
             LIMIT 100"
        )
        .param("dataset", dataset);

        let mut result = self.neo4j.graph.execute(q).await?;
        let mut hazards = Vec::new();
        while let Some(row) = result.next().await? {
            hazards.push(ActiveHazardInfo {
                segment_id: row.get("segment_id")?,
                road_name: row.get("road_name")?,
                operational_status: row.get("operational_status")?,
                from_id: row.get("from_id")?,
                to_id: row.get("to_id")?,
                from_lat: row.get("from_lat")?,
                from_lon: row.get("from_lon")?,
                to_lat: row.get("to_lat")?,
                to_lon: row.get("to_lon")?,
            });
        }

        Ok(hazards)
    }
}
