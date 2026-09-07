use std::collections::HashMap;

#[derive(Clone)]
pub struct SpatialResolver {
    landmark_map: HashMap<String, String>,
}

impl SpatialResolver {
    pub fn new() -> Self {
        let mut landmark_map = HashMap::new();

        // Direct junction IDs
        for i in 1..=8 {
            let j = format!("J{}", i);
            landmark_map.insert(j.to_lowercase(), j.clone());
            landmark_map.insert(format!("junction {}", i).to_lowercase(), j.clone());
            landmark_map.insert(format!("junction-{}", i).to_lowercase(), j);
        }

        // Landmark mappings to Junctions
        landmark_map.insert("west metro station".to_string(), "J1".to_string());
        landmark_map.insert("west sector".to_string(), "J1".to_string());
        landmark_map.insert("town hall".to_string(), "J2".to_string());
        landmark_map.insert("central square".to_string(), "J2".to_string());
        landmark_map.insert("city general hospital".to_string(), "J3".to_string());
        landmark_map.insert("hospital sector".to_string(), "J3".to_string());
        landmark_map.insert("hospital".to_string(), "J3".to_string());
        landmark_map.insert("highland water tank".to_string(), "J4".to_string());
        landmark_map.insert("north ridge".to_string(), "J4".to_string());
        landmark_map.insert("aluva river bridge".to_string(), "J5".to_string());
        landmark_map.insert("east bridge".to_string(), "J5".to_string());
        landmark_map.insert("river bridge".to_string(), "J5".to_string());
        landmark_map.insert("riverside community school".to_string(), "J6".to_string());
        landmark_map.insert("riverside lowlands".to_string(), "J6".to_string());
        landmark_map.insert("community school".to_string(), "J6".to_string());
        landmark_map.insert("riverside".to_string(), "J6".to_string());
        landmark_map.insert("south fire station".to_string(), "J7".to_string());
        landmark_map.insert("south outpost".to_string(), "J7".to_string());
        // Real Aluva Pilot Landmarks
        landmark_map.insert("aluva railway station".to_string(), "node/4664235699".to_string());
        landmark_map.insert("aluva station".to_string(), "node/4664235699".to_string());
        landmark_map.insert("railway station".to_string(), "node/4664235699".to_string());
        landmark_map.insert("aluva town hall".to_string(), "node/4664235729".to_string());
        landmark_map.insert("town hall aluva".to_string(), "node/4664235729".to_string());
        landmark_map.insert("civil station".to_string(), "node/4664235729".to_string());
        landmark_map.insert("uc college".to_string(), "node/9903123560".to_string());
        landmark_map.insert("uc college aluva".to_string(), "node/9903123560".to_string());
        landmark_map.insert("union christian college".to_string(), "node/9903123560".to_string());
        landmark_map.insert("aluva manappuram".to_string(), "node/7048449098".to_string());
        landmark_map.insert("manappuram".to_string(), "node/7048449098".to_string());
        landmark_map.insert("shiva temple aluva".to_string(), "node/7048449098".to_string());
        landmark_map.insert("manappuram temple".to_string(), "node/7048449098".to_string());
        landmark_map.insert("aluva taluk hospital".to_string(), "node/343716109".to_string());
        landmark_map.insert("taluk hospital".to_string(), "node/343716109".to_string());
        landmark_map.insert("govt hospital aluva".to_string(), "node/343716109".to_string());
        landmark_map.insert("pump junction".to_string(), "node/7992454789".to_string());
        landmark_map.insert("pump jn".to_string(), "node/7992454789".to_string());
        landmark_map.insert("bank junction".to_string(), "node/4742583964".to_string());
        landmark_map.insert("bank jn".to_string(), "node/4742583964".to_string());

        Self { landmark_map }
    }

    /// Check if a query looks like a raw node ID (e.g. node/4664235699)
    pub fn is_node_id(query: &str) -> bool {
        let q = query.trim();
        q.starts_with("node/") && q.len() > 5 && q[5..].chars().all(|c| c.is_ascii_digit())
    }

    /// Try to parse GPS coordinates (lat, lon) from unstructured text
    pub fn parse_coordinates(query: &str) -> Option<(f64, f64)> {
        let cleaned = query.replace(['(', ')', '[', ']', ';'], " ");
        let tokens: Vec<&str> = cleaned
            .split([',', ' ', '\t'])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        // Check pairs of floating point numbers
        for i in 0..tokens.len().saturating_sub(1) {
            let lat_part = tokens[i].trim_start_matches("lat:").trim_start_matches("lat=");
            let lon_part = tokens[i + 1].trim_start_matches("lon:").trim_start_matches("lon=").trim_start_matches("lng:").trim_start_matches("lng=");

            if let (Ok(lat), Ok(lon)) = (lat_part.parse::<f64>(), lon_part.parse::<f64>()) {
                // Validate plausible lat/lon range for Kerala/India (lat 8-13, lon 74-78)
                if (8.0..=13.0).contains(&lat) && (74.0..=78.0).contains(&lon) {
                    return Some((lat, lon));
                }
                // General world lat/lon validation
                if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
                    return Some((lat, lon));
                }
            }
        }

        None
    }

    /// Resolve unstructured location string to a known Junction ID
    pub fn resolve(&self, location_query: &str) -> Option<String> {
        let cleaned = location_query.trim().to_lowercase();

        // Direct node ID format
        if Self::is_node_id(&cleaned) {
            return Some(location_query.trim().to_string());
        }
        
        // Exact match
        if let Some(junction) = self.landmark_map.get(&cleaned) {
            return Some(junction.clone());
        }

        // Substring / Lexical match (longest landmark key matches first for specificity)
        let mut sorted_landmarks: Vec<(&String, &String)> = self.landmark_map.iter().collect();
        sorted_landmarks.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));

        for (landmark, junction) in sorted_landmarks {
            if cleaned.contains(landmark) || landmark.contains(&cleaned) {
                return Some(junction.clone());
            }
        }

        None
    }
}
