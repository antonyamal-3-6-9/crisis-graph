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
        landmark_map.insert("harbor gate 4".to_string(), "J8".to_string());
        landmark_map.insert("coastline depot".to_string(), "J8".to_string());

        Self { landmark_map }
    }

    /// Resolve unstructured location string to a known Junction ID
    pub fn resolve(&self, location_query: &str) -> Option<String> {
        let cleaned = location_query.trim().to_lowercase();
        
        // Exact match
        if let Some(junction) = self.landmark_map.get(&cleaned) {
            return Some(junction.clone());
        }

        // Substring / Lexical match
        for (landmark, junction) in &self.landmark_map {
            if cleaned.contains(landmark) || landmark.contains(&cleaned) {
                return Some(junction.clone());
            }
        }

        None
    }
}
