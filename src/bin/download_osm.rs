//! Downloads raw OSM road data for a CrisisGraph pilot boundary through Overpass.
//!
//! Example:
//! cargo run --bin download_osm -- \
//!   --boundary data/boundaries/aluva-periyar-pilot.geojson \
//!   --output data/raw/osm/aluva-periyar-pilot.osm.json

use serde::Deserialize;
use serde_json::Value;
use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

const DEFAULT_ENDPOINT: &str = "https://overpass-api.de/api/interpreter";

#[derive(Debug)]
struct Args {
    boundary: String,
    output: String,
    endpoint: String,
}

#[derive(Debug, Deserialize)]
struct BoundaryFile {
    properties: BoundaryProperties,
}

#[derive(Debug, Deserialize)]
struct BoundaryProperties {
    centre: Centre,
    radius_km: f64,
}

#[derive(Debug, Deserialize)]
struct Centre {
    latitude: f64,
    longitude: f64,
}

fn usage() -> &'static str {
    "Usage: cargo run --bin download_osm -- \\  --boundary <pilot-boundary.geojson> --output <raw.osm.json> \\  [--endpoint <overpass-interpreter-url>]"
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut boundary = None;
    let mut output = None;
    let mut endpoint = DEFAULT_ENDPOINT.to_string();
    let mut values = env::args().skip(1);

    while let Some(flag) = values.next() {
        if flag == "--help" || flag == "-h" {
            println!("{}", usage());
            std::process::exit(0);
        }

        let value = values
            .next()
            .ok_or_else(|| format!("Missing value for {flag}.\n{}", usage()))?;

        match flag.as_str() {
            "--boundary" => boundary = Some(value),
            "--output" => output = Some(value),
            "--endpoint" => endpoint = value,
            _ => return Err(format!("Unknown option: {flag}.\n{}", usage()).into()),
        }
    }

    Ok(Args {
        boundary: boundary.ok_or_else(|| format!("--boundary is required.\n{}", usage()))?,
        output: output.ok_or_else(|| format!("--output is required.\n{}", usage()))?,
        endpoint,
    })
}

fn build_query(latitude: f64, longitude: f64, radius_m: u64) -> String {
    // The query returns road ways and every node referenced by them. The raw result
    // is preserved; filtering to directed routing segments happens in a later stage.
    format!(
        r#"[out:json][timeout:180];
way(around:{radius_m},{latitude:.7},{longitude:.7})
  ["highway"~"^(motorway|motorway_link|trunk|trunk_link|primary|primary_link|secondary|secondary_link|tertiary|tertiary_link|unclassified|residential|service|track)$"];
out body;
>;
out skel qt;"#
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let boundary_text = fs::read_to_string(&args.boundary)?;
    let boundary: BoundaryFile = serde_json::from_str(&boundary_text)?;

    if boundary.properties.radius_km <= 0.0 {
        return Err("Boundary radius_km must be positive.".into());
    }

    let radius_m = (boundary.properties.radius_km * 1_000.0).round() as u64;
    let query = build_query(
        boundary.properties.centre.latitude,
        boundary.properties.centre.longitude,
        radius_m,
    );

    println!(
        "Requesting OSM roads within {} m of ({:.5}, {:.5})...",
        radius_m, boundary.properties.centre.latitude, boundary.properties.centre.longitude
    );

    let client = reqwest::Client::builder()
        .user_agent("CrisisGraph/0.1 pilot OSM downloader")
        .timeout(std::time::Duration::from_secs(240))
        .build()?;
    let response = client
        .post(&args.endpoint)
        .form(&[("data", query.as_str())])
        .send()
        .await?
        .error_for_status()?;
    let raw_json = response.text().await?;

    // Reject HTML/error pages before they are saved as map data.
    let document: Value = serde_json::from_str(&raw_json)?;
    let element_count = document
        .get("elements")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    if element_count == 0 {
        return Err("Overpass returned zero elements; refusing to save an empty map dataset.".into());
    }

    let output_path = Path::new(&args.output);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, serde_json::to_string_pretty(&document)? + "\n")?;

    println!(
        "Saved {} OSM elements to {}.",
        element_count,
        output_path.display()
    );
    Ok(())
}

