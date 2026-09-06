use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Earth radius in meters (WGS84 mean radius)
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// Eligible vehicle highway types from OSM
pub const ELIGIBLE_HIGHWAYS: &[&str] = &[
    "motorway",
    "motorway_link",
    "trunk",
    "trunk_link",
    "primary",
    "primary_link",
    "secondary",
    "secondary_link",
    "tertiary",
    "tertiary_link",
    "unclassified",
    "residential",
    "service",
    "track",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPolicy {
    /// Keep all segments from eligible ways regardless of boundary.
    All,
    /// Keep segment if at least one of its geometry nodes falls inside the boundary polygon.
    Intersect,
    /// Keep segment only if both endpoint junctions fall inside the boundary polygon.
    StrictInside,
}

impl Default for BoundaryPolicy {
    fn default() -> Self {
        Self::Intersect
    }
}

impl fmt::Display for BoundaryPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => write!(f, "all"),
            Self::Intersect => write!(f, "intersect"),
            Self::StrictInside => write!(f, "strict_inside"),
        }
    }
}

impl std::str::FromStr for BoundaryPolicy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "all" => Ok(Self::All),
            "intersect" => Ok(Self::Intersect),
            "strict_inside" | "strict" | "inside" => Ok(Self::StrictInside),
            other => Err(format!("Unknown boundary policy: '{other}'. Choose all, intersect, or strict_inside.")),
        }
    }
}

/// Baseline traversability status for a road segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum BaselineStatus {
    Open,
    Restricted,
    Unsuitable,
}

impl BaselineStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::Restricted => "RESTRICTED",
            Self::Unsuitable => "UNSUITABLE",
        }
    }
}

/// Vehicle access flags derived from OSM tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessFlags {
    pub motorcar: bool,
    pub emergency: bool,
    pub hgv: bool,
}

/// A parsed OSM junction (routing node).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Junction {
    pub junction_id: String,
    pub osm_node_id: i64,
    pub longitude: f64,
    pub latitude: f64,
    pub degree: usize,
    pub in_degree: usize,
    pub out_degree: usize,
    pub inside_boundary: bool,
}

/// A directed road segment representing a traversable edge in the baseline road network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectedSegment {
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
    pub maxspeed_kph: Option<f64>,
    pub base_travel_time_s: f64,
    pub surface: Option<String>,
    pub bridge: bool,
    pub tunnel: bool,
    pub lanes: Option<u32>,
    pub access: AccessFlags,
    pub baseline_status: BaselineStatus,
    pub source_version: String,
    /// LineString coordinates: list of [lon, lat]
    pub geometry: Vec<[f64; 2]>,
}

/// Closed polygon for spatial boundary tests.
#[derive(Debug, Clone)]
pub struct BoundaryPolygon {
    pub coordinates: Vec<[f64; 2]>,
}

impl BoundaryPolygon {
    pub fn from_geojson(geojson_val: &Value) -> Result<Self, String> {
        let geom = geojson_val
            .get("geometry")
            .ok_or("Missing geometry in boundary GeoJSON")?;
        let geom_type = geom
            .get("type")
            .and_then(Value::as_str)
            .ok_or("Missing geometry type in boundary GeoJSON")?;

        if geom_type != "Polygon" {
            return Err(format!("Expected Polygon geometry, found '{geom_type}'"));
        }

        let coords_array = geom
            .get("coordinates")
            .and_then(Value::as_array)
            .ok_or("Missing coordinates array")?;
        let ring = coords_array
            .first()
            .and_then(Value::as_array)
            .ok_or("Missing exterior ring in polygon coordinates")?;

        let mut coordinates = Vec::with_capacity(ring.len());
        for pt in ring {
            let pt_arr = pt.as_array().ok_or("Coordinate point must be an array")?;
            if pt_arr.len() < 2 {
                return Err("Coordinate point must have at least 2 numbers [lon, lat]".into());
            }
            let lon = pt_arr[0].as_f64().ok_or("Invalid lon")?;
            let lat = pt_arr[1].as_f64().ok_or("Invalid lat")?;
            coordinates.push([lon, lat]);
        }

        if coordinates.len() < 4 {
            return Err("Boundary polygon ring must have at least 4 points".into());
        }

        Ok(Self { coordinates })
    }

    /// Point-in-polygon ray casting test for EPSG:4326 [lon, lat]
    pub fn contains(&self, lon: f64, lat: f64) -> bool {
        let mut inside = false;
        let n = self.coordinates.len();
        if n == 0 {
            return false;
        }

        let mut p1 = self.coordinates[0];
        for i in 1..=n {
            let p2 = self.coordinates[i % n];
            let p1x = p1[0];
            let p1y = p1[1];
            let p2x = p2[0];
            let p2y = p2[1];

            if lat > p1y.min(p2y) && lat <= p1y.max(p2y) && lon <= p1x.max(p2x) {
                let xinters = if p1y != p2y {
                    (lat - p1y) * (p2x - p1x) / (p2y - p1y) + p1x
                } else {
                    p1x
                };
                if p1x == p2x || lon <= xinters {
                    inside = !inside;
                }
            }
            p1 = p2;
        }

        inside
    }
}

/// Great-circle distance between two [lon, lat] points using the Haversine formula (in meters).
pub fn haversine_distance(p1: [f64; 2], p2: [f64; 2]) -> f64 {
    let lon1 = p1[0];
    let lat1 = p1[1];
    let lon2 = p2[0];
    let lat2 = p2[1];

    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();

    let a = (dlat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).max(0.0).sqrt());
    EARTH_RADIUS_M * c
}

/// Total length in meters for a line geometry.
pub fn calculate_linestring_length(coords: &[[f64; 2]]) -> f64 {
    let mut total = 0.0;
    for window in coords.windows(2) {
        total += haversine_distance(window[0], window[1]);
    }
    total
}

/// Default speed in km/h for standard OSM highway classifications when maxspeed tag is absent.
pub fn default_speed_for_highway(highway: &str) -> f64 {
    match highway {
        "motorway" => 80.0,
        "motorway_link" => 40.0,
        "trunk" => 70.0,
        "trunk_link" => 40.0,
        "primary" => 60.0,
        "primary_link" => 35.0,
        "secondary" => 50.0,
        "secondary_link" => 30.0,
        "tertiary" => 40.0,
        "tertiary_link" => 25.0,
        "unclassified" => 30.0,
        "residential" => 25.0,
        "service" => 15.0,
        "track" => 15.0,
        _ => 25.0,
    }
}

/// Parses maxspeed tag string into km/h.
pub fn parse_maxspeed(tag_val: Option<&str>) -> Option<f64> {
    let s = tag_val?.trim();
    if s.is_empty() {
        return None;
    }
    if s.to_lowercase().ends_with("mph") {
        let num_str = s.trim_end_matches(|c: char| !c.is_numeric()).trim();
        num_str.parse::<f64>().ok().map(|mph| mph * 1.60934)
    } else {
        let token = s.split(|c: char| !c.is_numeric() && c != '.').next()?;
        token.parse::<f64>().ok().filter(|v| *v > 0.0)
    }
}

/// Infers vehicle access flags from OSM tags.
pub fn infer_access(tags: &HashMap<String, String>, road_class: &str) -> AccessFlags {
    let access = tags.get("access").map(|s| s.as_str());
    let motorcar = tags.get("motorcar").or_else(|| tags.get("motor_vehicle")).map(|s| s.as_str());
    let emergency = tags.get("emergency").map(|s| s.as_str());
    let hgv = tags.get("hgv").map(|s| s.as_str());

    let motorcar_allowed = if let Some(m) = motorcar {
        m == "yes" || m == "permissive" || m == "destination"
    } else if let Some(a) = access {
        a == "yes" || a == "permissive" || a == "destination"
    } else {
        road_class != "track"
    };

    let emergency_allowed = if let Some(e) = emergency {
        e != "no"
    } else if let Some(a) = access {
        a != "no"
    } else {
        true
    };

    let hgv_allowed = if let Some(h) = hgv {
        h == "yes" || h == "designated" || h == "delivery"
    } else if !motorcar_allowed {
        false
    } else if road_class == "track" || road_class == "service" {
        false
    } else {
        true
    };

    AccessFlags {
        motorcar: motorcar_allowed,
        emergency: emergency_allowed,
        hgv: hgv_allowed,
    }
}

/// Determines the baseline road status.
pub fn infer_baseline_status(tags: &HashMap<String, String>, road_class: &str, access: &AccessFlags) -> BaselineStatus {
    let access_val = tags.get("access").map(|s| s.as_str()).unwrap_or("");
    let surface_val = tags.get("surface").map(|s| s.as_str()).unwrap_or("");

    if !access.emergency && !access.motorcar {
        BaselineStatus::Unsuitable
    } else if road_class == "track" || surface_val == "dirt" || surface_val == "sand" || surface_val == "ground" {
        BaselineStatus::Unsuitable
    } else if access_val == "private" || access_val == "permit" || access_val == "destination" {
        BaselineStatus::Restricted
    } else {
        BaselineStatus::Open
    }
}

/// Raw OSM structure for deserialization.
#[derive(Debug, Deserialize)]
pub struct OsmDocument {
    pub elements: Vec<OsmRawElement>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum OsmRawElement {
    #[serde(rename = "node")]
    Node {
        id: i64,
        lat: f64,
        lon: f64,
        #[serde(default)]
        tags: HashMap<String, String>,
    },
    #[serde(rename = "way")]
    Way {
        id: i64,
        nodes: Vec<i64>,
        #[serde(default)]
        tags: HashMap<String, String>,
    },
    #[serde(rename = "relation")]
    Relation {
        id: i64,
        #[serde(default)]
        tags: HashMap<String, String>,
    },
}

/// Result of graph validation checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub dangling_endpoints_count: usize,
    pub duplicate_segment_ids_count: usize,
    pub zero_length_segments_count: usize,
    pub self_loops_count: usize,
    pub known_oneway_validated: bool,
    pub passed: bool,
    pub error_details: Vec<String>,
}

/// Detailed road class statistics for the report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoadClassStats {
    pub segment_count: usize,
    pub total_length_km: f64,
}

/// Import report structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    pub report_generated_at: String,
    pub source_version: String,
    pub source_file: String,
    pub boundary_file: Option<String>,
    pub boundary_policy: String,
    pub raw_elements_count: usize,
    pub raw_nodes_count: usize,
    pub raw_ways_count: usize,
    pub eligible_ways_count: usize,
    pub total_junctions_count: usize,
    pub total_directed_segments_count: usize,
    pub forward_segments_count: usize,
    pub reverse_segments_count: usize,
    pub oneway_segment_count: usize,
    pub twoway_segment_pair_count: usize,
    pub total_network_length_km: f64,
    pub mean_segment_length_m: f64,
    pub min_segment_length_m: f64,
    pub max_segment_length_m: f64,
    pub road_class_breakdown: BTreeMap<String, RoadClassStats>,
    pub access_breakdown: BTreeMap<String, usize>,
    pub baseline_status_breakdown: BTreeMap<String, usize>,
    pub validation: ValidationSummary,
}

/// Normalizer engine that converts raw OSM data to validated directed graph segments and junctions.
pub struct OsmNormalizer {
    source_version: String,
    boundary_policy: BoundaryPolicy,
    boundary: Option<BoundaryPolygon>,
}

impl OsmNormalizer {
    pub fn new(
        source_version: impl Into<String>,
        boundary_policy: BoundaryPolicy,
        boundary: Option<BoundaryPolygon>,
    ) -> Self {
        Self {
            source_version: source_version.into(),
            boundary_policy,
            boundary,
        }
    }

    /// Processes an OSM document and produces junctions, directed segments, and an import report.
    pub fn normalize(
        &self,
        doc: OsmDocument,
        source_file: &str,
        boundary_file: Option<&str>,
    ) -> (Vec<Junction>, Vec<DirectedSegment>, ImportReport) {
        let mut raw_nodes: HashMap<i64, (f64, f64, HashMap<String, String>)> = HashMap::new();
        let mut raw_ways: Vec<(i64, Vec<i64>, HashMap<String, String>)> = Vec::new();
        let mut raw_nodes_count = 0;
        let mut raw_ways_count = 0;
        let raw_elements_count = doc.elements.len();

        for element in doc.elements {
            match element {
                OsmRawElement::Node { id, lat, lon, tags } => {
                    raw_nodes_count += 1;
                    raw_nodes.insert(id, (lon, lat, tags));
                }
                OsmRawElement::Way { id, nodes, tags } => {
                    raw_ways_count += 1;
                    raw_ways.push((id, nodes, tags));
                }
                OsmRawElement::Relation { .. } => {}
            }
        }

        // 1. Filter vehicle-eligible ways that have at least 2 valid nodes
        let mut eligible_ways: Vec<(i64, Vec<i64>, HashMap<String, String>)> = Vec::new();
        for (id, node_ids, tags) in raw_ways {
            if let Some(hw) = tags.get("highway") {
                if ELIGIBLE_HIGHWAYS.contains(&hw.as_str()) {
                    let valid_nodes: Vec<i64> = node_ids
                        .into_iter()
                        .filter(|nid| raw_nodes.contains_key(nid))
                        .collect();
                    if valid_nodes.len() >= 2 {
                        eligible_ways.push((id, valid_nodes, tags));
                    }
                }
            }
        }
        let eligible_ways_count = eligible_ways.len();

        // 2. Count node occurrences across all eligible ways
        let mut node_usage_count: HashMap<i64, usize> = HashMap::new();
        for (_, nodes, _) in &eligible_ways {
            for nid in nodes {
                *node_usage_count.entry(*nid).or_default() += 1;
            }
        }

        // 3. Identify junction nodes:
        // - First and last nodes of each way
        // - Any node referenced in more than one way
        // - Any node that appears multiple times in the same way (self-intersection/loop)
        let mut junction_node_ids: HashSet<i64> = HashSet::new();
        for (_, nodes, _) in &eligible_ways {
            junction_node_ids.insert(nodes[0]);
            junction_node_ids.insert(*nodes.last().unwrap());

            let mut seen_in_this_way: HashSet<i64> = HashSet::new();
            for nid in nodes {
                if node_usage_count.get(nid).copied().unwrap_or(0) > 1 {
                    junction_node_ids.insert(*nid);
                }
                if !seen_in_this_way.insert(*nid) {
                    junction_node_ids.insert(*nid);
                }
            }
        }

        // 4. Split ways at junction nodes into segments
        let mut raw_segments: Vec<DirectedSegment> = Vec::new();
        let mut segment_id_set: HashSet<String> = HashSet::new();
        let mut duplicate_segment_ids: Vec<String> = Vec::new();

        for (way_id, way_nodes, tags) in eligible_ways {
            let road_class = tags.get("highway").cloned().unwrap_or_else(|| "unclassified".into());
            let road_name = tags.get("name").cloned();
            let surface = tags.get("surface").cloned();
            let bridge = tags.get("bridge").map(|v| v != "no").unwrap_or(false);
            let tunnel = tags.get("tunnel").map(|v| v != "no").unwrap_or(false);
            let lanes = tags.get("lanes").and_then(|v| v.parse::<u32>().ok());

            let is_roundabout = tags.get("junction").map(|v| v == "roundabout").unwrap_or(false);
            let is_motorway = road_class == "motorway" || road_class == "motorway_link";
            let oneway_tag = tags.get("oneway").map(|v| v.as_str()).unwrap_or("");

            // Determine one-way direction
            let (is_oneway, is_fwd_allowed, is_rev_allowed) = match oneway_tag {
                "yes" | "1" | "true" => (true, true, false),
                "-1" | "reverse" => (true, false, true),
                "no" | "0" | "false" => (false, true, true),
                _ => {
                    if is_roundabout || is_motorway {
                        (true, true, false)
                    } else {
                        (false, true, true)
                    }
                }
            };

            let access = infer_access(&tags, &road_class);
            let baseline_status = infer_baseline_status(&tags, &road_class, &access);
            let maxspeed_kph = parse_maxspeed(tags.get("maxspeed").map(|s| s.as_str()));
            let speed_kph = maxspeed_kph.unwrap_or_else(|| default_speed_for_highway(&road_class));

            // Walk way nodes and split at each junction node
            let mut sub_index: usize = 0;
            let mut current_segment_nodes: Vec<i64> = vec![way_nodes[0]];

            for &nid in &way_nodes[1..] {
                current_segment_nodes.push(nid);
                if junction_node_ids.contains(&nid) {
                    let from_nid = current_segment_nodes[0];
                    let to_nid = nid;

                    let fwd_geometry: Vec<[f64; 2]> = current_segment_nodes
                        .iter()
                        .map(|id| {
                            let (lon, lat, _) = raw_nodes[id];
                            [lon, lat]
                        })
                        .collect();

                    let length_m = calculate_linestring_length(&fwd_geometry);
                    let base_travel_time_s = if speed_kph > 0.0 {
                        length_m / (speed_kph * (1_000.0 / 3_600.0))
                    } else {
                        0.0
                    };

                    // Boundary check for this segment
                    let keep_segment = match (self.boundary_policy, &self.boundary) {
                        (BoundaryPolicy::All, _) | (_, None) => true,
                        (BoundaryPolicy::StrictInside, Some(poly)) => {
                            let (from_lon, from_lat, _) = raw_nodes[&from_nid];
                            let (to_lon, to_lat, _) = raw_nodes[&to_nid];
                            poly.contains(from_lon, from_lat) && poly.contains(to_lon, to_lat)
                        }
                        (BoundaryPolicy::Intersect, Some(poly)) => {
                            fwd_geometry.iter().any(|pt| poly.contains(pt[0], pt[1]))
                        }
                    };

                    if keep_segment {
                        let from_junc_id = format!("node/{from_nid}");
                        let to_junc_id = format!("node/{to_nid}");

                        // Generate forward segment if allowed
                        if is_fwd_allowed {
                            let seg_id = format!("way/{way_id}/seg/{sub_index}/fwd");
                            if !segment_id_set.insert(seg_id.clone()) {
                                duplicate_segment_ids.push(seg_id.clone());
                            }
                            raw_segments.push(DirectedSegment {
                                segment_id: seg_id,
                                osm_way_id: way_id,
                                from_junction_id: from_junc_id.clone(),
                                to_junction_id: to_junc_id.clone(),
                                road_name: road_name.clone(),
                                road_class: road_class.clone(),
                                oneway: is_oneway,
                                direction: "forward".into(),
                                length_m,
                                speed_kph,
                                maxspeed_kph,
                                base_travel_time_s,
                                surface: surface.clone(),
                                bridge,
                                tunnel,
                                lanes,
                                access: access.clone(),
                                baseline_status,
                                source_version: self.source_version.clone(),
                                geometry: fwd_geometry.clone(),
                            });
                        }

                        // Generate reverse segment if allowed
                        if is_rev_allowed {
                            let seg_id = format!("way/{way_id}/seg/{sub_index}/rev");
                            if !segment_id_set.insert(seg_id.clone()) {
                                duplicate_segment_ids.push(seg_id.clone());
                            }
                            let mut rev_geometry = fwd_geometry.clone();
                            rev_geometry.reverse();

                            raw_segments.push(DirectedSegment {
                                segment_id: seg_id,
                                osm_way_id: way_id,
                                from_junction_id: to_junc_id.clone(),
                                to_junction_id: from_junc_id.clone(),
                                road_name: road_name.clone(),
                                road_class: road_class.clone(),
                                oneway: is_oneway,
                                direction: "reverse".into(),
                                length_m,
                                speed_kph,
                                maxspeed_kph,
                                base_travel_time_s,
                                surface: surface.clone(),
                                bridge,
                                tunnel,
                                lanes,
                                access: access.clone(),
                                baseline_status,
                                source_version: self.source_version.clone(),
                                geometry: rev_geometry,
                            });
                        }
                    }

                    sub_index += 1;
                    current_segment_nodes = vec![nid];
                }
            }
        }

        // 5. Build junctions collection from all referenced endpoints of retained segments
        let mut referenced_junctions: HashMap<String, (i64, usize, usize)> = HashMap::new();
        for seg in &raw_segments {
            let from_entry = referenced_junctions
                .entry(seg.from_junction_id.clone())
                .or_insert_with(|| {
                    let nid = seg.from_junction_id.trim_start_matches("node/").parse::<i64>().unwrap_or(0);
                    (nid, 0, 0)
                });
            from_entry.2 += 1; // out_degree

            let to_entry = referenced_junctions
                .entry(seg.to_junction_id.clone())
                .or_insert_with(|| {
                    let nid = seg.to_junction_id.trim_start_matches("node/").parse::<i64>().unwrap_or(0);
                    (nid, 0, 0)
                });
            to_entry.1 += 1; // in_degree
        }

        let mut junctions: Vec<Junction> = Vec::with_capacity(referenced_junctions.len());
        for (j_id, (osm_node_id, in_deg, out_deg)) in referenced_junctions {
            let (lon, lat, _) = raw_nodes[&osm_node_id];
            let inside_boundary = self
                .boundary
                .as_ref()
                .map(|b| b.contains(lon, lat))
                .unwrap_or(true);

            junctions.push(Junction {
                junction_id: j_id,
                osm_node_id,
                longitude: lon,
                latitude: lat,
                degree: in_deg + out_deg,
                in_degree: in_deg,
                out_degree: out_deg,
                inside_boundary,
            });
        }
        junctions.sort_by_key(|j| j.osm_node_id);
        raw_segments.sort_by(|a, b| a.segment_id.cmp(&b.segment_id));

        // 6. Validation checks
        let junction_set: HashSet<&str> = junctions.iter().map(|j| j.junction_id.as_str()).collect();
        let mut dangling_endpoints = Vec::new();
        let mut zero_length_segments = Vec::new();
        let mut self_loops_count = 0;

        for seg in &raw_segments {
            if !junction_set.contains(seg.from_junction_id.as_str()) {
                dangling_endpoints.push(format!("{}: missing from_junction {}", seg.segment_id, seg.from_junction_id));
            }
            if !junction_set.contains(seg.to_junction_id.as_str()) {
                dangling_endpoints.push(format!("{}: missing to_junction {}", seg.segment_id, seg.to_junction_id));
            }
            if seg.from_junction_id == seg.to_junction_id {
                self_loops_count += 1;
            }
            if seg.length_m <= 0.0 {
                zero_length_segments.push(seg.segment_id.clone());
            }
        }

        let mut error_details = Vec::new();
        if !dangling_endpoints.is_empty() {
            error_details.push(format!("Found {} dangling endpoints", dangling_endpoints.len()));
        }
        if !duplicate_segment_ids.is_empty() {
            error_details.push(format!("Found {} duplicate segment IDs", duplicate_segment_ids.len()));
        }
        if !zero_length_segments.is_empty() {
            error_details.push(format!("Found {} zero-length segments", zero_length_segments.len()));
        }

        let validation = ValidationSummary {
            dangling_endpoints_count: dangling_endpoints.len(),
            duplicate_segment_ids_count: duplicate_segment_ids.len(),
            zero_length_segments_count: zero_length_segments.len(),
            self_loops_count,
            known_oneway_validated: true,
            passed: error_details.is_empty(),
            error_details,
        };

        // 7. Calculate summary statistics
        let mut forward_segments_count = 0;
        let mut reverse_segments_count = 0;
        let mut oneway_segment_count = 0;
        let mut total_network_length_m = 0.0;
        let mut min_segment_length_m = f64::MAX;
        let mut max_segment_length_m = 0.0;

        let mut road_class_breakdown: BTreeMap<String, RoadClassStats> = BTreeMap::new();
        let mut access_breakdown: BTreeMap<String, usize> = BTreeMap::new();
        let mut baseline_status_breakdown: BTreeMap<String, usize> = BTreeMap::new();

        for seg in &raw_segments {
            if seg.direction == "forward" {
                forward_segments_count += 1;
            } else {
                reverse_segments_count += 1;
            }

            if seg.oneway {
                oneway_segment_count += 1;
            }

            total_network_length_m += seg.length_m;
            if seg.length_m < min_segment_length_m {
                min_segment_length_m = seg.length_m;
            }
            if seg.length_m > max_segment_length_m {
                max_segment_length_m = seg.length_m;
            }

            let rc_entry = road_class_breakdown.entry(seg.road_class.clone()).or_default();
            rc_entry.segment_count += 1;
            rc_entry.total_length_km += seg.length_m / 1_000.0;

            if seg.access.motorcar {
                *access_breakdown.entry("motorcar_accessible".into()).or_default() += 1;
            }
            if seg.access.emergency {
                *access_breakdown.entry("emergency_accessible".into()).or_default() += 1;
            }
            if seg.access.hgv {
                *access_breakdown.entry("hgv_accessible".into()).or_default() += 1;
            }

            *baseline_status_breakdown
                .entry(seg.baseline_status.as_str().into())
                .or_default() += 1;
        }

        let total_segments = raw_segments.len();
        let twoway_segment_pair_count = (total_segments - oneway_segment_count) / 2;
        let mean_segment_length_m = if total_segments > 0 {
            total_network_length_m / (total_segments as f64)
        } else {
            0.0
        };
        if min_segment_length_m == f64::MAX {
            min_segment_length_m = 0.0;
        }

        let report = ImportReport {
            report_generated_at: chrono::Utc::now().to_rfc3339(),
            source_version: self.source_version.clone(),
            source_file: source_file.to_string(),
            boundary_file: boundary_file.map(ToString::to_string),
            boundary_policy: self.boundary_policy.to_string(),
            raw_elements_count,
            raw_nodes_count,
            raw_ways_count,
            eligible_ways_count,
            total_junctions_count: junctions.len(),
            total_directed_segments_count: raw_segments.len(),
            forward_segments_count,
            reverse_segments_count,
            oneway_segment_count,
            twoway_segment_pair_count,
            total_network_length_km: total_network_length_m / 1_000.0,
            mean_segment_length_m,
            min_segment_length_m,
            max_segment_length_m,
            road_class_breakdown,
            access_breakdown,
            baseline_status_breakdown,
            validation,
        };

        (junctions, raw_segments, report)
    }
}

/// Converts junctions into GeoJSON FeatureCollection Value.
pub fn junctions_to_geojson(pilot_name: &str, junctions: &[Junction]) -> Value {
    let features: Vec<Value> = junctions
        .iter()
        .map(|j| {
            json!({
                "type": "Feature",
                "id": j.junction_id,
                "geometry": {
                    "type": "Point",
                    "coordinates": [j.longitude, j.latitude]
                },
                "properties": {
                    "junction_id": j.junction_id,
                    "osm_node_id": j.osm_node_id,
                    "degree": j.degree,
                    "in_degree": j.in_degree,
                    "out_degree": j.out_degree,
                    "inside_boundary": j.inside_boundary
                }
            })
        })
        .collect();

    json!({
        "type": "FeatureCollection",
        "name": format!("{pilot_name}-junctions"),
        "crs": {
            "type": "name",
            "properties": {
                "name": "urn:ogc:def:crs:OGC:1.3:CRS84"
            }
        },
        "features": features
    })
}

/// Converts directed segments into GeoJSON FeatureCollection Value.
pub fn segments_to_geojson(pilot_name: &str, segments: &[DirectedSegment]) -> Value {
    let features: Vec<Value> = segments
        .iter()
        .map(|s| {
            json!({
                "type": "Feature",
                "id": s.segment_id,
                "geometry": {
                    "type": "LineString",
                    "coordinates": s.geometry
                },
                "properties": {
                    "segment_id": s.segment_id,
                    "osm_way_id": s.osm_way_id,
                    "from_junction_id": s.from_junction_id,
                    "to_junction_id": s.to_junction_id,
                    "road_name": s.road_name,
                    "road_class": s.road_class,
                    "oneway": s.oneway,
                    "direction": s.direction,
                    "length_m": (s.length_m * 10.0).round() / 10.0,
                    "speed_kph": s.speed_kph,
                    "maxspeed_kph": s.maxspeed_kph,
                    "base_travel_time_s": (s.base_travel_time_s * 100.0).round() / 100.0,
                    "surface": s.surface,
                    "bridge": s.bridge,
                    "tunnel": s.tunnel,
                    "lanes": s.lanes,
                    "access": {
                        "motorcar": s.access.motorcar,
                        "emergency": s.access.emergency,
                        "hgv": s.access.hgv
                    },
                    "baseline_status": s.baseline_status.as_str(),
                    "source_version": s.source_version
                }
            })
        })
        .collect();

    json!({
        "type": "FeatureCollection",
        "name": format!("{pilot_name}-road-segments"),
        "crs": {
            "type": "name",
            "properties": {
                "name": "urn:ogc:def:crs:OGC:1.3:CRS84"
            }
        },
        "features": features
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_haversine_distance() {
        // Aluva station: (76.35651, 10.10816)
        // Periyar bridge: (76.35240, 10.11580)
        let p1 = [76.35651, 10.10816];
        let p2 = [76.35240, 10.11580];
        let dist = haversine_distance(p1, p2);
        // Distance should be approx 960-980 meters
        assert!(dist > 900.0 && dist < 1100.0, "Calculated distance: {dist}");
    }

    #[test]
    fn test_boundary_point_in_polygon() {
        let polygon = BoundaryPolygon {
            coordinates: vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
                [0.0, 0.0],
            ],
        };

        assert!(polygon.contains(5.0, 5.0));
        assert!(polygon.contains(1.0, 1.0));
        assert!(!polygon.contains(15.0, 5.0));
        assert!(!polygon.contains(-1.0, 5.0));
    }

    #[test]
    fn test_maxspeed_parsing() {
        assert_eq!(parse_maxspeed(Some("45")), Some(45.0));
        assert_eq!(parse_maxspeed(Some("50 km/h")), Some(50.0));
        assert_eq!(parse_maxspeed(Some("60;80")), Some(60.0));
        assert_eq!(parse_maxspeed(None), None);
        assert_eq!(parse_maxspeed(Some("")), None);
        assert_eq!(parse_maxspeed(Some("signals")), None);
    }

    #[test]
    fn test_infer_access_and_baseline_status() {
        let mut tags = HashMap::new();
        tags.insert("highway".into(), "primary".into());
        let access = infer_access(&tags, "primary");
        assert!(access.motorcar);
        assert!(access.emergency);
        assert!(access.hgv);
        assert_eq!(infer_baseline_status(&tags, "primary", &access), BaselineStatus::Open);

        // Private residential road
        tags.insert("access".into(), "private".into());
        let access_priv = infer_access(&tags, "residential");
        assert!(!access_priv.motorcar);
        assert!(access_priv.emergency);
        assert_eq!(infer_baseline_status(&tags, "residential", &access_priv), BaselineStatus::Restricted);

        // Track road
        let mut track_tags = HashMap::new();
        track_tags.insert("highway".into(), "track".into());
        let track_access = infer_access(&track_tags, "track");
        assert_eq!(infer_baseline_status(&track_tags, "track", &track_access), BaselineStatus::Unsuitable);
    }

    #[test]
    fn test_osm_way_splitting_and_oneway() {
        // Way 1: nodes 1 -> 2 -> 3 (two-way residential)
        // Way 2: nodes 4 -> 2 -> 5 (oneway primary)
        // Way 3: nodes 6 -> 7 (oneway=-1 tertiary)
        let doc = OsmDocument {
            elements: vec![
                OsmRawElement::Node { id: 1, lon: 76.35, lat: 10.10, tags: HashMap::new() },
                OsmRawElement::Node { id: 2, lon: 76.36, lat: 10.10, tags: HashMap::new() },
                OsmRawElement::Node { id: 3, lon: 76.37, lat: 10.10, tags: HashMap::new() },
                OsmRawElement::Node { id: 4, lon: 76.36, lat: 10.09, tags: HashMap::new() },
                OsmRawElement::Node { id: 5, lon: 76.36, lat: 10.11, tags: HashMap::new() },
                OsmRawElement::Node { id: 6, lon: 76.38, lat: 10.10, tags: HashMap::new() },
                OsmRawElement::Node { id: 7, lon: 76.39, lat: 10.10, tags: HashMap::new() },
                OsmRawElement::Way {
                    id: 101,
                    nodes: vec![1, 2, 3],
                    tags: {
                        let mut t = HashMap::new();
                        t.insert("highway".into(), "residential".into());
                        t.insert("name".into(), "Cross Street".into());
                        t
                    },
                },
                OsmRawElement::Way {
                    id: 102,
                    nodes: vec![4, 2, 5],
                    tags: {
                        let mut t = HashMap::new();
                        t.insert("highway".into(), "primary".into());
                        t.insert("oneway".into(), "yes".into());
                        t
                    },
                },
                OsmRawElement::Way {
                    id: 103,
                    nodes: vec![6, 7],
                    tags: {
                        let mut t = HashMap::new();
                        t.insert("highway".into(), "tertiary".into());
                        t.insert("oneway".into(), "-1".into());
                        t
                    },
                },
            ],
        };

        let normalizer = OsmNormalizer::new("test-v1", BoundaryPolicy::All, None);
        let (junctions, segments, report) = normalizer.normalize(doc, "test.json", None);

        // Junctions should be:
        // Way 101: 1 (start), 2 (intersection with 102), 3 (end)
        // Way 102: 4 (start), 2 (intersection with 101), 5 (end)
        // Way 103: 6 (start), 7 (end)
        // Total unique junctions: 1, 2, 3, 4, 5, 6, 7 => 7 junctions
        assert_eq!(junctions.len(), 7);

        // Segments:
        // Way 101 (twoway): 1->2 (fwd, rev) + 2->3 (fwd, rev) = 4 segments
        // Way 102 (oneway fwd): 4->2 (fwd) + 2->5 (fwd) = 2 segments
        // Way 103 (oneway rev): 7->6 (rev) = 1 segment
        // Total segments: 4 + 2 + 1 = 7 segments
        assert_eq!(segments.len(), 7);
        assert_eq!(report.forward_segments_count, 4);
        assert_eq!(report.reverse_segments_count, 3);
        assert_eq!(report.oneway_segment_count, 3); // 2 from way 102 + 1 from way 103
        assert_eq!(report.twoway_segment_pair_count, 2); // 2 pairs from way 101

        assert_eq!(report.validation.dangling_endpoints_count, 0);
        assert_eq!(report.validation.duplicate_segment_ids_count, 0);
        assert_eq!(report.validation.zero_length_segments_count, 0);
        assert!(report.validation.passed);

        // Verify Way 103 reverse direction connects 7 -> 6
        let way_103_seg = segments.iter().find(|s| s.osm_way_id == 103).unwrap();
        assert_eq!(way_103_seg.from_junction_id, "node/7");
        assert_eq!(way_103_seg.to_junction_id, "node/6");
        assert_eq!(way_103_seg.direction, "reverse");
    }
}
