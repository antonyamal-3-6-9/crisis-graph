use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::{SosAlert, TriageReport};

/// Persisted lifecycle of an SOS incident. These values are API/storage
/// contracts; renaming one requires an explicit migration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IncidentStatus {
    Received,
    TriageExtracted,
    ReviewRequired,
    TriageApproved,
    Allocating,
    Routing,
    Verifying,
    RouteVerified,
    Assigned,
    Acknowledged,
    EnRoute,
    Arrived,
    Completed,
    RouteInvalidated,
    Rejected,
    Cancelled,
    Closed,
}

impl IncidentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Received => "RECEIVED",
            Self::TriageExtracted => "TRIAGE_EXTRACTED",
            Self::ReviewRequired => "REVIEW_REQUIRED",
            Self::TriageApproved => "TRIAGE_APPROVED",
            Self::Allocating => "ALLOCATING",
            Self::Routing => "ROUTING",
            Self::Verifying => "VERIFYING",
            Self::RouteVerified => "ROUTE_VERIFIED",
            Self::Assigned => "ASSIGNED",
            Self::Acknowledged => "ACKNOWLEDGED",
            Self::EnRoute => "EN_ROUTE",
            Self::Arrived => "ARRIVED",
            Self::Completed => "COMPLETED",
            Self::RouteInvalidated => "ROUTE_INVALIDATED",
            Self::Rejected => "REJECTED",
            Self::Cancelled => "CANCELLED",
            Self::Closed => "CLOSED",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Rejected | Self::Cancelled | Self::Closed
        )
    }

    pub fn allows_transition_to(self, next: Self) -> bool {
        use IncidentStatus::*;

        matches!(
            (self, next),
            (Received, TriageExtracted)
                | (Received, ReviewRequired)
                | (TriageExtracted, ReviewRequired)
                | (TriageExtracted, TriageApproved)
                | (ReviewRequired, TriageApproved)
                | (ReviewRequired, Rejected)
                | (ReviewRequired, Closed)
                | (TriageApproved, Allocating)
                | (Allocating, Routing)
                | (Allocating, ReviewRequired)
                | (Routing, Verifying)
                | (Routing, ReviewRequired)
                | (Verifying, RouteVerified)
                | (Verifying, ReviewRequired)
                | (RouteVerified, Assigned)
                | (RouteVerified, RouteInvalidated)
                | (Assigned, Acknowledged)
                | (Assigned, RouteInvalidated)
                | (Assigned, Cancelled)
                | (Acknowledged, EnRoute)
                | (Acknowledged, RouteInvalidated)
                | (Acknowledged, Cancelled)
                | (EnRoute, Arrived)
                | (EnRoute, RouteInvalidated)
                | (EnRoute, Cancelled)
                | (Arrived, Completed)
                | (Arrived, Cancelled)
                | (RouteInvalidated, Routing)
                | (RouteInvalidated, ReviewRequired)
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActorRole {
    Observer,
    Dispatcher,
    HazardVerifier,
    Responder,
    EocAdmin,
    SystemService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ReadOperations,
    IngestIncident,
    AdvancePipeline,
    ReviewIncident,
    AssignDispatch,
    AcknowledgeAssignment,
    UpdateResponderStatus,
    CloseIncident,
    VerifyHazard,
    AdministerSystem,
}

/// Trusted identity produced by future authentication middleware after it has
/// validated a token. Handlers must never build this from unverified headers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorContext {
    pub actor_id: String,
    pub roles: BTreeSet<ActorRole>,
}

impl ActorContext {
    pub fn new(actor_id: impl Into<String>, roles: impl IntoIterator<Item = ActorRole>) -> Self {
        Self {
            actor_id: actor_id.into(),
            roles: roles.into_iter().collect(),
        }
    }

    pub fn can(&self, permission: Permission) -> bool {
        use ActorRole::*;
        use Permission::*;

        self.roles.iter().any(|role| match role {
            EocAdmin => true,
            Observer => matches!(permission, ReadOperations),
            Dispatcher => matches!(
                permission,
                ReadOperations | ReviewIncident | AssignDispatch | CloseIncident
            ),
            HazardVerifier => matches!(permission, ReadOperations | VerifyHazard),
            Responder => matches!(
                permission,
                ReadOperations | AcknowledgeAssignment | UpdateResponderStatus
            ),
            SystemService => matches!(permission, IngestIncident | AdvancePipeline),
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TriageRevisionSource {
    Model,
    Dispatcher,
}

/// An immutable snapshot. Human corrections append a revision instead of
/// overwriting the model extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageRevision {
    pub revision: u64,
    pub source: TriageRevisionSource,
    pub triage: TriageReport,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewDecisionKind {
    Approve,
    Reject,
    RequestChanges,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewDecision {
    pub decision_id: String,
    pub incident_id: String,
    pub decision: ReviewDecisionKind,
    pub reviewer_id: String,
    pub reason: String,
    pub resulting_triage_revision: Option<u64>,
    pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub incident_id: String,
    pub actor_id: String,
    pub action: String,
    pub from_status: IncidentStatus,
    pub to_status: IncidentStatus,
    pub previous_version: u64,
    pub new_version: u64,
    pub reason: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub incident_id: String,
    pub original_sos: SosAlert,
    pub status: IncidentStatus,
    pub version: u64,
    pub assigned_reviewer_id: Option<String>,
    pub triage_revisions: Vec<TriageRevision>,
    pub review_decisions: Vec<ReviewDecision>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Incident {
    pub fn receive(
        original_sos: SosAlert,
        actor: &ActorContext,
        now: DateTime<Utc>,
    ) -> Result<(Self, AuditEvent), ControlPlaneError> {
        if actor.actor_id.trim().is_empty() {
            return Err(ControlPlaneError::InvalidActor);
        }
        if !actor.can(Permission::IngestIncident) {
            return Err(ControlPlaneError::Forbidden {
                actor_id: actor.actor_id.clone(),
                permission: Permission::IngestIncident,
            });
        }
        if original_sos.alert_id.trim().is_empty() {
            return Err(ControlPlaneError::InvalidIncidentId);
        }

        let incident = Self {
            incident_id: original_sos.alert_id.clone(),
            original_sos,
            status: IncidentStatus::Received,
            version: 1,
            assigned_reviewer_id: None,
            triage_revisions: Vec::new(),
            review_decisions: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        let audit = AuditEvent {
            event_id: Uuid::new_v4().to_string(),
            incident_id: incident.incident_id.clone(),
            actor_id: actor.actor_id.clone(),
            action: "INCIDENT_RECEIVED".to_string(),
            from_status: IncidentStatus::Received,
            to_status: IncidentStatus::Received,
            previous_version: 0,
            new_version: 1,
            reason: "SOS received".to_string(),
            occurred_at: now,
        };
        Ok((incident, audit))
    }

    pub fn current_triage(&self) -> Option<&TriageReport> {
        self.triage_revisions
            .last()
            .map(|revision| &revision.triage)
    }

    pub fn append_model_extraction(
        &mut self,
        actor: &ActorContext,
        expected_version: u64,
        triage: TriageReport,
        now: DateTime<Utc>,
    ) -> Result<AuditEvent, ControlPlaneError> {
        self.require_permission(actor, Permission::AdvancePipeline)?;
        self.require_version(expected_version)?;
        if self.status != IncidentStatus::Received {
            return Err(ControlPlaneError::InvalidTransition {
                from: self.status,
                to: IncidentStatus::TriageExtracted,
            });
        }

        self.triage_revisions.push(TriageRevision {
            revision: 1,
            source: TriageRevisionSource::Model,
            triage,
            created_at: now,
            created_by: actor.actor_id.clone(),
            reason: "Initial model extraction".to_string(),
        });
        self.apply_transition(
            actor,
            IncidentStatus::TriageExtracted,
            "Model extraction recorded",
            now,
        )
    }

    pub fn transition(
        &mut self,
        actor: &ActorContext,
        expected_version: u64,
        next: IncidentStatus,
        reason: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<AuditEvent, ControlPlaneError> {
        self.require_version(expected_version)?;
        if self.status == IncidentStatus::ReviewRequired
            && matches!(
                next,
                IncidentStatus::TriageApproved | IncidentStatus::Rejected
            )
        {
            return Err(ControlPlaneError::DedicatedReviewActionRequired);
        }
        let permission = permission_for_transition(self.status, next)?;
        self.require_permission(actor, permission)?;
        self.apply_transition(actor, next, reason, now)
    }

    pub fn apply_review(
        &mut self,
        actor: &ActorContext,
        expected_version: u64,
        decision: ReviewDecisionKind,
        reason: impl Into<String>,
        reviewed_triage: Option<TriageReport>,
        now: DateTime<Utc>,
    ) -> Result<AuditEvent, ControlPlaneError> {
        self.require_version(expected_version)?;
        self.require_permission(actor, Permission::ReviewIncident)?;
        if self.status != IncidentStatus::ReviewRequired {
            return Err(ControlPlaneError::ReviewNotAllowed(self.status));
        }

        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(ControlPlaneError::EmptyReason);
        }

        let resulting_triage_revision = match decision {
            ReviewDecisionKind::Approve => {
                let triage = reviewed_triage.ok_or(ControlPlaneError::ReviewedTriageRequired)?;
                if !triage_is_dispatch_ready(&triage) {
                    return Err(ControlPlaneError::ReviewedTriageNotDispatchReady);
                }
                Some(self.append_dispatcher_revision(actor, triage, &reason, now))
            }
            ReviewDecisionKind::Reject => {
                if reviewed_triage.is_some() {
                    return Err(ControlPlaneError::UnexpectedReviewedTriage);
                }
                None
            }
            ReviewDecisionKind::RequestChanges => {
                if reviewed_triage.is_some() {
                    return Err(ControlPlaneError::UnexpectedReviewedTriage);
                }
                None
            }
        };

        let next = match decision {
            ReviewDecisionKind::Approve => IncidentStatus::TriageApproved,
            ReviewDecisionKind::Reject => IncidentStatus::Rejected,
            ReviewDecisionKind::RequestChanges => IncidentStatus::ReviewRequired,
        };

        let previous_status = self.status;
        let previous_version = self.version;
        self.version += 1;
        self.status = next;
        self.updated_at = now;
        self.review_decisions.push(ReviewDecision {
            decision_id: Uuid::new_v4().to_string(),
            incident_id: self.incident_id.clone(),
            decision,
            reviewer_id: actor.actor_id.clone(),
            reason: reason.clone(),
            resulting_triage_revision,
            decided_at: now,
        });

        let action = match decision {
            ReviewDecisionKind::Approve => "REVIEW_APPROVE",
            ReviewDecisionKind::Reject => "REVIEW_REJECT",
            ReviewDecisionKind::RequestChanges => "REVIEW_REQUEST_CHANGES",
        };

        Ok(AuditEvent {
            event_id: Uuid::new_v4().to_string(),
            incident_id: self.incident_id.clone(),
            actor_id: actor.actor_id.clone(),
            action: action.to_string(),
            from_status: previous_status,
            to_status: next,
            previous_version,
            new_version: self.version,
            reason,
            occurred_at: now,
        })
    }

    fn append_dispatcher_revision(
        &mut self,
        actor: &ActorContext,
        triage: TriageReport,
        reason: &str,
        now: DateTime<Utc>,
    ) -> u64 {
        let revision = self
            .triage_revisions
            .last()
            .map_or(1, |current| current.revision + 1);
        self.triage_revisions.push(TriageRevision {
            revision,
            source: TriageRevisionSource::Dispatcher,
            triage,
            created_at: now,
            created_by: actor.actor_id.clone(),
            reason: reason.to_string(),
        });
        revision
    }

    fn apply_transition(
        &mut self,
        actor: &ActorContext,
        next: IncidentStatus,
        reason: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<AuditEvent, ControlPlaneError> {
        if !self.status.allows_transition_to(next) {
            return Err(ControlPlaneError::InvalidTransition {
                from: self.status,
                to: next,
            });
        }
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(ControlPlaneError::EmptyReason);
        }

        let from = self.status;
        let previous_version = self.version;
        self.status = next;
        self.version += 1;
        self.updated_at = now;

        Ok(AuditEvent {
            event_id: Uuid::new_v4().to_string(),
            incident_id: self.incident_id.clone(),
            actor_id: actor.actor_id.clone(),
            action: format!("TRANSITION_{}_TO_{}", from.as_str(), next.as_str()),
            from_status: from,
            to_status: next,
            previous_version,
            new_version: self.version,
            reason,
            occurred_at: now,
        })
    }

    fn require_version(&self, expected_version: u64) -> Result<(), ControlPlaneError> {
        if self.version != expected_version {
            return Err(ControlPlaneError::VersionConflict {
                expected: expected_version,
                actual: self.version,
            });
        }
        Ok(())
    }

    fn require_permission(
        &self,
        actor: &ActorContext,
        permission: Permission,
    ) -> Result<(), ControlPlaneError> {
        if actor.actor_id.trim().is_empty() {
            return Err(ControlPlaneError::InvalidActor);
        }
        if !actor.can(permission) {
            return Err(ControlPlaneError::Forbidden {
                actor_id: actor.actor_id.clone(),
                permission,
            });
        }
        Ok(())
    }
}

pub fn triage_is_dispatch_ready(triage: &TriageReport) -> bool {
    !triage.needs_human_review
        && triage.uncertainty_reasons.is_empty()
        && triage.resolved_junction_id.is_some()
        && triage.headcount.is_some()
        && triage.required_asset.is_some()
        && triage.hazards.iter().all(|hazard| {
            hazard.road_segment.is_some()
                && hazard.status.is_some()
                && hazard.duration_hours.is_some()
        })
}

fn permission_for_transition(
    from: IncidentStatus,
    to: IncidentStatus,
) -> Result<Permission, ControlPlaneError> {
    use IncidentStatus::*;

    if !from.allows_transition_to(to) {
        return Err(ControlPlaneError::InvalidTransition { from, to });
    }

    let permission = match (from, to) {
        (ReviewRequired, TriageApproved | Rejected) => Permission::ReviewIncident,
        (ReviewRequired, Closed) => Permission::CloseIncident,
        (RouteVerified, Assigned) => Permission::AssignDispatch,
        (Assigned, Acknowledged) => Permission::AcknowledgeAssignment,
        (Acknowledged, EnRoute) | (EnRoute, Arrived) | (Arrived, Completed) => {
            Permission::UpdateResponderStatus
        }
        (Assigned | Acknowledged | EnRoute | Arrived, Cancelled) => Permission::CloseIncident,
        _ => Permission::AdvancePipeline,
    };
    Ok(permission)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControlPlaneError {
    #[error("incident version conflict: expected {expected}, current version is {actual}")]
    VersionConflict { expected: u64, actual: u64 },
    #[error("invalid incident transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: IncidentStatus,
        to: IncidentStatus,
    },
    #[error("actor '{actor_id}' lacks permission {permission:?}")]
    Forbidden {
        actor_id: String,
        permission: Permission,
    },
    #[error("actor identity cannot be empty")]
    InvalidActor,
    #[error("incident ID cannot be empty")]
    InvalidIncidentId,
    #[error("transition/review reason cannot be empty")]
    EmptyReason,
    #[error("review is not allowed while incident is {0:?}")]
    ReviewNotAllowed(IncidentStatus),
    #[error("an approved review requires a reviewed triage revision")]
    ReviewedTriageRequired,
    #[error("reviewed triage is not complete and dispatch-ready")]
    ReviewedTriageNotDispatchReady,
    #[error("reviewed triage is only accepted with an approval decision")]
    UnexpectedReviewedTriage,
    #[error("review approval/rejection must use the dedicated review action")]
    DedicatedReviewActionRequired,
}
