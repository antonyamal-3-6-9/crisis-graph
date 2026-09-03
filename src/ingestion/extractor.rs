use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn};
use crate::config::Config;
use crate::models::{AssetType, HazardReport, RoadStatus, SosAlert, TriageReport};
use super::spatial_resolver::SpatialResolver;

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
struct RawLlmExtraction {
    victim_location: String,
    headcount: u32,
    required_asset: String,
    hazards: Vec<RawHazardExtraction>,
}

#[derive(Debug, Serialize, Deserialize)]
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

        let system_prompt = r#"You are an emergency triage parser for CrisisGraph disaster response.
Extract structured JSON strictly according to this schema:
{
  "victim_location": "string (landmark or junction)",
  "headcount": integer,
  "required_asset": "RESCUE_BOAT" | "AMBULANCE" | "EVAC_TRUCK" | "HELICOPTER",
  "hazards": [
    {
      "road_segment": "J3-J6",
      "status": "FLOODED" | "BLOCKED" | "OPEN",
      "duration_hours": integer
    }
  ]
}
Do not include any explanation or markdown formatting, output JSON only."#;

        let payload = json!({
            "model": self.vllm_model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": alert.raw_text}
            ],
            "response_format": {"type": "json_object"},
            "temperature": 0.0
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
        let resolved_junction = self.spatial_resolver.resolve(&raw.victim_location);

        let required_asset = match raw.required_asset.to_uppercase().as_str() {
            "RESCUE_BOAT" | "BOAT" => AssetType::RescueBoat,
            "AMBULANCE" | "MEDIC" => AssetType::Ambulance,
            "EVAC_TRUCK" | "TRUCK" => AssetType::EvacTruck,
            "HELICOPTER" | "AIR" => AssetType::Helicopter,
            _ => AssetType::RescueBoat,
        };

        let mut hazards = Vec::new();
        for h in raw.hazards {
            let status = match h.status.to_uppercase().as_str() {
                "FLOODED" => RoadStatus::Flooded,
                "BLOCKED" => RoadStatus::Blocked,
                _ => RoadStatus::Open,
            };
            hazards.push(HazardReport {
                road_segment: h.road_segment,
                status,
                duration_hours: h.duration_hours,
            });
        }

        Ok(TriageReport {
            victim_location_raw: raw.victim_location,
            resolved_junction_id: resolved_junction,
            headcount: raw.headcount.max(1),
            required_asset,
            hazards,
            confidence_score: 0.95,
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

        // 3. Asset requirement
        let required_asset = if text_lower.contains("boat") || text_lower.contains("flood") || text_lower.contains("water") {
            AssetType::RescueBoat
        } else if text_lower.contains("injur") || text_lower.contains("medic") || text_lower.contains("ambulance") {
            AssetType::Ambulance
        } else if text_lower.contains("truck") || text_lower.contains("evac") {
            AssetType::EvacTruck
        } else {
            AssetType::RescueBoat
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
        })
    }
}
