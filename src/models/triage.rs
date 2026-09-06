use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AssetType {
    #[serde(rename = "RESCUE_BOAT")]
    RescueBoat,
    #[serde(rename = "AMBULANCE")]
    Ambulance,
    #[serde(rename = "EVAC_TRUCK")]
    EvacTruck,
    #[serde(rename = "HELICOPTER")]
    Helicopter,
}

impl std::fmt::Display for AssetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetType::RescueBoat => write!(f, "RESCUE_BOAT"),
            AssetType::Ambulance => write!(f, "AMBULANCE"),
            AssetType::EvacTruck => write!(f, "EVAC_TRUCK"),
            AssetType::Helicopter => write!(f, "HELICOPTER"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RoadStatus {
    #[serde(rename = "OPEN")]
    Open,
    #[serde(rename = "BLOCKED")]
    Blocked,
    #[serde(rename = "FLOODED")]
    Flooded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HazardReport {
    pub road_segment: String, // e.g. "J3-J6"
    pub status: RoadStatus,
    pub duration_hours: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SosAlert {
    pub alert_id: String,
    pub raw_text: String,
    pub timestamp: DateTime<Utc>,
    pub source_channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageReport {
    pub victim_location_raw: String,
    pub resolved_junction_id: Option<String>,
    pub headcount: u32,
    pub required_asset: AssetType,
    pub hazards: Vec<HazardReport>,
    pub confidence_score: f32,
}
