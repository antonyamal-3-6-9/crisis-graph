//! Normalizes raw OSM JSON data into validated directed segments and junctions GeoJSON.
//!
//! Usage:
//! cargo run --bin normalize_osm -- \
//!   --input data/raw/osm/aluva-periyar-pilot.osm.json \
//!   --boundary data/boundaries/aluva-periyar-pilot.geojson \
//!   --output-dir data/processed/aluva-periyar-pilot \
//!   --source-version "aluva-periyar-pilot-2026-09-04" \
//!   --boundary-policy intersect

use crisis_graph::geospatial::{
    junctions_to_geojson, segments_to_geojson, BoundaryPolicy, BoundaryPolygon, OsmDocument,
    OsmNormalizer,
};
use serde_json::Value;
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::PathBuf;

#[derive(Debug)]
struct Args {
    input: PathBuf,
    boundary: Option<PathBuf>,
    output_dir: PathBuf,
    source_version: String,
    boundary_policy: BoundaryPolicy,
    pilot_name: String,
}

fn usage() -> &'static str {
    "Usage: cargo run --bin normalize_osm -- \\\n  --input <raw-osm.json> \\\n  --output-dir <output-directory> \\\n  [--boundary <pilot-boundary.geojson>] \\\n  [--source-version <version-string>] \\\n  [--boundary-policy <all|intersect|strict_inside>] \\\n  [--pilot-name <name>]"
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut input = None;
    let mut boundary = None;
    let mut output_dir = None;
    let mut source_version = None;
    let mut boundary_policy = BoundaryPolicy::Intersect;
    let mut pilot_name = None;

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
            "--input" => input = Some(PathBuf::from(value)),
            "--boundary" => boundary = Some(PathBuf::from(value)),
            "--output-dir" => output_dir = Some(PathBuf::from(value)),
            "--source-version" => source_version = Some(value),
            "--boundary-policy" => boundary_policy = value.parse()?,
            "--pilot-name" => pilot_name = Some(value),
            _ => return Err(format!("Unknown option: {flag}.\n{}", usage()).into()),
        }
    }

    let input_path = input.ok_or_else(|| format!("--input is required.\n{}", usage()))?;
    let out_dir = output_dir.ok_or_else(|| format!("--output-dir is required.\n{}", usage()))?;

    let name = pilot_name.unwrap_or_else(|| {
        out_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("pilot")
            .to_string()
    });

    let version = source_version.unwrap_or_else(|| {
        format!("{name}-{}", chrono::Utc::now().format("%Y-%m-%d"))
    });

    Ok(Args {
        input: input_path,
        boundary,
        output_dir: out_dir,
        source_version: version,
        boundary_policy,
        pilot_name: name,
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;

    println!("==================================================");
    println!(" CrisisGraph OSM Normalization Pipeline");
    println!("==================================================");
    println!("Input:           {}", args.input.display());
    println!("Output dir:      {}", args.output_dir.display());
    println!("Pilot name:      {}", args.pilot_name);
    println!("Source version:  {}", args.source_version);
    println!("Boundary policy: {}", args.boundary_policy);

    // 1. Load boundary if specified
    let boundary_polygon = if let Some(ref b_path) = args.boundary {
        println!("Boundary file:   {}", b_path.display());
        let content = fs::read_to_string(b_path)?;
        let val: Value = serde_json::from_str(&content)?;
        Some(BoundaryPolygon::from_geojson(&val)?)
    } else {
        println!("Boundary file:   None (global/unbounded)");
        None
    };

    // 2. Load and parse raw OSM document
    println!("Loading raw OSM data from {}...", args.input.display());
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let doc: OsmDocument = serde_json::from_reader(reader)?;
    println!("Parsed raw OSM JSON ({} elements).", doc.elements.len());

    // 3. Run normalizer
    println!("Running normalization, topology splitting, and validation...");
    let normalizer = OsmNormalizer::new(
        &args.source_version,
        args.boundary_policy,
        boundary_polygon,
    );

    let input_str = args.input.to_string_lossy();
    let boundary_str = args.boundary.as_ref().map(|p| p.to_string_lossy().to_string());
    let (junctions, segments, report) = normalizer.normalize(
        doc,
        &input_str,
        boundary_str.as_deref(),
    );

    println!("--------------------------------------------------");
    println!(" Normalization Results:");
    println!(" - Junctions created:         {}", junctions.len());
    println!(" - Directed segments:         {}", segments.len());
    println!("   - Forward:                 {}", report.forward_segments_count);
    println!("   - Reverse:                 {}", report.reverse_segments_count);
    println!("   - One-way segments:        {}", report.oneway_segment_count);
    println!("   - Two-way pairs:           {}", report.twoway_segment_pair_count);
    println!(" - Total road network length: {:.2} km", report.total_network_length_km);
    println!(" - Mean segment length:       {:.1} m", report.mean_segment_length_m);
    println!(" - Min / Max segment length:  {:.1} m / {:.1} m", report.min_segment_length_m, report.max_segment_length_m);
    println!("--------------------------------------------------");
    println!(" Validation Checks:");
    println!(" - Dangling endpoints:        {}", report.validation.dangling_endpoints_count);
    println!(" - Duplicate segment IDs:     {}", report.validation.duplicate_segment_ids_count);
    println!(" - Zero-length segments:      {}", report.validation.zero_length_segments_count);
    println!(" - Self loops:                {}", report.validation.self_loops_count);
    println!(" - Status:                    {}", if report.validation.passed { "PASSED" } else { "FAILED" });
    if !report.validation.passed {
        for err in &report.validation.error_details {
            eprintln!("   [ERROR] {err}");
        }
        return Err("Graph validation failed.".into());
    }

    // 4. Write output files
    fs::create_dir_all(&args.output_dir)?;

    let junctions_path = args.output_dir.join("junctions.geojson");
    let segments_path = args.output_dir.join("road_segments.geojson");
    let report_path = args.output_dir.join("import_report.json");

    println!("Writing staging artifacts...");

    let junctions_geojson = junctions_to_geojson(&args.pilot_name, &junctions);
    fs::write(&junctions_path, serde_json::to_string_pretty(&junctions_geojson)? + "\n")?;
    println!("Saved junctions:       {}", junctions_path.display());

    let segments_geojson = segments_to_geojson(&args.pilot_name, &segments);
    fs::write(&segments_path, serde_json::to_string_pretty(&segments_geojson)? + "\n")?;
    println!("Saved road segments:   {}", segments_path.display());

    fs::write(&report_path, serde_json::to_string_pretty(&report)? + "\n")?;
    println!("Saved import report:   {}", report_path.display());

    println!("==================================================");
    println!(" Normalization complete!");
    println!("==================================================");

    Ok(())
}
