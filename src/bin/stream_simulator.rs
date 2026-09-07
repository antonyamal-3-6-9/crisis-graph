use chrono::Utc;
use uuid::Uuid;
use crisis_graph::config::Config;
use crisis_graph::db::RedisClient;
use crisis_graph::models::SosAlert;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("============================================================");
    println!("     CRISISGRAPH - DISASTER STREAM LOAD SIMULATOR           ");
    println!("         Injecting Concurrent Alerts into Redis             ");
    println!("============================================================");

    let config = Config::from_env();
    let redis = RedisClient::connect(&config).await?;
    redis.ping().await?;

    let stream_key = "sos:stream:aluva";
    println!("Connected to Redis. Publishing to stream: {stream_key}\n");

    let alerts = [
        "URGENT: Water entering ground floor at (lat: 10.1135, lon: 76.3540) near Pump Junction. 4 persons trapped, ambulance needed!",
        "Aluva Railway Station has 3 injured passengers, need ambulance dispatch immediately.",
        "Water entering Aluva Manappuram temple grounds, 12 pilgrims stranded on temple steps. Urgent boat rescue needed!",
        "House partially submerged near Bank Junction (lat: 10.1082, lon: 76.3565). 2 elderly persons need rescue.",
        "Flash flood at UC College gate 2, 6 students stranded in water, need evacuation truck.",
    ];

    for (i, alert_text) in alerts.iter().enumerate() {
        let alert = SosAlert {
            alert_id: format!("SIM-BURST-{}", Uuid::new_v4().to_string()[..6].to_uppercase()),
            raw_text: alert_text.to_string(),
            timestamp: Utc::now(),
            source_channel: Some("disaster_stream_simulator_cli".to_string()),
        };

        let payload = serde_json::to_string(&alert)?;
        let msg_id = redis.xadd(stream_key, &payload).await?;
        println!("[{}/{}] Enqueued: {} -> Stream Msg ID: {}", i + 1, alerts.len(), alert.alert_id, msg_id);
    }

    println!("\nSuccessfully injected {} concurrent emergency events into Redis Stream.", alerts.len());
    println!("Check the server logs and web console (http://localhost:3000) to see live processing!");
    Ok(())
}
