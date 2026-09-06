use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Instant;

use neo4rs::{query, BoltList, BoltMap, BoltType};
use serde::Deserialize;
use tracing::info;

use super::neo4j::Neo4jClient;

#[derive(Debug, Clone, Deserialize)]
pub struct JunctionFeature {
    pub id: String,
    pub geometry: GeometryPoint,
    pub properties: JunctionProperties,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeometryPoint {
    pub coordinates: [f64; 2],
}

#[derive(Debug, Clone, Deserialize)]
pub struct JunctionProperties {
    pub junction_id: String,
    pub osm_node_id: i64,
    pub degree: i64,
    pub in_degree: i64,
    pub out_degree: i64,
    pub inside_boundary: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SegmentFeature {
    pub id: String,
    pub properties: SegmentProperties,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SegmentProperties {
    pub segment_id: String,
    pub osm_way_id: i64,
    pub from_junction_id: String,
    pub to_junction_id: String,
    pub road_name: Option<String>,
    pub road_class: String,
    pub oneway: bool,
    pub direction: String,
    pub length_m: f64,
    pub speed_kph: f64,
    pub base_travel_time_s: f64,
    pub access: SegmentAccess,
    pub baseline_status: String,
    pub source_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SegmentAccess {
    pub emergency: bool,
    pub motorcar: bool,
    pub hgv: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct GeoJsonCollection<T> {
    pub features: Vec<T>,
}

#[derive(Debug, Clone)]
pub struct BaselineLoadReport {
    pub junctions_loaded: usize,
    pub segments_loaded: usize,
    pub elapsed_seconds: f64,
}

pub struct BaselineLoader;

impl BaselineLoader {
    /// Sets up schema constraints and indexes for high-speed graph queries
    pub async fn setup_schema(neo4j: &Neo4jClient) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Setting up Neo4j schema constraints and indexes...");

        let schema_statements = [
            "CREATE CONSTRAINT junction_id_unique IF NOT EXISTS FOR (j:Junction) REQUIRE j.id IS UNIQUE",
            "CREATE INDEX junction_osm_node_id IF NOT EXISTS FOR (j:Junction) ON (j.osm_node_id)",
            "CREATE INDEX junction_dataset IF NOT EXISTS FOR (j:Junction) ON (j.dataset)",
            "CREATE INDEX connects_to_segment_id IF NOT EXISTS FOR ()-[r:CONNECTS_TO]-() ON (r.segment_id)",
            "CREATE INDEX connects_to_dataset IF NOT EXISTS FOR ()-[r:CONNECTS_TO]-() ON (r.dataset)",
            "CREATE POINT INDEX junction_point_idx IF NOT EXISTS FOR (j:Junction) ON (j.location)",
        ];

        for stmt in schema_statements {
            neo4j.graph.run(query(stmt)).await?;
        }

        info!("Neo4j schema constraints and indexes established.");
        Ok(())
    }

    /// Cleans existing data for a specific dataset tag (isolates pilot from other runs)
    pub async fn clean_dataset(neo4j: &Neo4jClient, dataset: &str) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        info!("Purging existing data for dataset '{dataset}'...");

        let delete_query = "MATCH (j:Junction {dataset: $dataset}) DETACH DELETE j";
        neo4j.graph.run(query(delete_query).param("dataset", dataset)).await?;

        info!("Dataset '{dataset}' purged.");
        Ok(0)
    }

    /// Purges all data in the database (clean slate)
    pub async fn clean_all(neo4j: &Neo4jClient) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Purging entire Neo4j database...");
        neo4j.graph.run(query("MATCH (n) DETACH DELETE n")).await?;
        info!("Neo4j database cleared.");
        Ok(())
    }

    /// Loads junctions from a GeoJSON file into Neo4j in batches
    pub async fn load_junctions_file(
        neo4j: &Neo4jClient,
        path: &Path,
        dataset: &str,
        batch_size: usize,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        info!("Reading junctions from {}...", path.display());
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let collection: GeoJsonCollection<JunctionFeature> = serde_json::from_reader(reader)?;
        let total = collection.features.len();
        info!("Found {total} junctions to ingest.");

        let mut loaded = 0;
        let cypher = "
            UNWIND $batch AS j
            CREATE (n:Junction {
                id: j.id,
                osm_node_id: j.osm_node_id,
                lat: j.lat,
                lon: j.lon,
                degree: j.degree,
                in_degree: j.in_degree,
                out_degree: j.out_degree,
                inside_boundary: j.inside_boundary,
                dataset: j.dataset,
                location: point({latitude: j.lat, longitude: j.lon})
            })
        ";

        for chunk in collection.features.chunks(batch_size) {
            let mut bolt_maps = Vec::with_capacity(chunk.len());
            for feat in chunk {
                let mut map = BoltMap::new();
                map.put("id".into(), feat.properties.junction_id.clone().into());
                map.put("osm_node_id".into(), feat.properties.osm_node_id.into());
                map.put("lon".into(), feat.geometry.coordinates[0].into());
                map.put("lat".into(), feat.geometry.coordinates[1].into());
                map.put("degree".into(), feat.properties.degree.into());
                map.put("in_degree".into(), feat.properties.in_degree.into());
                map.put("out_degree".into(), feat.properties.out_degree.into());
                map.put("inside_boundary".into(), feat.properties.inside_boundary.into());
                map.put("dataset".into(), dataset.into());
                bolt_maps.push(BoltType::Map(map));
            }

            let batch_list = BoltList { value: bolt_maps };
            neo4j.graph.run(query(cypher).param("batch", BoltType::List(batch_list))).await?;
            loaded += chunk.len();
            if loaded % 5_000 == 0 || loaded == total {
                info!("  -> Ingested {loaded} / {total} junctions ({:.1}%)", (loaded as f64 / total as f64) * 100.0);
            }
        }

        Ok(loaded)
    }

    /// Loads road segments from a GeoJSON file into Neo4j as directed CONNECTS_TO relationships
    pub async fn load_segments_file(
        neo4j: &Neo4jClient,
        path: &Path,
        dataset: &str,
        batch_size: usize,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        info!("Reading road segments from {}...", path.display());
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let collection: GeoJsonCollection<SegmentFeature> = serde_json::from_reader(reader)?;
        let total = collection.features.len();
        info!("Found {total} directed road segments to ingest.");

        let mut loaded = 0;
        let cypher = "
            UNWIND $batch AS s
            MATCH (from:Junction {id: s.from_id})
            MATCH (to:Junction {id: s.to_id})
            CREATE (from)-[r:CONNECTS_TO {
                segment_id: s.segment_id,
                osm_way_id: s.osm_way_id,
                road_name: s.road_name,
                road_class: s.road_class,
                length_m: s.length_m,
                travel_time_s: s.travel_time_s,
                speed_kph: s.speed_kph,
                oneway: s.oneway,
                direction: s.direction,
                emergency: s.emergency,
                motorcar: s.motorcar,
                hgv: s.hgv,
                baseline_status: s.baseline_status,
                source_version: s.source_version,
                dataset: s.dataset
            }]->(to)
        ";

        for chunk in collection.features.chunks(batch_size) {
            let mut bolt_maps = Vec::with_capacity(chunk.len());
            for feat in chunk {
                let mut map = BoltMap::new();
                map.put("segment_id".into(), feat.properties.segment_id.clone().into());
                map.put("osm_way_id".into(), feat.properties.osm_way_id.into());
                map.put("from_id".into(), feat.properties.from_junction_id.clone().into());
                map.put("to_id".into(), feat.properties.to_junction_id.clone().into());
                map.put("road_name".into(), feat.properties.road_name.clone().unwrap_or_default().into());
                map.put("road_class".into(), feat.properties.road_class.clone().into());
                map.put("length_m".into(), feat.properties.length_m.into());
                map.put("travel_time_s".into(), feat.properties.base_travel_time_s.into());
                map.put("speed_kph".into(), feat.properties.speed_kph.into());
                map.put("oneway".into(), feat.properties.oneway.into());
                map.put("direction".into(), feat.properties.direction.clone().into());
                map.put("emergency".into(), feat.properties.access.emergency.into());
                map.put("motorcar".into(), feat.properties.access.motorcar.into());
                map.put("hgv".into(), feat.properties.access.hgv.into());
                map.put("baseline_status".into(), feat.properties.baseline_status.clone().into());
                map.put("source_version".into(), feat.properties.source_version.clone().into());
                map.put("dataset".into(), dataset.into());
                bolt_maps.push(BoltType::Map(map));
            }

            let batch_list = BoltList { value: bolt_maps };
            neo4j.graph.run(query(cypher).param("batch", BoltType::List(batch_list))).await?;
            loaded += chunk.len();
            if loaded % 5_000 == 0 || loaded == total {
                info!("  -> Ingested {loaded} / {total} road segments ({:.1}%)", (loaded as f64 / total as f64) * 100.0);
            }
        }

        Ok(loaded)
    }

    /// Full end-to-end baseline load from GeoJSON files
    pub async fn load_pilot_baseline(
        neo4j: &Neo4jClient,
        junctions_path: &Path,
        segments_path: &Path,
        dataset: &str,
        clean_first: bool,
        batch_size: usize,
    ) -> Result<BaselineLoadReport, Box<dyn std::error::Error + Send + Sync>> {
        let start = Instant::now();

        // 1. Setup constraints & indexes
        Self::setup_schema(neo4j).await?;

        // 2. Clean dataset if requested
        if clean_first {
            Self::clean_dataset(neo4j, dataset).await?;
        }

        // 3. Load Junctions
        let junctions_loaded = Self::load_junctions_file(neo4j, junctions_path, dataset, batch_size).await?;

        // 4. Load Road Segments
        let segments_loaded = Self::load_segments_file(neo4j, segments_path, dataset, batch_size).await?;

        let elapsed = start.elapsed().as_secs_f64();

        Ok(BaselineLoadReport {
            junctions_loaded,
            segments_loaded,
            elapsed_seconds: elapsed,
        })
    }
}
