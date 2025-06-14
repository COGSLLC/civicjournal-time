use civicjournal_time::{
    config::{Config, SnapshotConfig, TimeHierarchyConfig, LoggingConfig, StorageConfig, SnapshotAutomaticCreationConfig, SnapshotRetentionConfig, RetentionConfig as MainRetentionConfig},
    core::snapshot_manager::SnapshotManager,
    core::time_manager::TimeHierarchyManager,
    storage::memory::MemoryStorage,
    storage::StorageBackend,
    core::leaf::JournalLeaf,
    core::page::PageContent,
    core::merkle::MerkleTree, // For verifying empty Merkle root
    types::time::{LevelRollupConfig, RollupContentType, TimeLevel}, // Added TimeLevel, RollupContentType is correct here
    StorageType, // Assuming StorageType is re-exported at crate::config or crate root
};
use chrono::{Utc, Duration, TimeZone};
use std::sync::Arc;
use serde_json::json;
use uuid::Uuid;
use sha2::{Sha256, Digest};

// Helper function to set up SnapshotManager and its dependencies
fn setup_snapshot_manager(l0_max_leaves: Option<u32>) -> (SnapshotManager, Arc<MemoryStorage>, Arc<TimeHierarchyManager>, Config) {
    let memory_storage_for_config = Arc::new(MemoryStorage::new());
    let cfg = Config {
        logging: LoggingConfig::default(),
        storage: StorageConfig {
            storage_type: StorageType::Memory,
            base_path: "./target/test_data/snapshot_manager".to_string(), // Adjusted for test isolation
            max_open_files: 100, // Default or suitable for tests
        },
        time_hierarchy: TimeHierarchyConfig {
            levels: {
                let mut lvls = Vec::new();
                lvls.push(TimeLevel {
                    name: "L0_test_leaves".to_string(),
                    duration_seconds: 1,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: l0_max_leaves.unwrap_or(100) as usize,
                        max_page_age_seconds: 3600,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                });
                lvls
            },
        },
        snapshot: SnapshotConfig {
            enabled: true,
            dedicated_level: 250,
            retention: SnapshotRetentionConfig::default(),
            automatic_creation: SnapshotAutomaticCreationConfig::default(),
        },
        compression: Default::default(),
        metrics: Default::default(),
        retention: MainRetentionConfig::default(), // Added missing retention field
        force_rollup_on_shutdown: false,
    };
    // Use memory_storage_for_config for the TimeHierarchyManager and SnapshotManager instances if they need the exact same instance
    // Or, if they are okay with a new instance based on the config, that's fine too.
    // For consistency in tests, using the same instance passed to Config is better.
    let time_manager = Arc::new(TimeHierarchyManager::new(Arc::new(cfg.clone()), Arc::clone(&memory_storage_for_config) as Arc<dyn StorageBackend>));
    let snapshot_manager = SnapshotManager::new(Arc::new(cfg.clone()), Arc::clone(&memory_storage_for_config) as Arc<dyn StorageBackend>, Arc::clone(&time_manager));

    (snapshot_manager, memory_storage_for_config, time_manager, cfg)
}

#[tokio::test]
async fn test_create_snapshot_empty_system() {
    let (snapshot_manager, storage, _time_manager, config) = setup_snapshot_manager(None);
    let as_of_timestamp = Utc::now();

    let result = snapshot_manager.create_snapshot(as_of_timestamp, None).await;
    assert!(result.is_ok(), "Snapshot creation failed: {:?}", result.err());
    let snapshot_page_hash = result.unwrap();

    // Verify the snapshot page was stored
    let snapshot_level = config.snapshot.dedicated_level; // dedicated_level is used by SnapshotManager and for verifying stored page level
    let stored_page_option_result = storage.load_page_by_hash(snapshot_page_hash).await;
    assert!(stored_page_option_result.is_ok(), "Failed to query stored page: {:?}", stored_page_option_result.err());
    let stored_page_option = stored_page_option_result.unwrap();
    assert!(stored_page_option.is_some(), "Snapshot page not found in storage for hash: {:?}", snapshot_page_hash);
    let stored_page = stored_page_option.unwrap();

    assert_eq!(stored_page.level, snapshot_level);
    assert_eq!(stored_page.page_hash, snapshot_page_hash);

    if let PageContent::Snapshot(payload) = stored_page.content {
        assert!(Uuid::try_parse(&payload.snapshot_id).is_ok(), "Snapshot ID is not a valid UUID");
        assert_eq!(payload.as_of_timestamp, as_of_timestamp);
        // assert_eq!(payload.target_container_ids, None); // Field removed from SnapshotPagePayload
        assert!(payload.container_states.is_empty(), "Container states should be empty for an empty system");
        assert_eq!(payload.preceding_journal_page_hash, None, "Preceding journal page hash should be None");
        assert_eq!(payload.previous_snapshot_page_hash_on_snapshot_level, None, "Previous snapshot page hash should be None");
        
        // For an empty set of container states, SnapshotManager uses a zero Merkle root.
        let expected_empty_merkle_root: [u8; 32] = [0u8; 32];
        assert_eq!(payload.container_states_merkle_root, expected_empty_merkle_root, "Container states Merkle root mismatch for empty states");
        
        // Check created_at_timestamp is recent (e.g., within the last few seconds)
        assert!(Utc::now().signed_duration_since(payload.created_at_timestamp).num_seconds() < 5, "created_at_timestamp is not recent");
    } else {
        panic!("Stored page content is not a Snapshot");
    }
}

#[tokio::test]
async fn test_create_snapshot_with_active_l0_leaves() {
    let (snapshot_manager, storage, time_manager, _config) = setup_snapshot_manager(None);
    let now = Utc::now();

    let container_id_1 = "container_A".to_string();
    let container_id_2 = "container_B".to_string();

    let leaf1_ts = now - Duration::seconds(10);
    let leaf1 = JournalLeaf {
        leaf_id: 0, 
        timestamp: leaf1_ts,
        prev_hash: None, 
        container_id: container_id_1.clone(),
        delta_payload: json!({ "field1": "value1", "field2": 100 }),
        leaf_hash: [1u8; 32],
    };
    time_manager.add_leaf(&leaf1.clone(), Utc::now()).await.unwrap();

    let leaf2_ts = now - Duration::seconds(5);
    let leaf2 = JournalLeaf {
        leaf_id: 0, 
        timestamp: leaf2_ts,
        prev_hash: None, 
        container_id: container_id_1.clone(),
        delta_payload: json!({ "field1": "value1_updated", "field3": true }),
        leaf_hash: [2u8; 32],
    };
    time_manager.add_leaf(&leaf2.clone(), Utc::now()).await.unwrap();

    let leaf3_ts = now - Duration::seconds(2);
    let leaf3 = JournalLeaf {
        leaf_id: 0, 
        timestamp: leaf3_ts,
        prev_hash: None, 
        container_id: container_id_2.clone(),
        delta_payload: json!({ "status": "active" }),
        leaf_hash: [3u8; 32],
    };
    time_manager.add_leaf(&leaf3.clone(), Utc::now()).await.unwrap();

    let as_of_timestamp = now; 
    let result = snapshot_manager.create_snapshot(as_of_timestamp, None).await;
    assert!(result.is_ok(), "Snapshot creation failed: {:?}", result.err());
    let snapshot_page_hash = result.unwrap();

    let stored_page_result = storage.load_page_by_hash(snapshot_page_hash).await;
    assert!(stored_page_result.is_ok(), "Failed to query stored page: {:?}", stored_page_result.err());
    let stored_page_option = stored_page_result.unwrap();
    assert!(stored_page_option.is_some(), "Snapshot page not found in storage for hash: {:?}", snapshot_page_hash);
    let stored_page = stored_page_option.unwrap();

    if let PageContent::Snapshot(payload) = stored_page.content {
        assert_eq!(payload.as_of_timestamp, as_of_timestamp);
        assert_eq!(payload.container_states.len(), 2, "Should be 2 container states");

        let state_a = payload.container_states.iter().find(|s| s.container_id == container_id_1).expect("Container A state not found");
        let expected_payload_a = json!({ "field1": "value1_updated", "field2": 100, "field3": true });
        let deserialized_state_a: serde_json::Value = serde_json::from_slice(&state_a.state_payload).expect("Failed to deserialize state A");
        assert_eq!(deserialized_state_a, expected_payload_a);
        assert_eq!(state_a.last_leaf_timestamp_applied, leaf2_ts);

        let state_b = payload.container_states.iter().find(|s| s.container_id == container_id_2).expect("Container B state not found");
        let expected_payload_b = json!({ "status": "active" });
        let deserialized_state_b: serde_json::Value = serde_json::from_slice(&state_b.state_payload).expect("Failed to deserialize state B");
        assert_eq!(deserialized_state_b, expected_payload_b);
        assert_eq!(state_b.last_leaf_timestamp_applied, leaf3_ts);

        assert!(payload.preceding_journal_page_hash.is_some(), "Preceding journal page hash should be present");
        assert_eq!(payload.previous_snapshot_page_hash_on_snapshot_level, None);

        let state_hashes: Vec<[u8; 32]> = payload.container_states.iter().map(|state| {
            let mut hasher = Sha256::new();
            hasher.update(state.container_id.as_bytes());
            hasher.update(&state.state_payload);
            hasher.update(&state.last_leaf_hash_applied);
            hasher.update(&state.last_leaf_timestamp_applied.timestamp_micros().to_le_bytes());
            hasher.finalize().into()
        }).collect();

        let expected_merkle_root = if state_hashes.is_empty() {
            [0u8; 32]
        } else {
            MerkleTree::new(state_hashes)
                .expect("Failed to create Merkle tree for container states in test")
                .get_root()
                .expect("Failed to get Merkle root for container states in test")
        };
        assert_eq!(payload.container_states_merkle_root, expected_merkle_root);

    } else {
        panic!("Stored page content is not a Snapshot");
    }
}

#[tokio::test]
async fn test_create_snapshot_with_finalized_l0_page() {
    let (snapshot_manager, storage, time_manager, _config) = setup_snapshot_manager(Some(2));
    let now = Utc::now();

    let container_id = "finalizing_container".to_string();

    let leaf1_ts = now - Duration::seconds(20);
    let leaf1_payload = json!({ "event": "e1"});
    let leaf1 = JournalLeaf {
        leaf_id: 0, 
        timestamp: leaf1_ts,
        prev_hash: None, 
        container_id: container_id.clone(),
        delta_payload: leaf1_payload.clone(),
        leaf_hash: [0u8;32], 
    };
    time_manager.add_leaf(&leaf1, Utc::now()).await.unwrap();

    let leaf2_ts = now - Duration::seconds(15);
    let leaf2_payload = json!({ "event": "e2", "value": 10 });
    let leaf2 = JournalLeaf {
        leaf_id: 0, 
        timestamp: leaf2_ts,
        prev_hash: None, 
        container_id: container_id.clone(),
        delta_payload: leaf2_payload.clone(),
        leaf_hash: [0u8;32],
    };
    time_manager.add_leaf(&leaf2, Utc::now()).await.unwrap();

    let leaf3_ts = now - Duration::seconds(10);
    let leaf3_payload = json!({ "event": "e3", "status": "active" });
    let leaf3 = JournalLeaf {
        leaf_id: 0, 
        timestamp: leaf3_ts,
        prev_hash: None, 
        container_id: container_id.clone(),
        delta_payload: leaf3_payload.clone(),
        leaf_hash: [0u8;32],
    };
    time_manager.add_leaf(&leaf3, Utc::now()).await.unwrap();

    let finalized_l0_pages = storage.list_finalized_pages_summary(0).await.unwrap();
    assert!(!finalized_l0_pages.is_empty(), "Expected at least one finalized L0 page");

    let as_of_timestamp = now; 
    let result = snapshot_manager.create_snapshot(as_of_timestamp, None).await;
    assert!(result.is_ok(), "Snapshot creation failed: {:?}", result.err());
    let snapshot_page_hash = result.unwrap();

    // let _snapshot_level = config.snapshot.dedicated_level;
    let stored_snapshot_page_option_result = storage.load_page_by_hash(snapshot_page_hash).await;
    assert!(stored_snapshot_page_option_result.is_ok(), "Failed to query stored snapshot page: {:?}", stored_snapshot_page_option_result.err());
    let stored_snapshot_page_option = stored_snapshot_page_option_result.unwrap();
    assert!(stored_snapshot_page_option.is_some(), "Snapshot page not found in storage for hash: {:?}", snapshot_page_hash);
    let stored_snapshot_page = stored_snapshot_page_option.unwrap();

    if let PageContent::Snapshot(payload) = stored_snapshot_page.content {
        assert_eq!(payload.as_of_timestamp, as_of_timestamp);
        assert_eq!(payload.container_states.len(), 1, "Should be 1 container state");

        let state = payload.container_states.first().expect("Container state not found");
        assert_eq!(state.container_id, container_id);
        
        let expected_merged_payload = json!({
            "event": "e3",
            "value": 10,
            "status": "active"
        });
        let deserialized_state: serde_json::Value = serde_json::from_slice(&state.state_payload).expect("Failed to deserialize state");
        assert_eq!(deserialized_state, expected_merged_payload);
        assert_eq!(state.last_leaf_timestamp_applied, leaf3_ts);

        assert!(payload.preceding_journal_page_hash.is_some(), "Preceding journal page hash should be present");
        assert_eq!(payload.previous_snapshot_page_hash_on_snapshot_level, None);

        let state_hashes: Vec<[u8; 32]> = payload.container_states.iter().map(|s| {
            let mut hasher = Sha256::new();
            hasher.update(s.container_id.as_bytes());
            hasher.update(&s.state_payload);
            hasher.update(&s.last_leaf_hash_applied);
            hasher.update(&s.last_leaf_timestamp_applied.timestamp_micros().to_le_bytes());
            hasher.finalize().into()
        }).collect();
        let expected_merkle_tree = MerkleTree::new(state_hashes).unwrap();
        assert_eq!(payload.container_states_merkle_root, expected_merkle_tree.get_root().unwrap_or([0u8; 32]));

    } else {
        panic!("Stored page content is not a Snapshot");
    }
}

#[tokio::test]
async fn test_create_snapshot_with_preceding_snapshot() {
    let (snapshot_manager, storage, time_manager, _config) = setup_snapshot_manager(None);
    let initial_time = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();

    let container_id_1 = "multi_snap_container_1".to_string();

    let leaf1_ts = initial_time + Duration::minutes(1);
    let leaf1 = JournalLeaf { 
        leaf_id: 0, timestamp: leaf1_ts, 
        prev_hash: None, 
        container_id: container_id_1.clone(), 
        delta_payload: json!({ "value": 10 }), leaf_hash: [1u8;32]
    };
    time_manager.add_leaf(&leaf1.clone(), Utc::now()).await.unwrap();

    let snapshot1_as_of = initial_time + Duration::minutes(2);
    let result1 = snapshot_manager.create_snapshot(snapshot1_as_of, None).await;
    assert!(result1.is_ok(), "Snapshot 1 creation failed: {:?}", result1.err());
    let snapshot1_page_hash = result1.unwrap();

    // Verify snapshot 1 briefly (ensure it's there)
    let stored_snapshot1_page_result = storage.load_page_by_hash(snapshot1_page_hash).await;
    assert!(stored_snapshot1_page_result.is_ok(), "Failed to load stored snapshot 1 page");
    let stored_snapshot1_page = stored_snapshot1_page_result.unwrap().expect("Snapshot 1 page should exist");
    let snapshot1_payload = match stored_snapshot1_page.content {
        PageContent::Snapshot(p) => p,
        _ => panic!("Snapshot 1 content is not SnapshotPagePayload")
    };
    assert_eq!(snapshot1_payload.as_of_timestamp, snapshot1_as_of);

    // Add more leaves for snapshot 2
    let leaf2_ts = initial_time + Duration::minutes(3);
    let leaf2 = JournalLeaf {
        leaf_id: 0, timestamp: leaf2_ts, 
        prev_hash: None, // Added missing field
        container_id: container_id_1.clone(),
        delta_payload: json!({ "value": 20, "new_field": "added" }), leaf_hash: [0u8;32]
    };
    time_manager.add_leaf(&leaf2.clone(), Utc::now()).await.unwrap();

    // Create second snapshot
    let snapshot2_as_of = initial_time + Duration::minutes(4);
    let result2 = snapshot_manager.create_snapshot(snapshot2_as_of, None).await;
    assert!(result2.is_ok(), "Snapshot 2 creation failed: {:?}", result2.err());
    let snapshot2_page_hash = result2.unwrap();

    // Verify snapshot 2
    let stored_snapshot2_page_result = storage.load_page_by_hash(snapshot2_page_hash).await;
    assert!(stored_snapshot2_page_result.is_ok(), "Failed to load stored snapshot 2 page");
    let stored_snapshot2_page = stored_snapshot2_page_result.unwrap().expect("Snapshot 2 page should exist");

    if let PageContent::Snapshot(payload2) = stored_snapshot2_page.content {
        assert_eq!(payload2.as_of_timestamp, snapshot2_as_of);
        assert_eq!(payload2.container_states.len(), 1, "Snapshot 2 should have 1 container state");
        
        // Check linkage to snapshot 1
        assert_eq!(payload2.previous_snapshot_page_hash_on_snapshot_level, Some(snapshot1_page_hash), "Snapshot 2 not linked to Snapshot 1");

        // Verify container state for snapshot 2 (merges leaf1 and leaf2)
        let state2 = payload2.container_states.first().expect("Container state not found in Snapshot 2");
        assert_eq!(state2.container_id, container_id_1);
        let expected_merged_payload2 = json!({
            "value": 20,
            "new_field": "added"
        });
        let deserialized_state2: serde_json::Value = serde_json::from_slice(&state2.state_payload).expect("Failed to deserialize state for Snapshot 2");
        assert_eq!(deserialized_state2, expected_merged_payload2);
        assert_eq!(state2.last_leaf_timestamp_applied, leaf2_ts);

        // Preceding journal page hash might be None if no L0 page was finalized between snapshot1_as_of and leaf2_ts.
        // This test doesn't force L0 finalization, so we don't assert a specific Some value here.
        // A more complex test could ensure L0 finalization to check this field more rigorously.

        // Verify Merkle root for snapshot 2
        let state_hashes2: Vec<[u8; 32]> = payload2.container_states.iter().map(|s| {
            let mut hasher = Sha256::new();
            hasher.update(s.container_id.as_bytes());
            hasher.update(&s.state_payload);
            hasher.update(&s.last_leaf_hash_applied);
            hasher.update(&s.last_leaf_timestamp_applied.timestamp_micros().to_le_bytes());
            hasher.finalize().into()
        }).collect();
        let expected_merkle_tree2 = MerkleTree::new(state_hashes2).unwrap();
        assert_eq!(payload2.container_states_merkle_root, expected_merkle_tree2.get_root().unwrap_or([0u8; 32]));

    } else {
        panic!("Stored page content for Snapshot 2 is not a Snapshot");
    }
}


