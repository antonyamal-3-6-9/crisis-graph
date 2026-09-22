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

    // Application
    pub app_env: String,
    pub log_level: String,
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
            app_env,
            log_level,
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
