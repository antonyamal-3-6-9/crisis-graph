//! Generates a geodesic circular GeoJSON boundary for a CrisisGraph pilot area.
//!
//! Example:
//! cargo run --bin generate_boundary -- \
//!   --name aluva-periyar-pilot \
//!   --lat 10.10816 --lon 76.35651 --radius-km 10 \
//!   --output data/boundaries/aluva-periyar-pilot.geojson

use std::env;
use std::error::Error;
use std::f64::consts::PI;
use std::fs;
use std::path::Path;

const EARTH_RADIUS_M: f64 = 6_371_008.8;
const DEFAULT_SEGMENTS: usize = 64;

#[derive(Debug)]
struct Args {
    name: String,
    latitude: f64,
    longitude: f64,
    radius_km: f64,
    output: String,
    segments: usize,
}

fn usage() -> &'static str {
    "Usage: cargo run --bin generate_boundary -- \\  --name <pilot-id> --lat <latitude> --lon <longitude> \\  --radius-km <positive number> --output <file.geojson> [--segments <minimum 8>]"
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut name = None;
    let mut latitude = None;
    let mut longitude = None;
    let mut radius_km = None;
    let mut output = None;
    let mut segments = DEFAULT_SEGMENTS;

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
            "--name" => name = Some(value),
            "--lat" => latitude = Some(value.parse::<f64>()?),
            "--lon" => longitude = Some(value.parse::<f64>()?),
            "--radius-km" => radius_km = Some(value.parse::<f64>()?),
            "--output" => output = Some(value),
            "--segments" => segments = value.parse::<usize>()?,
            _ => return Err(format!("Unknown option: {flag}.\n{}", usage()).into()),
        }
    }

    let args = Args {
        name: name.ok_or_else(|| format!("--name is required.\n{}", usage()))?,
        latitude: latitude.ok_or_else(|| format!("--lat is required.\n{}", usage()))?,
        longitude: longitude.ok_or_else(|| format!("--lon is required.\n{}", usage()))?,
        radius_km: radius_km.ok_or_else(|| format!("--radius-km is required.\n{}", usage()))?,
        output: output.ok_or_else(|| format!("--output is required.\n{}", usage()))?,
        segments,
    };

    if !(-90.0..=90.0).contains(&args.latitude) {
        return Err("Latitude must be between -90 and 90.".into());
    }
    if !(-180.0..=180.0).contains(&args.longitude) {
        return Err("Longitude must be between -180 and 180.".into());
    }
    if args.radius_km <= 0.0 {
        return Err("Radius must be positive.".into());
    }
    if args.segments < 8 {
        return Err("--segments must be at least 8.".into());
    }

    Ok(args)
}

fn destination_point(
    latitude_deg: f64,
    longitude_deg: f64,
    bearing_rad: f64,
    distance_m: f64,
) -> (f64, f64) {
    let latitude_1 = latitude_deg.to_radians();
    let longitude_1 = longitude_deg.to_radians();
    let angular_distance = distance_m / EARTH_RADIUS_M;

    let latitude_2 = (latitude_1.sin() * angular_distance.cos()
        + latitude_1.cos() * angular_distance.sin() * bearing_rad.cos())
    .asin();

    let longitude_2 = longitude_1
        + (bearing_rad.sin() * angular_distance.sin() * latitude_1.cos())
            .atan2(angular_distance.cos() - latitude_1.sin() * latitude_2.sin());

    let normalized_longitude = (longitude_2 + 3.0 * PI).rem_euclid(2.0 * PI) - PI;
    (normalized_longitude.to_degrees(), latitude_2.to_degrees())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let radius_m = args.radius_km * 1_000.0;

    let mut ring = Vec::with_capacity(args.segments + 1);
    for index in 0..=args.segments {
        let bearing = 2.0 * PI * (index % args.segments) as f64 / args.segments as f64;
        let (longitude, latitude) =
            destination_point(args.latitude, args.longitude, bearing, radius_m);
        ring.push(vec![longitude, latitude]);
    }

    let feature = serde_json::json!({
        "type": "Feature",
        "properties": {
            "id": format!("{}-v1", args.name),
            "name": args.name,
            "centre": {
                "latitude": args.latitude,
                "longitude": args.longitude,
            },
            "radius_km": args.radius_km,
            "crs": "EPSG:4326",
            "generated_by": "crisis-graph generate_boundary",
            "segments": args.segments,
        },
        "geometry": {
            "type": "Polygon",
            "coordinates": [ring],
        },
    });

    let output_path = Path::new(&args.output);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, serde_json::to_string_pretty(&feature)? + "\n")?;

    println!(
        "Created {}: {} km geodesic boundary around ({:.5}, {:.5}) with {} segments.",
        output_path.display(),
        args.radius_km,
        args.latitude,
        args.longitude,
        args.segments
    );
    Ok(())
}

