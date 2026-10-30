//! Hazard records (M4): the operational overlay over the immutable baseline
//! road graph. A hazard is its own `:Hazard` record, never a property written
//! on a road segment (requirement R-17, rule R-37).
//!
//! At most one hazard per segment is active (`active_key`, unique). A new
//! hazard on the same segment supersedes the old one; clearing ends it. Ended
//! records stay as history. A hazard is in force while it is active, has
//! started, and has not passed `valid_until`; no `valid_until` means "until
//! cleared" (fail closed).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HazardStatus {
    Flooded,
    Blocked,
}

impl HazardStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flooded => "FLOODED",
            Self::Blocked => "BLOCKED",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "FLOODED" => Some(Self::Flooded),
            "BLOCKED" => Some(Self::Blocked),
            _ => None,
        }
    }
}

/// Where a hazard came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HazardSource {
    /// An authenticated operator through the hazard command.
    Operator,
    /// The legacy console endpoint, which has no authentication.
    LegacyConsole,
    /// Moved from the old properties on road segments by `migrate_ops`.
    Migration,
}

impl HazardSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "OPERATOR",
            Self::LegacyConsole => "LEGACY_CONSOLE",
            Self::Migration => "MIGRATION",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "OPERATOR" => Some(Self::Operator),
            "LEGACY_CONSOLE" => Some(Self::LegacyConsole),
            "MIGRATION" => Some(Self::Migration),
            _ => None,
        }
    }
}

/// Close (or re-close) one directed road segment.
#[derive(Debug, Clone)]
pub struct HazardCommand {
    /// Idempotency key: a retry with the same `request_id` returns the
    /// original result and changes nothing.
    pub request_id: String,
    pub dataset: String,
    pub segment_id: String,
    pub status: HazardStatus,
    /// `None`: in force until cleared.
    pub valid_until: Option<DateTime<Utc>>,
    pub reason: String,
    pub source: HazardSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HazardRecord {
    pub hazard_id: String,
    pub request_id: Option<String>,
    pub dataset: String,
    pub segment_id: String,
    pub status: HazardStatus,
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
    pub actor_id: String,
    pub reason: String,
    pub source: HazardSource,
    /// Overlay version this record was created at.
    pub overlay_version: u64,
}

impl HazardRecord {
    pub fn in_force_at(&self, now: DateTime<Utc>) -> bool {
        in_force(self.valid_from, self.valid_until, now)
    }
}

/// The shared rule for whether an active hazard applies at `now`.
pub fn in_force(valid_from: DateTime<Utc>, valid_until: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    valid_from <= now && valid_until.is_none_or(|until| until > now)
}

#[derive(Debug, Clone, PartialEq)]
pub enum HazardOutcome {
    /// A new hazard was recorded; `replaced` is the hazard it superseded.
    Applied { hazard: HazardRecord, overlay_version: u64, replaced: Option<String> },
    /// Same `request_id` as an earlier command: nothing changed.
    Replayed { hazard: HazardRecord },
    /// The segment's active hazard was ended.
    Cleared { hazard_id: String, overlay_version: u64 },
    /// No active hazard on the segment: nothing changed.
    NothingToClear,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn in_force_respects_start_expiry_and_open_ended() {
        let now = Utc::now();
        assert!(in_force(now - Duration::minutes(1), None, now), "open-ended stays in force");
        assert!(in_force(now - Duration::minutes(1), Some(now + Duration::hours(1)), now));
        assert!(!in_force(now - Duration::hours(2), Some(now - Duration::hours(1)), now), "expired");
        assert!(!in_force(now + Duration::minutes(5), None, now), "not started");
    }
}
