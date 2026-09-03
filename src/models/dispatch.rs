use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shelter {
    pub id: String,
    pub name: String,
    pub junction_id: String,
    pub capacity: u32,
    pub current_occupancy: u32,
    pub boats_available: u32,
    pub ambulances_available: u32,
    pub trucks_available: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DispatchStatus {
    #[serde(rename = "ROUTED_VERIFIED")]
    RoutedVerified,
    #[serde(rename = "ESCALATE_HUMAN_DISPATCHER")]
    EscalateHumanDispatcher,
    #[serde(rename = "RESOURCES_EXHAUSTED")]
    ResourcesExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TacticalBrief {
    pub incident_id: String,
    pub status: DispatchStatus,
    pub target_victim_junction: String,
    pub assigned_shelter_id: String,
    pub assigned_shelter_junction: String,
    pub assigned_asset: String,
    pub headcount: u32,
    pub computed_route: Vec<String>,
    pub total_distance_km: f64,
    pub detour_reason: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub raw_brief_text: String,
}
