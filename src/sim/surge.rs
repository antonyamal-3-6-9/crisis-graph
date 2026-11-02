//! Surge load-test scenarios (M8): deterministic report generation, and the
//! cleanup that removes a run. Posting reports, sampling queues and reading
//! results are in `src/bin/surge.rs`. Generation is pure and seeded, so a
//! scenario and seed reproduce the same reports.

use neo4rs::query;
use serde::{Deserialize, Serialize};

use crate::db::{Neo4jClient, OpsRepository};

use crate::models::{ActorContext, ActorRole, AssetType, MedicalCondition, VictimProfile, VulnerableGroup, WaterLevel, WaterTrend};
use crate::triage::simulated::SimTruth;

/// One load step: `rate` reports per second for `seconds`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Step {
    pub rate: f64,
    pub seconds: u64,
}

/// Everything that defines a run, recorded with its results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Scenario {
    pub run_id: String,
    pub seed: u64,
    pub steps: Vec<Step>,
    /// Share of reports from a different device at an earlier report's place
    /// (rule D-3: flagged as possible duplicates).
    pub neighbours: f64,
    /// Share of reports repeating an earlier caller's device and place
    /// (rule D-2: attached as follow-ups).
    pub repeats: f64,
    /// Seconds into the run at which a road near a reported place floods.
    pub hazards_at: Vec<u64>,
    /// `simulated` (reports carry a ground-truth tag) or the model in use.
    pub extraction: String,
    pub notes: String,
}

impl Scenario {
    pub fn total_reports(&self) -> usize {
        self.steps.iter().map(|s| (s.rate * s.seconds as f64).round() as usize).sum()
    }

    pub fn duration_secs(&self) -> u64 {
        self.steps.iter().map(|s| s.seconds).sum()
    }
}

/// A place reports can come from: a junction and its coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub junction_id: String,
    pub lat: f64,
    pub lon: f64,
}

/// A report the run will send.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedReport {
    /// Milliseconds after the start of the run.
    pub at_ms: u64,
    pub request_id: String,
    pub device_id: String,
    pub text: String,
    pub place: Place,
    pub truth: SimTruth,
    /// Intended tier: CRITICAL, URGENT, UNKNOWN or STABLE.
    pub intended: &'static str,
    pub neighbour: bool,
    pub repeat: bool,
}

/// SplitMix64: small, deterministic, no dependency.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Plan every report of the scenario over `places` (deterministic for a seed).
pub fn plan(scenario: &Scenario, places: &[Place]) -> Vec<PlannedReport> {
    assert!(!places.is_empty(), "a scenario needs places");
    let mut rng = Rng::new(scenario.seed);
    let mut reports: Vec<PlannedReport> = Vec::with_capacity(scenario.total_reports());
    let mut offset_ms = 0u64;
    for step in &scenario.steps {
        let count = (step.rate * step.seconds as f64).round() as u64;
        let gap_ms = (step.seconds * 1000).checked_div(count).unwrap_or(0);
        for i in 0..count {
            let n = reports.len();
            let at_ms = offset_ms + i * gap_ms;
            let roll = rng.unit();
            let (place, device, neighbour, repeat) = if n > 0 && roll < scenario.repeats {
                let earlier = &reports[rng.below(n as u64) as usize];
                (earlier.place.clone(), earlier.device_id.clone(), false, true)
            } else if n > 0 && roll < scenario.repeats + scenario.neighbours {
                let earlier = &reports[rng.below(n as u64) as usize];
                (earlier.place.clone(), format!("SURGE-{}-dev-{n}", scenario.run_id), true, false)
            } else {
                let place = places[rng.below(places.len() as u64) as usize].clone();
                (place, format!("SURGE-{}-dev-{n}", scenario.run_id), false, false)
            };
            let (intended, truth, text) = victim(&mut rng, &place);
            reports.push(PlannedReport {
                at_ms,
                request_id: format!("SURGE-{}-{n:06}", scenario.run_id),
                device_id: device,
                text,
                place,
                truth,
                intended,
                neighbour,
                repeat,
            });
        }
        offset_ms += step.seconds * 1000;
    }
    reports
}

/// One victim situation: the intended tier, the ground truth and the
/// caller's words (English, Manglish or Malayalam).
fn victim(rng: &mut Rng, place: &Place) -> (&'static str, SimTruth, String) {
    let people = 1 + rng.below(6) as u32;
    let lang = rng.below(3);
    let roll = rng.below(100);
    let (intended, profile, asset, words) = if roll < 20 {
        let profile = VictimProfile {
            reporter_on_scene: Some(true),
            medical_conditions: vec![MedicalCondition::Unconscious],
            water_level: Some(WaterLevel::Chest),
            water_trend: Some(WaterTrend::Rising),
            ..Default::default()
        };
        let words = match lang {
            0 => format!("My father is unconscious and the water is at chest level and rising. We are {people} people. Please hurry."),
            1 => format!("Achan bodham illa, vellam nenju vare und, koodunnu. {people} aalukal und. Vegam varu."),
            _ => format!("അച്ഛന് ബോധമില്ല, വെള്ളം നെഞ്ചോളം ഉയരുന്നു. ഞങ്ങൾ {people} പേരുണ്ട്."),
        };
        ("CRITICAL", profile, AssetType::Ambulance, words)
    } else if roll < 65 {
        let profile = VictimProfile {
            reporter_on_scene: Some(true),
            vulnerable_groups: vec![VulnerableGroup::Elderly],
            water_level: Some(WaterLevel::Waist),
            water_trend: Some(WaterTrend::Stable),
            ..Default::default()
        };
        let asset = if people > 4 { AssetType::EvacTruck } else { AssetType::Ambulance };
        let words = match lang {
            0 => format!("Water is at waist level inside the house, my grandmother is with us, {people} of us need to get out."),
            1 => format!("Veetil vellam aravare und, ammoomma koode und, {people} peru purathu irangan pattunnilla."),
            _ => format!("വീട്ടിൽ അരയോളം വെള്ളം, അമ്മൂമ്മയും ഉണ്ട്, ഞങ്ങൾ {people} പേർ."),
        };
        ("URGENT", profile, asset, words)
    } else if roll < 85 {
        let profile = VictimProfile {
            reporter_on_scene: Some(true),
            water_level: Some(WaterLevel::Ankle),
            water_trend: Some(WaterTrend::Stable),
            ..Default::default()
        };
        let words = match lang {
            0 => format!("Water is ankle deep in the street, we are {people} people and want to move to the camp."),
            1 => format!("Roadil kaal vare vellam, {people} peru camp-ilekk pokanam."),
            _ => format!("റോഡിൽ കണങ്കാലോളം വെള്ളം, ഞങ്ങൾ {people} പേർ ക്യാമ്പിലേക്ക് പോകണം."),
        };
        ("STABLE", profile, AssetType::EvacTruck, words)
    } else {
        let profile = VictimProfile { reporter_on_scene: Some(true), ..Default::default() };
        let words = match lang {
            0 => format!("Please help, we are {people} people here, the situation is bad."),
            1 => format!("Help venam, ivide {people} peru und, situation mosham aanu."),
            _ => format!("സഹായിക്കണം, ഇവിടെ {people} പേരുണ്ട്."),
        };
        ("UNKNOWN", profile, AssetType::Ambulance, words)
    };
    let truth = SimTruth { headcount: Some(people), asset: Some(asset), junction: Some(place.junction_id.clone()), profile };
    (intended, truth, words)
}

/// The value at quantile `q` (0..=1) of `values` (sorted copy, nearest rank).
pub fn quantile(values: &[f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    Some(sorted[rank - 1])
}

// ------------------------------------------------------------------ cleanup

/// What [`cleanup_run`] removed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleanupSummary {
    pub reports: usize,
    pub incidents: usize,
    pub tasks: usize,
    pub hazards_cleared: Vec<String>,
}

/// Remove a run's records (request IDs `SURGE-<run_id>-…`) and restore the
/// dataset: beds the run still holds are returned, its vehicle reservations
/// released, vehicles not on a task moved back to their depots, and the
/// hazards it applied cleared through T7 (their history is kept).
pub async fn cleanup_run(
    neo4j: &Neo4jClient,
    repo: &OpsRepository,
    dataset: &str,
    run_id: &str,
) -> Result<CleanupSummary, Box<dyn std::error::Error + Send + Sync>> {
    let dataset = dataset.to_string();
    let prefix = format!("SURGE-{run_id}-");
    let mut result = neo4j
        .graph
        .execute(
            query(
                "MATCH (r:Report) WHERE r.request_id STARTS WITH $p \
                 OPTIONAL MATCH (r)-[:REPORTS]->(i:OpsIncident) \
                 OPTIONAL MATCH (i)-[:HAS_TASK]->(t:Task) \
                 RETURN collect(DISTINCT r.report_id) AS reports, collect(DISTINCT i.incident_id) AS incidents, \
                        collect(DISTINCT t.task_id) AS tasks",
            )
            .param("p", prefix.clone()),
        )
        .await?;
    let row = result.next().await?.ok_or("cleanup query returned no row")?;
    let (reports, incidents, tasks): (Vec<String>, Vec<String>, Vec<String>) =
        (row.get("reports")?, row.get("incidents")?, row.get("tasks")?);
    let mut ids = reports.clone();
    ids.extend(incidents.clone());
    ids.extend(tasks.clone());

    // Beds still held by the run go back; active vehicle reservations are
    // released before the records are deleted.
    for statement in [
        "MATCH (b:Reservation {resource_kind: 'BEDS'})-[:AT_SHELTER]->(s:Shelter) \
         WHERE b.task_id IN $ids AND b.status = 'COMMITTED' \
         WITH s, sum(b.beds) AS beds \
         SET s.current_occupancy = CASE WHEN s.current_occupancy >= beds THEN s.current_occupancy - beds ELSE 0 END",
        "MATCH (r:Reservation)-[:OF_VEHICLE]->(v:Vehicle) WHERE r.task_id IN $ids AND r.active_vehicle_key IS NOT NULL \
         SET v.status = 'AVAILABLE', v.version = v.version + 1",
        "MATCH (r:Reservation) WHERE r.task_id IN $ids OR r.incident_id IN $ids \
         OPTIONAL MATCH (r)-[:HAS_AUDIT_EVENT]->(a:AuditEvent) DETACH DELETE r, a",
        "MATCH (a:AuditEvent) WHERE a.aggregate_id IN $ids DETACH DELETE a",
        "MATCH (e:OutboxEvent) WHERE e.aggregate_id IN $ids DELETE e",
        "MATCH (c:Command) WHERE any(id IN $ids WHERE c.request_id CONTAINS id) DELETE c",
        "MATCH (n) WHERE (n:Task AND n.task_id IN $ids) OR (n:OpsIncident AND n.incident_id IN $ids) \
            OR (n:Report AND n.report_id IN $ids) DETACH DELETE n",
    ] {
        neo4j.graph.run(query(statement).param("ids", ids.clone())).await?;
    }
    // Vehicles not on any task go back to their depots.
    neo4j
        .graph
        .run(
            query(
                "MATCH (v:Vehicle)-[:BASED_AT]->(s:Shelter {dataset: $d}) \
                 WHERE NOT EXISTS { MATCH (r:Reservation {active_vehicle_key: v.vehicle_id}) } \
                 MATCH (j:Junction {id: s.junction_id, dataset: $d}) \
                 SET v.junction_id = s.junction_id, v.position = point({latitude: j.lat, longitude: j.lon}), \
                     v.status = CASE WHEN v.status IN ['ASSIGNED', 'HELD'] THEN 'AVAILABLE' ELSE v.status END",
            )
            .param("d", dataset.clone()),
        )
        .await?;
    // Hazards the run applied are cleared through T7 (history is kept).
    let mut result = neo4j
        .graph
        .execute(
            query("MATCH (h:Hazard {dataset: $d}) WHERE h.request_id STARTS WITH $p AND h.active_key IS NOT NULL RETURN h.segment_id AS s")
                .param("d", dataset.clone())
                .param("p", format!("{prefix}HZ-")),
        )
        .await?;
    let mut cleared = Vec::new();
    let actor = ActorContext::new("surge-tool", [ActorRole::HazardVerifier]);
    while let Some(row) = result.next().await? {
        let segment: String = row.get("s")?;
        repo.clear_hazard(&actor, &dataset, &segment, &format!("surge {run_id} cleanup")).await?;
        cleared.push(segment);
    }
    Ok(CleanupSummary { reports: reports.len(), incidents: incidents.len(), tasks: tasks.len(), hazards_cleared: cleared })
}


#[cfg(test)]
mod tests {
    use super::*;

    fn scenario() -> Scenario {
        Scenario {
            run_id: "t".into(),
            seed: 7,
            steps: vec![Step { rate: 2.0, seconds: 50 }, Step { rate: 5.0, seconds: 20 }],
            neighbours: 0.1,
            repeats: 0.1,
            hazards_at: vec![],
            extraction: "simulated".into(),
            notes: String::new(),
        }
    }

    fn places() -> Vec<Place> {
        (0..50).map(|i| Place { junction_id: format!("node/{i}"), lat: 10.0 + i as f64 * 0.001, lon: 76.3 }).collect()
    }

    #[test]
    fn plan_is_deterministic_and_follows_the_mix() {
        let a = plan(&scenario(), &places());
        let b = plan(&scenario(), &places());
        assert_eq!(a, b, "same seed, same reports");
        assert_eq!(a.len(), 200);
        assert_eq!(a[100].at_ms, 50_000, "second step starts at 50 s");
        let neighbours = a.iter().filter(|r| r.neighbour).count();
        let repeats = a.iter().filter(|r| r.repeat).count();
        assert!((10..=35).contains(&neighbours), "~10% neighbours: {neighbours}");
        assert!((10..=35).contains(&repeats), "~10% repeat callers: {repeats}");
        for r in a.iter().filter(|r| r.repeat) {
            assert!(a.iter().any(|o| !o.repeat && o.device_id == r.device_id && o.place == r.place));
        }
        let critical = a.iter().filter(|r| r.intended == "CRITICAL").count();
        assert!((20..=60).contains(&critical), "~20% critical: {critical}");
        let other = plan(&Scenario { seed: 8, ..scenario() }, &places());
        assert_ne!(a, other);
    }

    #[test]
    fn quantile_uses_nearest_rank() {
        let values = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(quantile(&values, 0.5), Some(3.0));
        assert_eq!(quantile(&values, 0.95), Some(5.0));
        assert_eq!(quantile(&[], 0.5), None);
    }
}
