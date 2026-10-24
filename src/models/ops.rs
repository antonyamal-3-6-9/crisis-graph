//! Operational data model (docs/internal/operational-data-model-2026-10-24.md):
//! reports, tasks, vehicles and reservations used by the new pipeline.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::AssetType;

/// Declares a string-backed enum with stable wire names, `as_str()` and
/// `parse()`, matching the values stored in Neo4j.
macro_rules! ops_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
        pub enum $name {
            $(#[serde(rename = $wire)] $variant),+
        }

        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }

            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($wire => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

ops_enum!(
    /// Availability of an individual vehicle. Minimal set for the core; the
    /// full mission lifecycle is out of scope.
    VehicleStatus {
        Available => "AVAILABLE",
        Held => "HELD",
        Assigned => "ASSIGNED",
        OutOfService => "OUT_OF_SERVICE",
    }
);

ops_enum!(
    /// One leg of work for an incident.
    TaskKind {
        RoadPickup => "ROAD_PICKUP",
        WaterExtraction => "WATER_EXTRACTION",
        RoadEvacuation => "ROAD_EVACUATION",
    }
);

ops_enum!(
    TaskStatus {
        PendingTrigger => "PENDING_TRIGGER",
        Queued => "QUEUED",
        Assigned => "ASSIGNED",
        Completed => "COMPLETED",
        Cancelled => "CANCELLED",
    }
);

ops_enum!(
    ReservationStatus {
        Held => "HELD",
        Committed => "COMMITTED",
        Released => "RELEASED",
        Expired => "EXPIRED",
    }
);

ops_enum!(
    ResourceKind {
        Vehicle => "VEHICLE",
        Beds => "BEDS",
    }
);

ops_enum!(
    /// Extraction checkpoint on a report (saved before deduplication).
    ReportExtractionStatus {
        Pending => "PENDING",
        Extracted => "EXTRACTED",
        Failed => "FAILED",
    }
);

/// Key stored on an active reservation; unique per task and resource kind.
pub fn active_task_key(task_id: &str, kind: ResourceKind) -> String {
    format!("{}:{}", task_id, kind.as_str())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Vehicle {
    pub vehicle_id: String,
    pub asset_type: AssetType,
    pub org: String,
    pub capacity_persons: u32,
    pub status: VehicleStatus,
    pub version: u64,
    pub home_shelter_id: String,
    pub junction_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reservation {
    pub reservation_id: String,
    pub task_id: String,
    pub incident_id: String,
    pub resource_kind: ResourceKind,
    pub vehicle_id: Option<String>,
    pub shelter_id: Option<String>,
    pub beds: u32,
    pub status: ReservationStatus,
}

/// Input for a new report arriving at the intake API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewReport {
    pub request_id: String,
    pub device_id: String,
    pub channel: String,
    pub raw_text: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy_m: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub report_id: String,
    pub request_id: String,
    pub device_id: String,
    pub channel: String,
    pub raw_text: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy_m: Option<f64>,
    pub received_at: DateTime<Utc>,
    pub extraction_status: ReportExtractionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub task_id: String,
    pub incident_id: String,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub required_asset: AssetType,
    pub version: u64,
}

/// An event waiting in (or published from) the transactional outbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutboxRecord {
    pub event_id: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub payload_json: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_through_wire_names() {
        for status in [
            VehicleStatus::Available,
            VehicleStatus::Held,
            VehicleStatus::Assigned,
            VehicleStatus::OutOfService,
        ] {
            assert_eq!(VehicleStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(TaskStatus::parse("PENDING_TRIGGER"), Some(TaskStatus::PendingTrigger));
        assert_eq!(ResourceKind::parse("BEDS"), Some(ResourceKind::Beds));
        assert_eq!(ReservationStatus::parse("bogus"), None);
        assert_eq!(
            serde_json::to_string(&TaskKind::RoadEvacuation).unwrap(),
            "\"ROAD_EVACUATION\""
        );
    }

    #[test]
    fn active_task_key_includes_resource_kind() {
        assert_eq!(active_task_key("T-1", ResourceKind::Vehicle), "T-1:VEHICLE");
        assert_ne!(
            active_task_key("T-1", ResourceKind::Vehicle),
            active_task_key("T-1", ResourceKind::Beds)
        );
    }
}
