use dotenvy::dotenv;
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    // Neo4j Graph Database
    pub neo4j_uri: String,
    pub neo4j_user: String,
    pub neo4j_password: String,

    // Redis / Valkey
    pub redis_host: String,
    pub redis_port: u16,
    pub redis_db: u8,
    pub redis_password: Option<String>,

    // OpenAI-compatible inference (local llama.cpp/vLLM or a reachable endpoint)
    pub inference_backend: String,
    pub inference_base_url: String,
    pub inference_model: String,
    pub inference_api_key: Option<String>,
    pub inference_timeout_secs: u64,
    pub triage_adapter_id: Option<String>,
    pub triage_adapter_sha256: Option<String>,

    /// Maximum concurrent inference requests per worker. Keep low for
    /// llama.cpp on CPU; match llama-server `-np` slots.
    pub inference_max_concurrency: usize,

    // Application
    pub app_env: String,
    pub log_level: String,

    /// Tunable parameters for the incident pipeline (docs: pipeline §4.10).
    pub params: PipelineParams,
}

/// Pipeline parameters with the proposed defaults from the core design.
/// Every value can be overridden by an environment variable of the same name
/// in upper case (e.g. `MATCHER_CYCLE_MS`).
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineParams {
    // Deduplication (minimal rules D-2 / D-3)
    pub dedup_same_device_window_secs: u64,
    pub dedup_nearby_radius_m: f64,
    pub dedup_nearby_window_secs: u64,

    // Delivery
    pub stream_claim_idle_ms: u64,
    pub stream_max_retries: u32,
    pub retry_backoff_initial_ms: u64,
    pub retry_backoff_max_ms: u64,
    pub outbox_poll_ms: u64,
    pub outbox_batch_size: usize,
    pub outbox_retention_days: u64,

    // Allocation
    pub matcher_cycle_ms: u64,
    pub matcher_debounce_ms: u64,
    pub hold_expiry_secs: u64,
    pub aging_stable_to_urgent_secs: u64,
    pub escalate_critical_secs: u64,
    pub escalate_urgent_secs: u64,
    pub escalate_stable_secs: u64,
    pub reserve_ahead_secs: u64,
    pub jit_margin_secs: u64,

    // Leases
    pub lease_ttl_ms: u64,
    pub lease_renew_ms: u64,

    // Resilience
    pub breaker_consecutive_failures: u32,
    pub breaker_error_rate: f64,
    pub breaker_window_secs: u64,
    pub breaker_half_open_secs: u64,

    // Graph snapshot
    pub overlay_refresh_secs: u64,

    // Runtime
    pub role_drain_timeout_secs: u64,
}

impl Default for PipelineParams {
    fn default() -> Self {
        Self {
            dedup_same_device_window_secs: 30 * 60,
            dedup_nearby_radius_m: 100.0,
            dedup_nearby_window_secs: 60 * 60,
            stream_claim_idle_ms: 60_000,
            stream_max_retries: 5,
            retry_backoff_initial_ms: 1_000,
            retry_backoff_max_ms: 30_000,
            outbox_poll_ms: 250,
            outbox_batch_size: 100,
            outbox_retention_days: 7,
            matcher_cycle_ms: 2_000,
            matcher_debounce_ms: 250,
            hold_expiry_secs: 30,
            aging_stable_to_urgent_secs: 30 * 60,
            escalate_critical_secs: 3 * 60,
            escalate_urgent_secs: 10 * 60,
            escalate_stable_secs: 30 * 60,
            reserve_ahead_secs: 10 * 60,
            jit_margin_secs: 2 * 60,
            lease_ttl_ms: 10_000,
            lease_renew_ms: 3_000,
            breaker_consecutive_failures: 5,
            breaker_error_rate: 0.5,
            breaker_window_secs: 30,
            breaker_half_open_secs: 30,
            overlay_refresh_secs: 30,
            role_drain_timeout_secs: 10,
        }
    }
}

impl PipelineParams {
    /// Load from the process environment, falling back to defaults for any
    /// missing or invalid value.
    pub fn from_env() -> Self {
        Self::from_lookup(|key| env::var(key).ok())
    }

    /// Load using an arbitrary key lookup (used by tests).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let d = Self::default();
        let u64_or = |key: &str, default: u64| {
            lookup(key)
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(default)
        };
        let u32_or = |key: &str, default: u32| {
            lookup(key)
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(default)
        };
        let usize_or = |key: &str, default: usize| {
            lookup(key)
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(default)
        };
        let f64_or = |key: &str, default: f64, max: f64| {
            lookup(key)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v > 0.0 && *v <= max)
                .unwrap_or(default)
        };

        let params = Self {
            dedup_same_device_window_secs: u64_or("DEDUP_SAME_DEVICE_WINDOW_SECS", d.dedup_same_device_window_secs),
            dedup_nearby_radius_m: f64_or("DEDUP_NEARBY_RADIUS_M", d.dedup_nearby_radius_m, 10_000.0),
            dedup_nearby_window_secs: u64_or("DEDUP_NEARBY_WINDOW_SECS", d.dedup_nearby_window_secs),
            stream_claim_idle_ms: u64_or("STREAM_CLAIM_IDLE_MS", d.stream_claim_idle_ms),
            stream_max_retries: u32_or("STREAM_MAX_RETRIES", d.stream_max_retries),
            retry_backoff_initial_ms: u64_or("RETRY_BACKOFF_INITIAL_MS", d.retry_backoff_initial_ms),
            retry_backoff_max_ms: u64_or("RETRY_BACKOFF_MAX_MS", d.retry_backoff_max_ms),
            outbox_poll_ms: u64_or("OUTBOX_POLL_MS", d.outbox_poll_ms),
            outbox_batch_size: usize_or("OUTBOX_BATCH_SIZE", d.outbox_batch_size),
            outbox_retention_days: u64_or("OUTBOX_RETENTION_DAYS", d.outbox_retention_days),
            matcher_cycle_ms: u64_or("MATCHER_CYCLE_MS", d.matcher_cycle_ms),
            matcher_debounce_ms: u64_or("MATCHER_DEBOUNCE_MS", d.matcher_debounce_ms),
            hold_expiry_secs: u64_or("HOLD_EXPIRY_SECS", d.hold_expiry_secs),
            aging_stable_to_urgent_secs: u64_or("AGING_STABLE_TO_URGENT_SECS", d.aging_stable_to_urgent_secs),
            escalate_critical_secs: u64_or("ESCALATE_CRITICAL_SECS", d.escalate_critical_secs),
            escalate_urgent_secs: u64_or("ESCALATE_URGENT_SECS", d.escalate_urgent_secs),
            escalate_stable_secs: u64_or("ESCALATE_STABLE_SECS", d.escalate_stable_secs),
            reserve_ahead_secs: u64_or("RESERVE_AHEAD_SECS", d.reserve_ahead_secs),
            jit_margin_secs: u64_or("JIT_MARGIN_SECS", d.jit_margin_secs),
            lease_ttl_ms: u64_or("LEASE_TTL_MS", d.lease_ttl_ms),
            lease_renew_ms: u64_or("LEASE_RENEW_MS", d.lease_renew_ms),
            breaker_consecutive_failures: u32_or("BREAKER_CONSECUTIVE_FAILURES", d.breaker_consecutive_failures),
            breaker_error_rate: f64_or("BREAKER_ERROR_RATE", d.breaker_error_rate, 1.0),
            breaker_window_secs: u64_or("BREAKER_WINDOW_SECS", d.breaker_window_secs),
            breaker_half_open_secs: u64_or("BREAKER_HALF_OPEN_SECS", d.breaker_half_open_secs),
            overlay_refresh_secs: u64_or("OVERLAY_REFRESH_SECS", d.overlay_refresh_secs),
            role_drain_timeout_secs: u64_or("ROLE_DRAIN_TIMEOUT_SECS", d.role_drain_timeout_secs),
        };

        // A lease must be renewed well before it expires; otherwise fall back
        // to the safe defaults for both.
        if params.lease_renew_ms * 2 > params.lease_ttl_ms {
            return Self {
                lease_ttl_ms: d.lease_ttl_ms,
                lease_renew_ms: d.lease_renew_ms,
                ..params
            };
        }
        params
    }
}

impl Config {
    pub fn from_env() -> Self {
        dotenv().ok();

        let neo4j_uri =
            env::var("NEO4J_URI").unwrap_or_else(|_| "bolt://localhost:7687".to_string());
        let neo4j_user = env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
        let neo4j_password =
            env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "crisisgraph123".to_string());

        let redis_host = env::var("REDIS_HOST").unwrap_or_else(|_| "localhost".to_string());
        let redis_port = env::var("REDIS_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(6379);
        let redis_db = env::var("REDIS_DB")
            .ok()
            .and_then(|d| d.parse::<u8>().ok())
            .unwrap_or(0);
        let redis_password = env::var("REDIS_PASSWORD").ok().filter(|p| !p.is_empty());

        // INFERENCE_* is backend-neutral. VLLM_* remains a compatibility alias
        // for existing local configuration.
        let inference_backend =
            env::var("INFERENCE_BACKEND").unwrap_or_else(|_| "vllm".to_string());
        let inference_base_url = env::var("INFERENCE_BASE_URL")
            .or_else(|_| env::var("VLLM_BASE_URL"))
            .unwrap_or_else(|_| "http://localhost:8000/v1".to_string());
        let inference_model = env::var("INFERENCE_MODEL")
            .or_else(|_| env::var("VLLM_MODEL"))
            .unwrap_or_else(|_| "Qwen/Qwen3-4B-Instruct-2507".to_string());
        let inference_api_key = env::var("INFERENCE_API_KEY")
            .ok()
            .filter(|value| !value.is_empty());
        let inference_timeout_secs = env::var("INFERENCE_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (1..=300).contains(value))
            .unwrap_or(60);
        let triage_adapter_id = env::var("TRIAGE_ADAPTER_ID")
            .ok()
            .filter(|value| !value.is_empty());
        let triage_adapter_sha256 = env::var("TRIAGE_ADAPTER_SHA256")
            .ok()
            .filter(|value| !value.is_empty());

        let inference_max_concurrency = env::var("INFERENCE_MAX_CONCURRENCY")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| (1..=64).contains(value))
            .unwrap_or(1);

        let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());
        let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        Self {
            neo4j_uri,
            neo4j_user,
            neo4j_password,
            redis_host,
            redis_port,
            redis_db,
            redis_password,
            inference_backend,
            inference_base_url,
            inference_model,
            inference_api_key,
            inference_timeout_secs,
            triage_adapter_id,
            triage_adapter_sha256,
            inference_max_concurrency,
            app_env,
            log_level,
            params: PipelineParams::from_env(),
        }
    }

    pub fn redis_url(&self) -> String {
        match &self.redis_password {
            Some(pass) => format!(
                "redis://:{}@{}:{}/{}",
                pass, self.redis_host, self.redis_port, self.redis_db
            ),
            None => format!(
                "redis://{}:{}/{}",
                self.redis_host, self.redis_port, self.redis_db
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PipelineParams;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn empty_environment_yields_defaults() {
        assert_eq!(PipelineParams::from_lookup(lookup(&[])), PipelineParams::default());
    }

    #[test]
    fn valid_overrides_are_applied() {
        let params = PipelineParams::from_lookup(lookup(&[
            ("MATCHER_CYCLE_MS", "1500"),
            ("DEDUP_NEARBY_RADIUS_M", "250.5"),
            ("BREAKER_ERROR_RATE", "0.25"),
            ("OUTBOX_BATCH_SIZE", "50"),
        ]));
        assert_eq!(params.matcher_cycle_ms, 1500);
        assert_eq!(params.dedup_nearby_radius_m, 250.5);
        assert_eq!(params.breaker_error_rate, 0.25);
        assert_eq!(params.outbox_batch_size, 50);
    }

    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let d = PipelineParams::default();
        let params = PipelineParams::from_lookup(lookup(&[
            ("MATCHER_CYCLE_MS", "fast"),
            ("HOLD_EXPIRY_SECS", "0"),
            ("BREAKER_ERROR_RATE", "1.5"),
            ("DEDUP_NEARBY_RADIUS_M", "-3"),
        ]));
        assert_eq!(params.matcher_cycle_ms, d.matcher_cycle_ms);
        assert_eq!(params.hold_expiry_secs, d.hold_expiry_secs);
        assert_eq!(params.breaker_error_rate, d.breaker_error_rate);
        assert_eq!(params.dedup_nearby_radius_m, d.dedup_nearby_radius_m);
    }

    #[test]
    fn lease_renewal_must_leave_headroom() {
        let d = PipelineParams::default();
        let params = PipelineParams::from_lookup(lookup(&[
            ("LEASE_TTL_MS", "4000"),
            ("LEASE_RENEW_MS", "3000"),
        ]));
        assert_eq!(params.lease_ttl_ms, d.lease_ttl_ms);
        assert_eq!(params.lease_renew_ms, d.lease_renew_ms);
    }
}
