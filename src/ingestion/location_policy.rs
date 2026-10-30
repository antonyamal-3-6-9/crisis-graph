//! Finding F-1: which position locates the victims.
//!
//! The caller's device position locates the victims only when the caller is
//! on scene. A remote caller ("my parents are trapped at Manappuram", phoned
//! from Kochi) must be routed to the described place, never to the phone.
//!
//! - `reporter_on_scene = true`: the device position, else the described place.
//!   If the device is measurably farther than the on-scene radius from a
//!   described place, the claim contradicts the phone: the described place is
//!   used and the incident goes to review (a model can mark a remote caller
//!   as on scene; distance decides, not the claim).
//! - `reporter_on_scene = false`: the described place only; if it cannot be
//!   resolved, human review.
//! - unknown (including `reporter_relation = UNDER_REVIEW`): distance decides
//!   (owner decision 2026-10-25). A device within the on-scene radius
//!   (widened by its accuracy) of the described place counts as on scene;
//!   otherwise the described place is used. With only a device position the
//!   incident keeps it for display and deduplication but goes to review,
//!   since nothing says the caller is with the victims.

use crate::models::LocationSource;

/// A position named in the message, resolved to a junction.
#[derive(Debug, Clone, PartialEq)]
pub struct DescribedFix {
    pub junction_id: String,
    pub source: LocationSource,
}

/// The caller's device position, snapped to a junction.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceFix {
    pub junction_id: String,
    pub accuracy_m: f64,
    /// Distance from the device to the described junction, when both exist
    /// and the described junction has coordinates.
    pub distance_to_described_m: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocationDecision {
    pub junction_id: Option<String>,
    pub source: Option<LocationSource>,
    /// Set when the position cannot be used for automatic dispatch.
    pub review_reason: Option<String>,
}

pub const REVIEW_REMOTE_UNRESOLVED: &str =
    "Reporter is not on scene and the described location could not be resolved";
pub const REVIEW_ON_SCENE_CONTRADICTED: &str =
    "Reporter marked on scene but the caller's position is far from the described location";
pub const REVIEW_ON_SCENE_UNCONFIRMED: &str =
    "Only the caller's position is known and the reporter is not confirmed on scene";

pub fn choose_location(
    on_scene: Option<bool>,
    described: Option<DescribedFix>,
    device: Option<DeviceFix>,
    on_scene_radius_m: f64,
) -> LocationDecision {
    let use_device = |d: DeviceFix| LocationDecision {
        junction_id: Some(d.junction_id),
        source: Some(LocationSource::DeviceGps),
        review_reason: None,
    };
    let use_described = |d: DescribedFix| LocationDecision {
        junction_id: Some(d.junction_id),
        source: Some(d.source),
        review_reason: None,
    };
    let review = |reason: &str| LocationDecision {
        junction_id: None,
        source: None,
        review_reason: Some(reason.to_string()),
    };

    let far = |d: &DeviceFix| {
        d.distance_to_described_m.is_some_and(|m| m > on_scene_radius_m + d.accuracy_m.max(0.0))
    };

    match on_scene {
        Some(true) => match (device, described) {
            (Some(device), Some(described)) if far(&device) => LocationDecision {
                review_reason: Some(REVIEW_ON_SCENE_CONTRADICTED.to_string()),
                ..use_described(described)
            },
            (Some(device), _) => use_device(device),
            (None, Some(described)) => use_described(described),
            // The extractor already requires review when no location is given.
            (None, None) => LocationDecision { junction_id: None, source: None, review_reason: None },
        },
        Some(false) => match described {
            Some(described) => use_described(described),
            None => review(REVIEW_REMOTE_UNRESOLVED),
        },
        None => match (device, described) {
            (Some(device), Some(described)) => {
                let near = device
                    .distance_to_described_m
                    .is_some_and(|d| d <= on_scene_radius_m + device.accuracy_m.max(0.0));
                if near { use_device(device) } else { use_described(described) }
            }
            (None, Some(described)) => use_described(described),
            (Some(device), None) => LocationDecision {
                review_reason: Some(REVIEW_ON_SCENE_UNCONFIRMED.to_string()),
                ..use_device(device)
            },
            (None, None) => LocationDecision { junction_id: None, source: None, review_reason: None },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RADIUS: f64 = 500.0;

    fn described() -> Option<DescribedFix> {
        Some(DescribedFix { junction_id: "MANAPPURAM".into(), source: LocationSource::Described })
    }

    fn device(distance: Option<f64>) -> Option<DeviceFix> {
        Some(DeviceFix { junction_id: "PHONE".into(), accuracy_m: 20.0, distance_to_described_m: distance })
    }

    #[test]
    fn remote_third_party_is_routed_to_the_described_place() {
        let decision = choose_location(Some(false), described(), device(Some(20_000.0)), RADIUS);
        assert_eq!(decision.junction_id.as_deref(), Some("MANAPPURAM"));
        assert_eq!(decision.source, Some(LocationSource::Described));
        assert!(decision.review_reason.is_none());
    }

    #[test]
    fn remote_caller_without_a_described_place_goes_to_review_not_to_the_phone() {
        let decision = choose_location(Some(false), None, device(None), RADIUS);
        assert_eq!(decision.junction_id, None);
        assert_eq!(decision.review_reason.as_deref(), Some(REVIEW_REMOTE_UNRESOLVED));
    }

    #[test]
    fn on_scene_caller_uses_the_device() {
        let decision = choose_location(Some(true), described(), device(Some(300.0)), RADIUS);
        assert_eq!(decision.junction_id.as_deref(), Some("PHONE"));
        assert_eq!(decision.source, Some(LocationSource::DeviceGps));
    }

    #[test]
    fn on_scene_claim_contradicted_by_distance_uses_the_described_place_and_review() {
        let decision = choose_location(Some(true), described(), device(Some(20_000.0)), RADIUS);
        assert_eq!(decision.junction_id.as_deref(), Some("MANAPPURAM"));
        assert_eq!(decision.review_reason.as_deref(), Some(REVIEW_ON_SCENE_CONTRADICTED));
        // Unmeasured distance does not contradict the claim.
        let unmeasured = choose_location(Some(true), described(), device(None), RADIUS);
        assert_eq!(unmeasured.source, Some(LocationSource::DeviceGps));
    }

    #[test]
    fn unknown_on_scene_is_decided_by_distance() {
        let near = choose_location(None, described(), device(Some(400.0)), RADIUS);
        assert_eq!(near.source, Some(LocationSource::DeviceGps));
        // Accuracy widens the radius: 510 m away with 20 m accuracy is near.
        let widened = choose_location(None, described(), device(Some(510.0)), RADIUS);
        assert_eq!(widened.source, Some(LocationSource::DeviceGps));
        let far = choose_location(None, described(), device(Some(5_000.0)), RADIUS);
        assert_eq!(far.junction_id.as_deref(), Some("MANAPPURAM"));
        // No distance (described junction has no coordinates): described wins.
        let unmeasured = choose_location(None, described(), device(None), RADIUS);
        assert_eq!(unmeasured.source, Some(LocationSource::Described));
    }

    #[test]
    fn unknown_on_scene_with_only_the_device_keeps_it_but_requires_review() {
        let decision = choose_location(None, None, device(None), RADIUS);
        assert_eq!(decision.junction_id.as_deref(), Some("PHONE"));
        assert_eq!(decision.review_reason.as_deref(), Some(REVIEW_ON_SCENE_UNCONFIRMED));
    }
}
