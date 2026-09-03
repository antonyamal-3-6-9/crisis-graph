use std::sync::Arc;
use neo4rs::{Graph, query};
use crate::config::Config;

#[derive(Clone)]
pub struct Neo4jClient {
    pub graph: Arc<Graph>,
}

impl Neo4jClient {
    pub async fn connect(config: &Config) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let graph = Graph::new(&config.neo4j_uri, &config.neo4j_user, &config.neo4j_password).await?;
        Ok(Self {
            graph: Arc::new(graph),
        })
    }

    /// Healthcheck query
    pub async fn ping(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self.graph.execute(query("RETURN 1 AS num")).await?;
        if result.next().await?.is_some() {
            Ok(())
        } else {
            Err("Failed to execute Neo4j health query".into())
        }
    }
}
