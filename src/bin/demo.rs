//! A demo binary that shows the CivicJournal time hierarchy in action and saves output to disk

use std::collections::BTreeMap;
use civicjournal_time::schema::{Delta, Operation, CivicObject, CivicObjectV1};
use civicjournal_time::time_hierarchy::TimeHierarchy;
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json;
use std::{
    fs,
    path::Path,
};

/// Generate a simple hash for demo purposes
fn simple_hash(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Helper to save data as pretty-printed JSON
fn save_as_json<T: serde::Serialize>(
    path: impl AsRef<Path>,
    data: &T,
) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(data)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Helper to create a test delta
fn create_delta(timestamp: DateTime<Utc>, hour: u32, previous_hash: [u8; 32]) -> Delta {
    // Create a simple operation for demo purposes
    let mut data = BTreeMap::new();
    data.insert("hour".to_string(), hour.to_string());
    data.insert("description".to_string(), format!("Demo object created at hour {}", hour));
    
    let operation = Operation::Create {
        object: CivicObject::V1(CivicObjectV1 {
            id: format!("object_{}", hour),
            created_at: timestamp,
            data,
        })
    };
    
    // Create the delta with the operation and previous hash
    Delta::new(operation, timestamp, previous_hash)
}

/// Helper to create an initial hash for the first delta
fn initial_hash() -> [u8; 32] {
    let data = b"initial_hash";
    simple_hash(data)
}

fn main() -> anyhow::Result<()> {
    // Create output directory
    let output_dir = "./civicjournal_demo";
    if Path::new(output_dir).exists() {
        fs::remove_dir_all(output_dir)?;
    }
    fs::create_dir_all(output_dir)?;

    // Initialize time hierarchy
    let mut hierarchy = TimeHierarchy::new();
    let start_time = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
    
    // Create output directories
    let deltas_dir = format!("{}/deltas", output_dir);
    let snapshots_dir = format!("{}/snapshots", output_dir);
    fs::create_dir_all(&deltas_dir)?;
    fs::create_dir_all(&snapshots_dir)?;
    
    // Create a delta every hour for 3 days (for demo purposes)
    let total_hours = 24 * 3;
    let save_interval = 6; // Save hierarchy every 6 hours
    let mut previous_hash = initial_hash(); // Start with an initial hash
    
    for hour in 0..=total_hours {
        let current_time = start_time + Duration::hours(hour as i64);
        
        // Create and add delta
        let delta = create_delta(current_time, hour, previous_hash);
        
        // Save raw delta to disk as JSON
        let delta_file = format!("{}/delta_{:04}.json", deltas_dir, hour);
        save_as_json(&delta_file, &delta)?;
        
        // Add to hierarchy
        hierarchy.add_delta(delta.clone())?;
        
        // Update previous_hash for next delta
        previous_hash = delta.previous_hash; // Or calculate a new hash based on the operation
        
        // Save hierarchy state at intervals
        if hour % save_interval == 0 || hour == total_hours {
            let snapshot_file = format!(
                "{}/snapshot_hour_{:04}.json", 
                snapshots_dir,
                hour
            );
            
            // Save the current state of the hierarchy
            let snapshot = serde_json::json!({
                "timestamp": current_time.to_rfc3339(),
                "hour": hour,
                "chunks_count": hierarchy.calculate_stats().chunk_count,
                "total_deltas": hour + 1, // +1 because we start at hour 0
            });
            
            save_as_json(&snapshot_file, &snapshot)?;
            
            // In a real implementation, we would serialize the entire hierarchy here
            // let hierarchy_json = serde_json::to_string_pretty(&hierarchy)?;
            // std::fs::write(snapshot_file, hierarchy_json)?;
        }
    }
    
    // Create a summary of the demo
    let summary = format!(
        "CivicJournal Demo Summary\n\
        =========================\n\
        Total hours: {}\n\
        Total deltas: {}\n\
        Output directory: {}\n\
        Deltas saved to: {}\n\
        Hierarchy snapshots saved to: {}\n",
        total_hours,
        total_hours + 1, // +1 because we start at hour 0
        output_dir,
        deltas_dir,
        snapshots_dir,
    );
    
    // Save summary to file
    let summary_file = format!("{}/summary.txt", output_dir);
    std::fs::write(summary_file, &summary)?;
    
    // Print summary to console
    println!("\n{}", summary);
    
    println!("Demo complete! Check the '{}' directory for output files.", output_dir);
    
    Ok(())
}
