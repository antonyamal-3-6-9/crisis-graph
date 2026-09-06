//! Loads normalized directed baseline road network into Neo4j.
//!
//! Example:
//! cargo run --bin load_baseline_graph -- \
//!   --junctions data/processed/aluva-periyar-pilot/junctions.geojson \
//!   --segments data/processed/aluva-periyar-pilot/road_segments.geojson \
//!   --dataset aluva-periyar-pilot \
//!   --clean

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use neo4rs::query;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use crisis_graph::config::Config;
use crisis_graph::db::{BaselineLoader, Neo4jClient};

#[derive(Debug)]
struct Args {
    junctions_path: PathBuf,
    segments_path: PathBuf,
    dataset: String,
    batch_size: usize,
    clean: bool,
    clean_all: bool,
}

fn usage() -> &'static str {
    "Usage: cargo run --bin load_baseline_graph -- \\\n  [--junctions <path/junctions.geojson>] \\\n  [--segments <path/road_segments.geojson>] \\\n  [--dataset <name>] \\\n  [--batch-size <number>] \\\n  [--clean] \\\n  [--clean-all]"
}

fn parse_args() -> Result<Args, Box<dyn Error + Send + Sync>> {
    let mut junctions_path = PathBuf::from("data/processed/aluva-periyar-pilot/junctions.geojson");
    let mut segments_path = PathBuf::from("data/processed/aluva-periyar-pilot/road_segments.geojson");
    let mut dataset = "aluva-periyar-pilot".to_string();
    let mut batch_size = 1000;
    let mut clean = false;
    let mut clean_all = false;

    let mut values = env::args().skip(1);
    while let Some(flag) = values.next() {
        if flag == "--help" || flag == "-h" {
            println!("{}", usage());
            std::process::exit(0);
        }

        match flag.as_str() {
            "--junctions" => {
                let val = values.next().ok_or_else(|| format!("Missing value for {flag}"))?;
                junctions_path = PathBuf::from(val);
            }
            "--segments" => {
                let val = values.next().ok_or_else(|| format!("Missing value for {flag}"))?;
                segments_path = PathBuf::from(val);
            }
            "--dataset" => {
                let val = values.next().ok_or_else(|| format!("Missing value for {flag}"))?;
                dataset = val;
            }
            "--batch-size" => {
                let val = values.next().ok_or_else(|| format!("Missing value for {flag}"))?;
                batch_size = val.parse::<usize>()?;
            }
            "--clean" => clean = true,
            "--clean-all" => clean_all = true,
            _ => return Err(format!("Unknown option: {flag}.\n{}", usage()).into()),
        }
    }

    Ok(Args {
        junctions_path,
        segments_path,
        dataset,
        batch_size,
        clean,
        clean_all,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let args = parse_args()?;

    println!("==================================================");
    println!(" CrisisGraph Neo4j Baseline Graph Loader");
    println!("==================================================");
    println!("Junctions file:  {}", args.junctions_path.display());
    println!("Segments file:   {}", args.segments_path.display());
    println!("Dataset tag:     {}", args.dataset);
    println!("Batch size:      {}", args.batch_size);
    println!("Clean dataset:   {}", args.clean);
    println!("Clean entire DB: {}", args.clean_all);

    if !args.junctions_path.exists() {
        return Err(format!("Junctions file '{}' does not exist.", args.junctions_path.display()).into());
    }
    if !args.segments_path.exists() {
        return Err(format!("Segments file '{}' does not exist.", args.segments_path.display()).into());
    }

    // 1. Connect to Neo4j
    let config = Config::from_env();
    println!("Connecting to Neo4j at {} (user: {})...", config.neo4j_uri, config.neo4j_user);
    let neo4j = Neo4jClient::connect(&config).await?;
    neo4j.ping().await?;
    println!("Connected successfully to Neo4j.");

    // 2. Clear if requested
    if args.clean_all {
        BaselineLoader::clean_all(&neo4j).await?;
    } else if args.clean {
        BaselineLoader::clean_dataset(&neo4j, &args.dataset).await?;
    }

    // 3. Load baseline data
    let start = Instant::now();
    println!("Ingesting baseline graph into Neo4j in batches of {}...", args.batch_size);

    let report = BaselineLoader::load_pilot_baseline(
        &neo4j,
        &args.junctions_path,
        &args.segments_path,
        &args.dataset,
        false, // already handled above if requested
        args.batch_size,
    ).await?;

    // 4. Query live verification metrics from Neo4j
    println!("--------------------------------------------------");
    println!(" Verifying database state via Cypher queries...");

    let count_junctions_q = query("MATCH (j:Junction {dataset: $dataset}) RETURN count(j) AS count")
        .param("dataset", args.dataset.clone());
    let mut row_j = neo4j.graph.execute(count_junctions_q).await?;
    let db_junctions_count: i64 = if let Some(row) = row_j.next().await? {
        row.get("count").unwrap_or(0)
    } else {
        0
    };

    let count_segments_q = query("MATCH ()-[r:CONNECTS_TO {dataset: $dataset}]->() RETURN count(r) AS count")
        .param("dataset", args.dataset.clone());
    let mut row_s = neo4j.graph.execute(count_segments_q).await?;
    let db_segments_count: i64 = if let Some(row) = row_s.next().await? {
        row.get("count").unwrap_or(0)
    } else {
        0
    };

    println!("==================================================");
    println!(" Ingestion Summary:");
    println!(" - Junctions in Neo4j:       {} (file: {})", db_junctions_count, report.junctions_loaded);
    println!(" - Relationships in Neo4j:   {} (file: {})", db_segments_count, report.segments_loaded);
    println!(" - Elapsed ingestion time:   {:.2} seconds", report.elapsed_seconds);
    println!(" - Total process time:       {:.2} seconds", start.elapsed().as_secs_f64());
    println!("==================================================");

    if db_junctions_count as usize != report.junctions_loaded || db_segments_count as usize != report.segments_loaded {
        eprintln!("[WARNING] Database counts do not match input file feature counts!");
    } else {
        println!("Database verification PASSED: 100% of nodes and relationships verified.");
    }

    Ok(())
}
