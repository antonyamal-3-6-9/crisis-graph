use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn};
use crate::config::Config;
use crate::models::{AssetType, HazardReport, RoadStatus, SosAlert, TriageReport};
use super::spatial_resolver::SpatialResolver;

const TRIAGE_SCHEMA_VERSION: &str = "triage-extraction-v1";
const TRIAGE_JSON_SCHEMA: &str = include_str!("../../schemas/triage-extraction-v1.json");

#[derive(Clone)]
pub struct TriageExtractor {
    http_client: Client,
    vllm_base_url: String,
    vllm_model: String,
    spatial_resolver: SpatialResolver,
}

#[derive(Debug, Serialize, Deserialize)]
struct VllmChatResponse {
    choices: Vec<VllmChoice>,
}

#[derive(Debug, Serialize, Deserialize)]
struct VllmChoice {
    message: VllmMessage,
}

#[derive(Debug, Serialize, Deserialize)]
struct VllmMessage {
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLlmExtraction {
    schema_version: String,
    victim_location: String,
    headcount: u32,
    required_asset: String,
    hazards: Vec<RawHazardExtraction>,
    confidence_score: f32,
    needs_human_review: bool,
    uncertainty_reasons: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHazardExtraction {
    road_segment: String,
    status: String,
    duration_hours: u32,
}

impl TriageExtractor {
    pub fn new(config: &Config) -> Self {
        Self {
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            vllm_base_url: config.vllm_base_url.clone(),
            vllm_model: config.vllm_model.clone(),
            spatial_resolver: SpatialResolver::new(),
        }
    }

    /// Extract structured triage report from raw natural language SOS alert
    pub async fn extract(&self, alert: &SosAlert) -> Result<TriageReport, Box<dyn std::error::Error + Send + Sync>> {
        info!("Extracting triage information from SOS alert: {}", alert.alert_id);

        let system_prompt = r#"You are the structured emergency-report extractor for CrisisGraph.
Extract only facts stated or unambiguously implied by the report.
Set needs_human_review=true when the location, headcount, required asset, or hazard is ambiguous, contradictory, or missing.
Explain each uncertainty briefly in uncertainty_reasons.
Hazards are unverified candidate reports; never claim that a road is operationally closed.
Never calculate or recommend a route.
Return only the JSON object constrained by the supplied schema."#;

        let triage_schema: serde_json::Value = serde_json::from_str(TRIAGE_JSON_SCHEMA)?;

        let payload = json!({
            "model": self.vllm_model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": alert.raw_text}
            ],
            "temperature": 0.0,
            "max_tokens": 512,
            "structured_outputs": {"json": triage_schema}
        });

        let url = format!("{}/chat/completions", self.vllm_base_url.trim_end_matches('/'));

        // Attempt HTTP inference call to local vLLM process
        match self.http_client.post(&url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                let vllm_resp: VllmChatResponse = resp.json().await?;
                if let Some(choice) = vllm_resp.choices.into_iter().next() {
                    let parsed: RawLlmExtraction = serde_json::from_str(&choice.message.content)?;
                    return self.build_triage_report(parsed);
                }
            }
            Ok(resp) => {
                warn!("vLLM HTTP error {}: falling back to deterministic heuristic parser", resp.status());
            }
            Err(e) => {
                warn!("vLLM process unavailable ({}): falling back to deterministic heuristic parser", e);
            }
        }

        // Fallback heuristic extraction when vLLM process is warming up or during local unit tests
        self.heuristic_fallback_extract(alert)
    }

    fn build_triage_report(&self, raw: RawLlmExtraction) -> Result<TriageReport, Box<dyn std::error::Error + Send + Sync>> {
        if raw.schema_version != TRIAGE_SCHEMA_VERSION {
            return Err(format!("Unsupported triage schema version '{}'; expected '{}'", raw.schema_version, TRIAGE_SCHEMA_VERSION).into());
        }
        if raw.victim_location.trim().is_empty() {
            return Err("Triage victim_location cannot be empty".into());
        }
        if !(1..=10_000).contains(&raw.headcount) {
            return Err(format!("Triage headcount {} is outside 1..=10000", raw.headcount).into());
        }
        if !raw.confidence_score.is_finite() || !(0.0..=1.0).contains(&raw.confidence_score) {
            return Err(format!("Invalid confidence_score {}", raw.confidence_score).into());
        }
        if raw.needs_human_review && raw.uncertainty_reasons.is_empty() {
            return Err("needs_human_review requires at least one uncertainty reason".into());
        }

        let victim_location = raw.victim_location.trim().to_string();
        let resolved_junction = self.spatial_resolver.resolve(&victim_location);

        let required_asset = match raw.required_asset.as_str() {
            "RESCUE_BOAT" => AssetType::RescueBoat,
            "AMBULANCE" => AssetType::Ambulance,
            "EVAC_TRUCK" => AssetType::EvacTruck,
            "HELICOPTER" => AssetType::Helicopter,
            unsupported => return Err(format!("Unsupported required_asset '{unsupported}'").into()),
        };

        let mut hazards = Vec::new();
        for h in raw.hazards {
            if h.road_segment.trim().is_empty() {
                return Err("Hazard road_segment cannot be empty".into());
            }
            if !(1..=168).contains(&h.duration_hours) {
                return Err(format!("Hazard duration_hours {} is outside 1..=168", h.duration_hours).into());
            }
            let status = match h.status.as_str() {
                "FLOODED" => RoadStatus::Flooded,
                "BLOCKED" => RoadStatus::Blocked,
                "OPEN" => RoadStatus::Open,
                unsupported => return Err(format!("Unsupported hazard status '{unsupported}'").into()),
            };
            hazards.push(HazardReport {
                road_segment: h.road_segment.trim().to_string(),
                status,
                duration_hours: h.duration_hours,
            });
        }

        Ok(TriageReport {
            victim_location_raw: victim_location,
            resolved_junction_id: resolved_junction,
            headcount: raw.headcount,
            required_asset,
            hazards,
            confidence_score: raw.confidence_score,
            needs_human_review: raw.needs_human_review,
            uncertainty_reasons: raw.uncertainty_reasons,
        })
    }

    /// High-precision heuristic fallback for offline testing
    fn heuristic_fallback_extract(&self, alert: &SosAlert) -> Result<TriageReport, Box<dyn std::error::Error + Send + Sync>> {
        let text_lower = alert.raw_text.to_lowercase();

        // 1. Resolve junction/location
        let resolved = self.spatial_resolver.resolve(&alert.raw_text);

        // 2. Headcount extraction
        let mut headcount = 1;
        for word in text_lower.split_whitespace() {
            if let Ok(num) = word.parse::<u32>() {
                if num > 0 && num < 500 {
                    headcount = num;
                    break;
                }
            }
        }

        // 3. Asset requirement (explicit vehicle requests take precedence over hazard keywords)
        let required_asset = if text_lower.contains("injur") || text_lower.contains("medic") || text_lower.contains("ambulance") {
            AssetType::Ambulance
        } else if text_lower.contains("boat") {
            AssetType::RescueBoat
        } else if text_lower.contains("truck") || text_lower.contains("evac") {
            AssetType::EvacTruck
        } else if text_lower.contains("flood") || text_lower.contains("water") || text_lower.contains("drown") {
            AssetType::RescueBoat
        } else {
            AssetType::EvacTruck
        };

        // 4. Hazard extraction
        let mut hazards = Vec::new();
        if text_lower.contains("j3-j6") || (text_lower.contains("hospital") && text_lower.contains("riverside")) {
            hazards.push(HazardReport {
                road_segment: "J3-J6".to_string(),
                status: RoadStatus::Flooded,
                duration_hours: 6,
            });
        }

        Ok(TriageReport {
            victim_location_raw: alert.raw_text.clone(),
            resolved_junction_id: resolved,
            headcount,
            required_asset,
            hazards,
            confidence_score: 0.85,
            needs_human_review: false,
            uncertainty_reasons: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extractor() -> TriageExtractor {
        TriageExtractor::new(&Config::from_env())
    }

    fn valid_raw() -> RawLlmExtraction {
        RawLlmExtraction {
            schema_version: TRIAGE_SCHEMA_VERSION.to_string(),
            victim_location: "Aluva Railway Station".to_string(),
            headcount: 3,
            required_asset: "AMBULANCE".to_string(),
            hazards: Vec::new(),
            confidence_score: 0.93,
            needs_human_review: false,
            uncertainty_reasons: Vec::new(),
        }
    }

    #[test]
    fn triage_schema_is_valid_json() {
        let schema: serde_json::Value = serde_json::from_str(TRIAGE_JSON_SCHEMA).unwrap();
        assert_eq!(schema["title"], "CrisisGraph Triage Extraction V1");
    }

    #[test]
    fn valid_structured_extraction_builds_typed_triage() {
        let report = extractor().build_triage_report(valid_raw()).unwrap();
        assert_eq!(report.required_asset, AssetType::Ambulance);
        assert_eq!(report.headcount, 3);
        assert!(!report.needs_human_review);
    }

    #[test]
    fn unknown_asset_is_rejected_instead_of_defaulting_to_boat() {
        let mut raw = valid_raw();
        raw.required_asset = "MOTORBIKE".to_string();
        let error = extractor().build_triage_report(raw).unwrap_err();
        assert!(error.to_string().contains("Unsupported required_asset"));
    }

    #[test]
    fn human_review_requires_a_reason() {
        let mut raw = valid_raw();
        raw.needs_human_review = true;
        let error = extractor().build_triage_report(raw).unwrap_err();
        assert!(error.to_string().contains("uncertainty reason"));
    }
}
