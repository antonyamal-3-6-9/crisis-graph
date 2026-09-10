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

    // Local vLLM Inference
    pub vllm_base_url: String,
    pub vllm_model: String,

    // Application
    pub app_env: String,
    pub log_level: String,
}

impl Config {
    pub fn from_env() -> Self {
        dotenv().ok();

        let neo4j_uri = env::var("NEO4J_URI").unwrap_or_else(|_| "bolt://localhost:7687".to_string());
        let neo4j_user = env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
        let neo4j_password = env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "crisisgraph123".to_string());

        let redis_host = env::var("REDIS_HOST").unwrap_or_else(|_| "localhost".to_string());
        let redis_port = env::var("REDIS_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(6379);
        let redis_db = env::var("REDIS_DB")
            .ok()
            .and_then(|d| d.parse::<u8>().ok())
            .unwrap_or(0);
        let redis_password = env::var("REDIS_PASSWORD")
            .ok()
            .filter(|p| !p.is_empty());

        let vllm_base_url = env::var("VLLM_BASE_URL").unwrap_or_else(|_| "http://localhost:8000/v1".to_string());
        let vllm_model = env::var("VLLM_MODEL").unwrap_or_else(|_| "Qwen/Qwen3-4B-Instruct-2507".to_string());

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
            vllm_base_url,
            vllm_model,
            app_env,
            log_level,
        }
    }

    pub fn redis_url(&self) -> String {
        match &self.redis_password {
            Some(pass) => format!("redis://:{}@{}:{}/{}", pass, self.redis_host, self.redis_port, self.redis_db),
            None => format!("redis://{}:{}/{}", self.redis_host, self.redis_port, self.redis_db),
        }
    }
}
