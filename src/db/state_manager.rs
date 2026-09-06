use chrono::{Duration, Utc};
use neo4rs::query;
use crate::geospatial::{AccessFlags, BaselineStatus};
use crate::models::{AssetType, HazardReport, RoadStatus, Shelter};
use crate::solver::RoutingEdge;
use super::neo4j::Neo4jClient;

#[derive(Clone)]
pub struct StateManager {
    neo4j: Neo4jClient,
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
}
