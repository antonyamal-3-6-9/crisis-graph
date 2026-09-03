use neo4rs::query;
use super::neo4j::Neo4jClient;
use tracing::info;

pub async fn seed_database(neo4j: &Neo4jClient) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Seeding Neo4j disaster graph schema and initial topology...");

    // 1. Create uniqueness constraints & indexes
    let constraints = vec![
        "CREATE CONSTRAINT junction_id_unique IF NOT EXISTS FOR (j:Junction) REQUIRE j.id IS UNIQUE",
        "CREATE CONSTRAINT shelter_id_unique IF NOT EXISTS FOR (s:Shelter) REQUIRE s.id IS UNIQUE",
    ];

    for c in constraints {
        neo4j.graph.run(query(c)).await?;
    }

    // 2. Clear old data if needed or merge nodes
    let seed_cypher = r#"
    // Create Junctions
    MERGE (j1:Junction {id: 'J1', name: 'West Sector Hub', landmark: 'West Metro Station'})
    MERGE (j2:Junction {id: 'J2', name: 'Central Square', landmark: 'Town Hall'})
    MERGE (j3:Junction {id: 'J3', name: 'Hospital Sector', landmark: 'City General Hospital'})
    MERGE (j4:Junction {id: 'J4', name: 'North Ridge', landmark: 'Highland Water Tank'})
    MERGE (j5:Junction {id: 'J5', name: 'East Bridge', landmark: 'Aluva River Bridge'})
    MERGE (j6:Junction {id: 'J6', name: 'Riverside Lowlands', landmark: 'Riverside Community School'})
    MERGE (j7:Junction {id: 'J7', name: 'South Outpost', landmark: 'South Fire Station'})
    MERGE (j8:Junction {id: 'J8', name: 'Coastline Depot', landmark: 'Harbor Gate 4'})

    // Create Bidirectional Road Connections with Base Distances
    MERGE (j1)-[r1:CONNECTS_TO {base_distance_km: 3.2, status: 'OPEN', active_weight: 3.2, valid_until: datetime()}]->(j2)
    MERGE (j2)-[r2:CONNECTS_TO {base_distance_km: 3.2, status: 'OPEN', active_weight: 3.2, valid_until: datetime()}]->(j1)

    MERGE (j2)-[r3:CONNECTS_TO {base_distance_km: 2.1, status: 'OPEN', active_weight: 2.1, valid_until: datetime()}]->(j3)
    MERGE (j3)-[r4:CONNECTS_TO {base_distance_km: 2.1, status: 'OPEN', active_weight: 2.1, valid_until: datetime()}]->(j2)

    MERGE (j1)-[r5:CONNECTS_TO {base_distance_km: 4.5, status: 'OPEN', active_weight: 4.5, valid_until: datetime()}]->(j4)
    MERGE (j4)-[r6:CONNECTS_TO {base_distance_km: 4.5, status: 'OPEN', active_weight: 4.5, valid_until: datetime()}]->(j1)

    MERGE (j4)-[r7:CONNECTS_TO {base_distance_km: 3.8, status: 'OPEN', active_weight: 3.8, valid_until: datetime()}]->(j5)
    MERGE (j5)-[r8:CONNECTS_TO {base_distance_km: 3.8, status: 'OPEN', active_weight: 3.8, valid_until: datetime()}]->(j4)

    MERGE (j5)-[r9:CONNECTS_TO {base_distance_km: 2.7, status: 'OPEN', active_weight: 2.7, valid_until: datetime()}]->(j6)
    MERGE (j6)-[r10:CONNECTS_TO {base_distance_km: 2.7, status: 'OPEN', active_weight: 2.7, valid_until: datetime()}]->(j5)

    MERGE (j3)-[r11:CONNECTS_TO {base_distance_km: 5.0, status: 'OPEN', active_weight: 5.0, valid_until: datetime()}]->(j6)
    MERGE (j6)-[r12:CONNECTS_TO {base_distance_km: 5.0, status: 'OPEN', active_weight: 5.0, valid_until: datetime()}]->(j3)

    MERGE (j2)-[r13:CONNECTS_TO {base_distance_km: 4.1, status: 'OPEN', active_weight: 4.1, valid_until: datetime()}]->(j7)
    MERGE (j7)-[r14:CONNECTS_TO {base_distance_km: 4.1, status: 'OPEN', active_weight: 4.1, valid_until: datetime()}]->(j2)

    MERGE (j7)-[r15:CONNECTS_TO {base_distance_km: 6.2, status: 'OPEN', active_weight: 6.2, valid_until: datetime()}]->(j8)
    MERGE (j8)-[r16:CONNECTS_TO {base_distance_km: 6.2, status: 'OPEN', active_weight: 6.2, valid_until: datetime()}]->(j7)

    // Create Shelters
    MERGE (s1:Shelter {
        id: 'S1',
        name: 'West Sector Relief Complex',
        junction_id: 'J1',
        capacity: 150,
        current_occupancy: 20,
        boats_available: 4,
        ambulances_available: 2,
        trucks_available: 3,
        last_updated: datetime()
    })
    MERGE (s1)-[:LOCATED_AT]->(j1)

    MERGE (s2:Shelter {
        id: 'S2',
        name: 'North Highland Emergency Camp',
        junction_id: 'J4',
        capacity: 100,
        current_occupancy: 10,
        boats_available: 2,
        ambulances_available: 3,
        trucks_available: 2,
        last_updated: datetime()
    })
    MERGE (s2)-[:LOCATED_AT]->(j4)

    MERGE (s3:Shelter {
        id: 'S3',
        name: 'South Fire & Rescue Base',
        junction_id: 'J7',
        capacity: 200,
        current_occupancy: 45,
        boats_available: 5,
        ambulances_available: 4,
        trucks_available: 5,
        last_updated: datetime()
    })
    MERGE (s3)-[:LOCATED_AT]->(j7)
    "#;

    neo4j.graph.run(query(seed_cypher)).await?;
    info!("Disaster graph seeded successfully with 8 junctions and 3 shelters.");

    Ok(())
}
