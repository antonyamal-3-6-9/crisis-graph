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
    /// Hazard overlay version the task's route was computed on (M4).
    pub overlay_version: Option<u64>,
    /// The computed route (JSON), when one has been recorded.
    pub route_json: Option<String>,
}

/// A `QUEUED` task as the matcher sees it (M5): what it needs, where, how
/// urgent, and how long it has waited.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedTask {
    pub task_id: String,
    pub incident_id: String,
    pub version: u64,
    pub required_asset: AssetType,
    /// CRITICAL 2, URGENT/UNKNOWN 1, STABLE 0 (urgency-rules-v5).
    pub priority_rank: u8,
    pub queued_at: DateTime<Utc>,
    pub headcount: Option<u32>,
    /// The incident's junction in the matcher's dataset.
    pub junction_id: String,
    /// Failed live route checks so far.
    pub verify_failures: u32,
    /// Escalation reasons already raised (each is raised once).
    pub escalations: Vec<String>,
}

impl QueuedTask {
    /// People to carry: an unknown headcount counts as one.
    pub fn persons(&self) -> u32 {
        self.headcount.unwrap_or(1).max(1)
    }
}

/// A `HELD` vehicle reservation whose commit never happened (the matcher
/// stopped between hold and commit). The next cycle resumes it.
#[derive(Debug, Clone, PartialEq)]
pub struct OrphanedHold {
    pub reservation_id: String,
    pub task_id: String,
    pub vehicle_id: String,
}

/// A shelter that can take drop-offs, with its free beds.
#[derive(Debug, Clone, PartialEq)]
pub struct DropoffShelter {
    pub shelter_id: String,
    pub junction_id: String,
    pub free_beds: u32,
}

/// Incident statuses (new pipeline) that count as "open" for deduplication.
pub const OPEN_INCIDENT_STATUSES: &[&str] = &["REVIEW_REQUIRED", "TRIAGE_APPROVED", "ALLOCATING", "ASSIGNED"];

/// What the console shows for an incident (M7): its review or merge
/// state, or else the state of its tasks.
pub fn display_status(incident_status: &str, task_statuses: &[String]) -> &'static str {
    match incident_status {
        "REVIEW_REQUIRED" => return "REVIEW_REQUIRED",
        "REJECTED" => return "REJECTED",
        "MERGED" => return "MERGED",
        _ => {}
    }
    let live: Vec<&str> = task_statuses.iter().map(String::as_str).filter(|s| *s != "CANCELLED").collect();
    if live.is_empty() {
        "NEEDS_TASK"
    } else if live.contains(&"ASSIGNED") {
        "ASSIGNED"
    } else if live.iter().all(|s| *s == "COMPLETED") {
        "COMPLETED"
    } else {
        "QUEUED"
    }
}

/// The active task of an incident in a merge: assigned beats queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTask {
    None,
    Queued,
    Assigned,
}

/// What a merge does with the two incidents' tasks (owner decision
/// 2026-11-01): exactly one vehicle stays sent, and one already sent is
/// never recalled in favour of one that is only queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergePlan {
    /// The survivor keeps its task; the absorbed incident's tasks are
    /// cancelled and its vehicle (if any) released.
    KeepSurvivor,
    /// The absorbed incident's task moves to the survivor; the survivor's
    /// queued task (if any) is cancelled.
    MoveAbsorbed,
}

pub fn merge_plan(survivor: ActiveTask, absorbed: ActiveTask) -> MergePlan {
    match (survivor, absorbed) {
        (ActiveTask::Assigned, _) => MergePlan::KeepSurvivor,
        (_, ActiveTask::Assigned) => MergePlan::MoveAbsorbed,
        (ActiveTask::Queued, _) => MergePlan::KeepSurvivor,
        (ActiveTask::None, ActiveTask::Queued) => MergePlan::MoveAbsorbed,
        (ActiveTask::None, ActiveTask::None) => MergePlan::KeepSurvivor,
    }
}

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

    #[test]
    fn display_status_follows_review_then_tasks() {
        let tasks = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(display_status("REVIEW_REQUIRED", &tasks(&["QUEUED"])), "REVIEW_REQUIRED");
        assert_eq!(display_status("REJECTED", &[]), "REJECTED");
        assert_eq!(display_status("TRIAGE_APPROVED", &[]), "NEEDS_TASK");
        assert_eq!(display_status("TRIAGE_APPROVED", &tasks(&["QUEUED"])), "QUEUED");
        assert_eq!(display_status("ASSIGNED", &tasks(&["COMPLETED", "ASSIGNED"])), "ASSIGNED");
        assert_eq!(display_status("COMPLETED", &tasks(&["COMPLETED", "CANCELLED"])), "COMPLETED");
        assert_eq!(display_status("TRIAGE_APPROVED", &tasks(&["COMPLETED", "QUEUED"])), "QUEUED");
        assert_eq!(display_status("TRIAGE_APPROVED", &tasks(&["CANCELLED"])), "NEEDS_TASK");
    }

    #[test]
    fn merge_keeps_exactly_one_vehicle() {
        use ActiveTask::*;
        assert_eq!(merge_plan(Assigned, Assigned), MergePlan::KeepSurvivor, "absorbed vehicle released");
        assert_eq!(merge_plan(Assigned, Queued), MergePlan::KeepSurvivor);
        assert_eq!(merge_plan(Queued, Assigned), MergePlan::MoveAbsorbed, "a sent vehicle is never recalled");
        assert_eq!(merge_plan(None, Assigned), MergePlan::MoveAbsorbed);
        assert_eq!(merge_plan(Queued, Queued), MergePlan::KeepSurvivor);
        assert_eq!(merge_plan(None, Queued), MergePlan::MoveAbsorbed);
        assert_eq!(merge_plan(None, None), MergePlan::KeepSurvivor);
        assert_eq!(display_status("MERGED", &["ASSIGNED".to_string()]), "MERGED");
    }
}
