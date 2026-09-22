use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::info;

use super::spatial_resolver::SpatialResolver;
use crate::config::Config;
use crate::models::{
    AssetType, CandidateHazardReport, RoadStatus, SosAlert, TriageInferenceMetadata, TriageReport,
};

const TRIAGE_SCHEMA_VERSION: &str = "triage-extraction-v2";
const TRIAGE_PROMPT_VERSION: &str = "triage-extraction-v2.1.1";
const TRIAGE_JSON_SCHEMA: &str = include_str!("../../schemas/triage-extraction-v2.json");
const TRIAGE_SYSTEM_PROMPT: &str = include_str!("../../ml/prompts/triage-extraction-v2.1.1.txt");

type ExtractResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
pub struct TriageExtractor {
    http_client: Client,
    inference_backend: String,
    inference_base_url: String,
    inference_model: String,
    inference_api_key: Option<String>,
    adapter_id: Option<String>,
    adapter_sha256: Option<String>,
    spatial_resolver: SpatialResolver,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLlmExtraction {
    schema_version: String,
    victim_location: Option<String>,
    headcount: Option<u32>,
    required_asset: Option<String>,
    hazards: Vec<RawHazardExtraction>,
    confidence_score: f32,
    needs_human_review: bool,
    uncertainty_reasons: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHazardExtraction {
    road_segment: Option<String>,
    status: Option<String>,
    duration_hours: Option<u32>,
}

impl TriageExtractor {
    pub fn new(config: &Config) -> Self {
        Self {
            http_client: Client::builder()
                .timeout(Duration::from_secs(config.inference_timeout_secs))
                .build()
                .unwrap_or_default(),
            inference_backend: config.inference_backend.clone(),
            inference_base_url: config.inference_base_url.clone(),
            inference_model: config.inference_model.clone(),
            inference_api_key: config.inference_api_key.clone(),
            adapter_id: config.triage_adapter_id.clone(),
            adapter_sha256: config.triage_adapter_sha256.clone(),
            spatial_resolver: SpatialResolver::new(),
        }
    }

    /// Extract an unverified candidate triage report from natural-language SOS
    /// text. Endpoint, protocol, parse, and validation failures are returned to
    /// the pipeline, which fails closed to human dispatch; no dispatchable facts
    /// are guessed locally.
    pub async fn extract(&self, alert: &SosAlert) -> ExtractResult<TriageReport> {
        info!(
            alert_id = %alert.alert_id,
            backend = %self.inference_backend,
            model = %self.inference_model,
            adapter = ?self.adapter_id,
            "Extracting candidate triage facts"
        );

        let payload = self.request_payload(&alert.raw_text)?;
        let url = format!(
            "{}/chat/completions",
            self.inference_base_url.trim_end_matches('/')
        );
        let started = Instant::now();
        let mut request = self.http_client.post(&url).json(&payload);
        if let Some(api_key) = &self.inference_api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request
            .send()
            .await
            .map_err(|error| format!("Triage inference endpoint unavailable at {url}: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail: String = body.chars().take(500).collect();
            return Err(format!("Triage inference HTTP {status}: {detail}").into());
        }

        let response: OpenAiChatResponse = response
            .json()
            .await
            .map_err(|error| format!("Invalid OpenAI-compatible response: {error}"))?;
        let content = response
            .choices
            .into_iter()
            .next()
            .ok_or("Triage inference response contained no choices")?
            .message
            .content;
        let raw = Self::parse_model_content(&content)?;

        self.build_triage_report(raw, started.elapsed().as_millis() as u64)
    }

    fn parse_model_content(content: &str) -> ExtractResult<RawLlmExtraction> {
        let value: Value = serde_json::from_str(content)
            .map_err(|error| format!("Triage model returned invalid JSON: {error}"))?;
        let object = value
            .as_object()
            .ok_or("Triage model output must be a JSON object")?;
        for field in [
            "schema_version",
            "victim_location",
            "headcount",
            "required_asset",
            "hazards",
            "confidence_score",
            "needs_human_review",
            "uncertainty_reasons",
        ] {
            if !object.contains_key(field) {
                return Err(format!("Triage model output omitted required field '{field}'").into());
            }
        }
        if let Some(hazards) = object.get("hazards").and_then(Value::as_array) {
            for (index, hazard) in hazards.iter().enumerate() {
                let hazard = hazard
                    .as_object()
                    .ok_or_else(|| format!("Hazard {index} must be a JSON object"))?;
                for field in ["road_segment", "status", "duration_hours"] {
                    if !hazard.contains_key(field) {
                        return Err(
                            format!("Hazard {index} omitted required field '{field}'").into()
                        );
                    }
                }
            }
        }
        serde_json::from_value(value).map_err(|error| {
            format!("Triage model output violates the typed contract: {error}").into()
        })
    }

    fn request_payload(&self, report: &str) -> ExtractResult<Value> {
        let schema: Value = serde_json::from_str(TRIAGE_JSON_SCHEMA)?;
        let mut payload = json!({
            "model": self.inference_model,
            "messages": [
                {"role": "system", "content": TRIAGE_SYSTEM_PROMPT},
                {"role": "user", "content": report}
            ],
            "temperature": 0.0,
            "max_tokens": 512
        });

        match self.inference_backend.trim().to_ascii_lowercase().as_str() {
            "llama-cpp" | "llama.cpp" => {
                payload["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "crisisgraph_triage_extraction",
                        "strict": true,
                        "schema": schema
                    }
                });
            }
            "vllm" => {
                payload["structured_outputs"] = json!({"json": schema});
            }
            unsupported => {
                return Err(format!(
                    "Unsupported INFERENCE_BACKEND '{unsupported}'; expected llama-cpp or vllm"
                )
                .into());
            }
        }
        Ok(payload)
    }

    fn build_triage_report(
        &self,
        raw: RawLlmExtraction,
        latency_ms: u64,
    ) -> ExtractResult<TriageReport> {
        if raw.schema_version != TRIAGE_SCHEMA_VERSION {
            return Err(format!(
                "Unsupported triage schema version '{}'; expected '{}'",
                raw.schema_version, TRIAGE_SCHEMA_VERSION
            )
            .into());
        }
        if raw.hazards.len() > 20 {
            return Err("Triage hazards exceeds the 20-item schema limit".into());
        }
        if !raw.confidence_score.is_finite() || !(0.0..=1.0).contains(&raw.confidence_score) {
            return Err(format!("Invalid confidence_score {}", raw.confidence_score).into());
        }
        if raw.uncertainty_reasons.len() > 10 {
            return Err("Triage uncertainty_reasons exceeds the 10-item schema limit".into());
        }
        if raw
            .uncertainty_reasons
            .iter()
            .any(|reason| reason.trim().is_empty() || reason.chars().count() > 300)
        {
            return Err("Triage uncertainty reason is empty or exceeds 300 characters".into());
        }

        let victim_location = match raw.victim_location {
            Some(value) => {
                let value = value.trim().to_string();
                if value.is_empty() || value.chars().count() > 500 {
                    return Err("Triage victim_location is empty or exceeds 500 characters".into());
                }
                Some(value)
            }
            None => None,
        };
        if let Some(headcount) = raw.headcount {
            if !(1..=10_000).contains(&headcount) {
                return Err(format!("Triage headcount {headcount} is outside 1..=10000").into());
            }
        }

        let required_asset = match raw.required_asset.as_deref() {
            Some("RESCUE_BOAT") => Some(AssetType::RescueBoat),
            Some("AMBULANCE") => Some(AssetType::Ambulance),
            Some("EVAC_TRUCK") => Some(AssetType::EvacTruck),
            Some("HELICOPTER") => Some(AssetType::Helicopter),
            Some(unsupported) => {
                return Err(format!("Unsupported required_asset '{unsupported}'").into())
            }
            None => None,
        };

        let mut hazards = Vec::with_capacity(raw.hazards.len());
        for raw_hazard in raw.hazards {
            let road_segment = match raw_hazard.road_segment {
                Some(value) => {
                    let value = value.trim().to_string();
                    if value.is_empty() || value.chars().count() > 200 {
                        return Err("Hazard road_segment is empty or exceeds 200 characters".into());
                    }
                    Some(value)
                }
                None => None,
            };
            let status = match raw_hazard.status.as_deref() {
                Some("FLOODED") => Some(RoadStatus::Flooded),
                Some("BLOCKED") => Some(RoadStatus::Blocked),
                Some("OPEN") => Some(RoadStatus::Open),
                Some(unsupported) => {
                    return Err(format!("Unsupported hazard status '{unsupported}'").into())
                }
                None => None,
            };
            if let Some(duration) = raw_hazard.duration_hours {
                if !(1..=168).contains(&duration) {
                    return Err(
                        format!("Hazard duration_hours {duration} is outside 1..=168").into(),
                    );
                }
            }
            if road_segment.is_none() && status.is_none() && raw_hazard.duration_hours.is_none() {
                return Err("A hazard cannot have all fields null".into());
            }
            hazards.push(CandidateHazardReport {
                road_segment,
                status,
                duration_hours: raw_hazard.duration_hours,
            });
        }

        let resolved_junction_id = victim_location
            .as_deref()
            .and_then(|location| self.spatial_resolver.resolve(location));
        let mut needs_human_review = raw.needs_human_review;
        let mut uncertainty_reasons = raw.uncertainty_reasons;

        Self::require_review_if_missing(
            &mut needs_human_review,
            &mut uncertainty_reasons,
            victim_location.is_none(),
            "Victim location was not provided",
        );
        Self::require_review_if_missing(
            &mut needs_human_review,
            &mut uncertainty_reasons,
            raw.headcount.is_none(),
            "Headcount was not provided",
        );
        Self::require_review_if_missing(
            &mut needs_human_review,
            &mut uncertainty_reasons,
            required_asset.is_none(),
            "Required asset was not specified",
        );
        for hazard in &hazards {
            Self::require_review_if_missing(
                &mut needs_human_review,
                &mut uncertainty_reasons,
                hazard.road_segment.is_none(),
                "Hazard road was not specifically identified",
            );
            Self::require_review_if_missing(
                &mut needs_human_review,
                &mut uncertainty_reasons,
                hazard.status.is_none(),
                "Hazard status was not confirmed",
            );
            Self::require_review_if_missing(
                &mut needs_human_review,
                &mut uncertainty_reasons,
                hazard.duration_hours.is_none(),
                "Hazard duration was not provided",
            );
        }
        if !uncertainty_reasons.is_empty() {
            needs_human_review = true;
        }
        if needs_human_review && uncertainty_reasons.is_empty() {
            uncertainty_reasons.push("Model requested human review".to_string());
        }

        Ok(TriageReport {
            victim_location_raw: victim_location,
            resolved_junction_id,
            headcount: raw.headcount,
            required_asset,
            hazards,
            confidence_score: raw.confidence_score,
            needs_human_review,
            uncertainty_reasons,
            inference: TriageInferenceMetadata {
                backend: self.inference_backend.clone(),
                model: self.inference_model.clone(),
                adapter_id: self.adapter_id.clone(),
                adapter_sha256: self.adapter_sha256.clone(),
                prompt_version: TRIAGE_PROMPT_VERSION.to_string(),
                schema_version: TRIAGE_SCHEMA_VERSION.to_string(),
                latency_ms,
            },
        })
    }

    fn require_review_if_missing(
        needs_human_review: &mut bool,
        reasons: &mut Vec<String>,
        missing: bool,
        reason: &str,
    ) {
        if !missing {
            return;
        }
        *needs_human_review = true;
        if reasons.len() < 10 && !reasons.iter().any(|existing| existing == reason) {
            reasons.push(reason.to_string());
        }
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
            victim_location: Some("Aluva Railway Station".to_string()),
            headcount: Some(3),
            required_asset: Some("AMBULANCE".to_string()),
            hazards: Vec::new(),
            confidence_score: 0.95,
            needs_human_review: false,
            uncertainty_reasons: Vec::new(),
        }
    }

    #[test]
    fn triage_schema_is_valid_json() {
        let schema: Value = serde_json::from_str(TRIAGE_JSON_SCHEMA).unwrap();
        assert_eq!(schema["title"], "CrisisGraph Triage Extraction V2");
    }

    #[test]
    fn valid_structured_extraction_builds_typed_triage() {
        let report = extractor().build_triage_report(valid_raw(), 17).unwrap();
        assert_eq!(report.required_asset, Some(AssetType::Ambulance));
        assert_eq!(report.headcount, Some(3));
        assert!(!report.needs_human_review);
        assert_eq!(report.inference.latency_ms, 17);
    }

    #[test]
    fn unknown_asset_is_rejected_instead_of_defaulting() {
        let mut raw = valid_raw();
        raw.required_asset = Some("MOTORBIKE".to_string());
        let error = extractor().build_triage_report(raw, 0).unwrap_err();
        assert!(error.to_string().contains("Unsupported required_asset"));
    }

    #[test]
    fn missing_scalar_forces_human_review() {
        let mut raw = valid_raw();
        raw.required_asset = None;
        let report = extractor().build_triage_report(raw, 0).unwrap();
        assert!(report.needs_human_review);
        assert!(report
            .uncertainty_reasons
            .contains(&"Required asset was not specified".to_string()));
    }

    #[test]
    fn incomplete_hazard_forces_human_review() {
        let mut raw = valid_raw();
        raw.hazards.push(RawHazardExtraction {
            road_segment: Some("Bridge Road".to_string()),
            status: Some("FLOODED".to_string()),
            duration_hours: None,
        });
        let report = extractor().build_triage_report(raw, 0).unwrap();
        assert!(report.needs_human_review);
        assert_eq!(report.hazards[0].duration_hours, None);
    }

    #[test]
    fn omitted_nullable_field_is_rejected_but_explicit_null_is_accepted() {
        let explicit_null = r#"{
            "schema_version":"triage-extraction-v2",
            "victim_location":"Bank Junction",
            "headcount":5,
            "required_asset":null,
            "hazards":[],
            "confidence_score":0.55,
            "needs_human_review":true,
            "uncertainty_reasons":["Required asset was not specified"]
        }"#;
        assert!(TriageExtractor::parse_model_content(explicit_null).is_ok());

        let omitted = explicit_null.replace("\n            \"required_asset\":null,", "");
        let error = TriageExtractor::parse_model_content(&omitted).unwrap_err();
        assert!(error
            .to_string()
            .contains("omitted required field 'required_asset'"));
    }

    #[test]
    fn llama_cpp_payload_uses_openai_json_schema_format() {
        let mut extractor = extractor();
        extractor.inference_backend = "llama-cpp".to_string();
        let payload = extractor.request_payload("test report").unwrap();
        assert_eq!(payload["response_format"]["type"], "json_schema");
        assert!(payload.get("structured_outputs").is_none());
    }

    #[test]
    fn vllm_payload_uses_structured_outputs() {
        let mut extractor = extractor();
        extractor.inference_backend = "vllm".to_string();
        let payload = extractor.request_payload("test report").unwrap();
        assert!(payload["structured_outputs"]["json"].is_object());
        assert!(payload.get("response_format").is_none());
    }

    #[tokio::test]
    async fn unavailable_endpoint_returns_error_without_guessing_facts() {
        let mut extractor = extractor();
        extractor.inference_base_url = "http://127.0.0.1:1/v1".to_string();
        let alert = SosAlert {
            alert_id: "test-unavailable".to_string(),
            raw_text: "Three people at Aluva Railway Station need an ambulance.".to_string(),
            timestamp: chrono::Utc::now(),
            source_channel: Some("test".to_string()),
        };

        let error = extractor.extract(&alert).await.unwrap_err();
        assert!(error.to_string().contains("endpoint unavailable"));
    }
}
