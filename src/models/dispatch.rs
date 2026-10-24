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

/// Purpose of a resource reservation within an incident. An incident holds at most one
/// active reservation per role.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReservationRole {
    /// The primary road vehicle (or boat) dispatched to the victim.
    #[serde(rename = "ROAD_PRIMARY")]
    RoadPrimary,
    /// A rescue boat bridging a flooded corridor in a multi-modal plan.
    #[serde(rename = "WATER_MITIGATION")]
    WaterMitigation,
}

impl ReservationRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RoadPrimary => "ROAD_PRIMARY",
            Self::WaterMitigation => "WATER_MITIGATION",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DispatchStatus {
    #[serde(rename = "ROUTED_VERIFIED")]
    RoutedVerified,
    #[serde(rename = "MITIGATION_REQUIRED")]
    MitigationRequired,
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
    #[serde(default)]
    pub segment_path: Vec<String>,
    pub total_distance_km: f64,
    #[serde(default)]
    pub total_travel_time_s: f64,
    pub detour_reason: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub raw_brief_text: String,
    pub multimodal_plan: Option<crate::solver::MultiModalPlan>,
    pub uncertainty_waterfall: Option<super::waterfall::UncertaintyWaterfall>,
}
