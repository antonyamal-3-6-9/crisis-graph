//! Operational data model (docs/internal/operational-data-model-2026-10-24.md):
//! reports, tasks, vehicles and reservations used by the new pipeline.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{AssetType, LocationSource, ReviewPriority, UrgencyResult};

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

/// Incident statuses (new pipeline) that count as "open" for deduplication.
pub const OPEN_INCIDENT_STATUSES: &[&str] = &["REVIEW_REQUIRED", "TRIAGE_APPROVED", "ALLOCATING", "ASSIGNED"];

/// What the triage worker saves on a report after extraction (checkpoint).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractionRecord {
    /// False when the model call failed or was skipped (circuit open).
    pub succeeded: bool,
    /// Full extraction output (triage report, stage errors) for audit/display.
    pub extraction_json: String,
    pub headcount: Option<u32>,
    pub required_asset: Option<AssetType>,
    pub resolved_junction_id: Option<String>,
    /// All dispatch-critical facts present and no blocking errors.
    pub dispatch_ready: bool,
    pub review_reasons: Vec<String>,
    /// Derived urgency; UNKNOWN (reviewed first) when no extraction exists.
    pub urgency: UrgencyResult,
    /// Which position `resolved_junction_id` came from (finding F-1); `None`
    /// when no victim position is known.
    pub location_source: Option<LocationSource>,
    /// Asset suggested from the extracted facts when none was given (F-2).
    /// Shown to the dispatcher; never dispatched on its own.
    pub suggested_asset: Option<AssetType>,
}

/// An open incident whose urgency is UNKNOWN: the dispatcher calls back to
/// establish the facts. Its task (if any) is already queued at URGENT rank.
#[derive(Debug, Clone, PartialEq)]
pub struct CallbackItem {
    pub incident_id: String,
    pub status: String,
    pub review_priority: Option<ReviewPriority>,
    pub created_at: DateTime<Utc>,
}

/// A report as seen by the triage worker when it starts processing it.
#[derive(Debug, Clone, PartialEq)]
pub struct TriageState {
    pub report: Report,
    /// How many times processing has started (including this one).
    pub attempts: u32,
    /// Incident the report is already linked to, if any.
    pub linked_incident_id: Option<String>,
}

/// Result of committing a triaged report (transaction T1).
#[derive(Debug, Clone, PartialEq)]
pub enum TriageCommit {
    /// The report was already linked (duplicate or redelivered message).
    AlreadyLinked { incident_id: String },
    /// A new incident was created; `task_id` is set when it is dispatch-ready.
    /// `alerted`: an `INCIDENT_CRITICAL` alert was queued for the dispatcher.
    Created { incident_id: String, task_id: Option<String>, alerted: bool },
    /// Attached as a follow-up to an open incident from the same device (D-2).
    /// `urgency_raised`: the follow-up raised the incident's urgency.
    Attached { incident_id: String, headcount_increased: bool, urgency_raised: bool },
}

impl TriageCommit {
    pub fn incident_id(&self) -> &str {
        match self {
            Self::AlreadyLinked { incident_id }
            | Self::Created { incident_id, .. }
            | Self::Attached { incident_id, .. } => incident_id,
        }
    }
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
