use civicjournal_time::{
    config::{Config, RollupConfig, RollupContentType, SnapshotConfig, TimeHierarchyConfig, LoggingConfig, StorageConfig, SnapshotAutomaticCreationConfig, SnapshotRetentionConfig},
    core::snapshot_manager::SnapshotManager,
    core::time_manager::TimeHierarchyManager,
    storage::memory::MemoryStorage,
    storage::StorageBackend,
    core::leaf::JournalLeaf,
    core::page::{PageContent, JournalPage}, // JournalPage might still be unused, will be caught by compiler if so
    core::snapshot::{SnapshotContainerState, SnapshotPagePayload},
    core::merkle::MerkleTree, // For verifying empty Merkle root
    core::types::errors::CJError,
};
use chrono::{Utc, Duration, TimeZone};
use std::sync::Arc;
// HashMap removed as unused
use serde_json::json;
use uuid::Uuid;
use sha2::{Sha256, Digest};

// Helper function to set up SnapshotManager and its dependencies
fn setup_snapshot_manager(l0_max_leaves: Option<u32>) -> (SnapshotManager, Arc<MemoryStorage>, Arc<TimeHierarchyManager>, Config) {
    let memory_storage_for_config = Arc::new(MemoryStorage::new());
    let cfg = Config {
        logging: LoggingConfig::default(),
        storage: StorageConfig::Memory(Arc::clone(&memory_storage_for_config) as Arc<dyn StorageBackend + Send + Sync>),
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                RollupConfig { // Level 0
                    max_leaves_per_page: l0_max_leaves.unwrap_or(100),
                    max_page_age_seconds: Some(3600),
                    rollup_content_type: RollupContentType::Leaves,
                    finalize_if_only_one_child: false,
                },
            ],
        },
        snapshot: SnapshotConfig {
            enabled: true,
            dedicated_level: 250,
            retention: SnapshotRetentionConfig::default(),
            automatic_creation: SnapshotAutomaticCreationConfig::default(),
        },
        // Removed node_id, version, max_active_pages_per_level as they are not part of Config struct anymore
        // Add other required fields from Config if any, with default values or passed in
        compression: Default::default(),
        metrics: Default::default(),
        // retention: Default::default(), // This is part of SnapshotConfig now
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
    let snapshot_level = config.snapshot.dedicated_level;
    let stored_page_result = storage.load_page_by_hash(snapshot_level, snapshot_page_hash).await;
    assert!(stored_page_result.is_ok(), "Failed to load stored snapshot page: {:?}", stored_page_result.err());
    let stored_page = stored_page_result.unwrap().expect("Snapshot page should exist");

    assert_eq!(stored_page.level, snapshot_level);
    assert_eq!(stored_page.page_hash, snapshot_page_hash);

    if let PageContent::Snapshot(payload) = stored_page.content {
        assert!(Uuid::try_parse(&payload.snapshot_id).is_ok(), "Snapshot ID is not a valid UUID");
        assert_eq!(payload.as_of_timestamp, as_of_timestamp);
        // assert_eq!(payload.target_container_ids, None); // Field removed from SnapshotPagePayload
        assert!(payload.container_states.is_empty(), "Container states should be empty for an empty system");
        assert_eq!(payload.preceding_journal_page_hash, None, "Preceding journal page hash should be None");
        assert_eq!(payload.previous_snapshot_page_hash_on_snapshot_level, None, "Previous snapshot page hash should be None");
        
        // For an empty set of container states, SnapshotManager uses Sha256::digest(&[]) as the Merkle root.
        let expected_empty_merkle_root: [u8; 32] = Sha256::digest(&[]).into();
        assert_eq!(payload.container_states_merkle_root, expected_empty_merkle_root, "Container states Merkle root mismatch for empty states");
        
        // Check created_at_timestamp is recent (e.g., within the last few seconds)
        assert!(Utc::now().signed_duration_since(payload.created_at_timestamp).num_seconds() < 5, "created_at_timestamp is not recent");
    } else {
        panic!("Stored page content is not a Snapshot");
    }
}

#[tokio::test]
async fn test_create_snapshot_with_active_l0_leaves() {
    let (snapshot_manager, storage, time_manager, config) = setup_snapshot_manager(None);
    let now = Utc::now();

    // Add some leaves to the active L0 page
    let container_id_1 = "container_A".to_string();
    let container_id_2 = "container_B".to_string();

    let leaf1_ts = now - Duration::seconds(10);
    let leaf1 = JournalLeaf {
        leaf_id: 1, // Actual ID will be assigned by TimeHierarchyManager or its components
        timestamp: leaf1_ts,
        container_id: container_id_1.clone(),
        delta_payload: json!({ "field1": "value1", "field2": 100 }),
        leaf_hash: [1u8; 32],
    };
    time_manager.add_leaf(&leaf1.clone(), Utc::now()).await.unwrap();

    let leaf2_ts = now - Duration::seconds(5);
    let leaf2 = JournalLeaf {
        leaf_id: 2,
        timestamp: leaf2_ts,
        container_id: container_id_1.clone(),
        delta_payload: json!({ "field1": "value1_updated", "field3": true }),
        leaf_hash: [2u8; 32],
    };
    time_manager.add_leaf(&leaf2.clone(), Utc::now()).await.unwrap();

    let leaf3_ts = now - Duration::seconds(2);
    let leaf3 = JournalLeaf {
        leaf_id: 3,
        timestamp: leaf3_ts,
        container_id: container_id_2.clone(),
        delta_payload: json!({ "status": "active" }),
        leaf_hash: [3u8; 32],
    };
    time_manager.add_leaf(&leaf3.clone(), Utc::now()).await.unwrap();

    // Create snapshot
    let as_of_timestamp = now; // Snapshot includes all added leaves
    let result = snapshot_manager.create_snapshot(as_of_timestamp, None).await;
    assert!(result.is_ok(), "Snapshot creation failed: {:?}", result.err());
    let snapshot_page_hash = result.unwrap();

    // Verify the snapshot page
    let snapshot_level = config.snapshot.dedicated_level;
    let stored_page_result = storage.load_page_by_hash(snapshot_level, snapshot_page_hash).await;
    assert!(stored_page_result.is_ok(), "Failed to load stored snapshot page");
    let stored_page = stored_page_result.unwrap().expect("Snapshot page should exist");

    if let PageContent::Snapshot(payload) = stored_page.content {
        assert_eq!(payload.as_of_timestamp, as_of_timestamp);
        assert_eq!(payload.container_states.len(), 2, "Should be 2 container states");

        // Verify container A's state (merging leaf1 and leaf2)
        let state_a = payload.container_states.iter().find(|s| s.container_id == container_id_1).expect("Container A state not found");
        let expected_payload_a = json!({ "field1": "value1_updated", "field2": 100, "field3": true });
        let deserialized_state_a: serde_json::Value = serde_json::from_slice(&state_a.state_payload).expect("Failed to deserialize state A");
        assert_eq!(deserialized_state_a, expected_payload_a);
        assert_eq!(state_a.last_leaf_timestamp_applied, leaf2_ts);
        // Note: leaf_hash verification depends on how SnapshotManager calculates/stores it. Assuming it's leaf2.leaf_hash for now.
        // This might need adjustment if SnapshotManager uses its own internal hashing for reconstructed states.

        // Verify container B's state (from leaf3)
        let state_b = payload.container_states.iter().find(|s| s.container_id == container_id_2).expect("Container B state not found");
        let expected_payload_b = json!({ "status": "active" });
        let deserialized_state_b: serde_json::Value = serde_json::from_slice(&state_b.state_payload).expect("Failed to deserialize state B");
        assert_eq!(deserialized_state_b, expected_payload_b);
        assert_eq!(state_b.last_leaf_timestamp_applied, leaf3_ts);

        assert_eq!(payload.preceding_journal_page_hash, None, "Preceding journal page hash should be None as no L0 page finalized");
        assert_eq!(payload.previous_snapshot_page_hash_on_snapshot_level, None);

        // Verify Merkle root of container states
        let mut sorted_states_for_hashing = payload.container_states.clone();
        // Sort by container_id for consistent Merkle root calculation, mirroring SnapshotManager's logic.
        sorted_states_for_hashing.sort_by(|a, b| a.container_id.cmp(&b.container_id));

        let state_hashes: Vec<[u8; 32]> = sorted_states_for_hashing.iter().map(|state| {
            let serialized_state = serde_json::to_vec(state).expect("Failed to serialize SnapshotContainerState for hashing in test");
            Sha256::digest(&serialized_state).into()
        }).collect();

        let expected_merkle_root = if state_hashes.is_empty() {
            Sha256::digest(&[]).into() // Should not happen in this test case as we expect states
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
    let (snapshot_manager, storage, time_manager, config) = setup_snapshot_manager(Some(2)); // L0 finalizes after 2 leaves
    let now = Utc::now();

    let container_id = "finalizing_container".to_string();

    // Add leaf1 (will be in finalized page)
    let leaf1_ts = now - Duration::seconds(20);
    let leaf1_payload = json!({ "event": "e1"});
    let leaf1 = JournalLeaf {
        leaf_id: 0, // Placeholder, TimeHierarchyManager assigns actual ID and hash
        timestamp: leaf1_ts,
        container_id: container_id.clone(),
        delta_payload: leaf1_payload.clone(),
        leaf_hash: [0u8;32], // Placeholder
    };
    time_manager.add_leaf(&leaf1, Utc::now()).await.unwrap();

    // Add leaf2 (will be in finalized page, triggering finalization)
    let leaf2_ts = now - Duration::seconds(15);
    let leaf2_payload = json!({ "event": "e2", "value": 10 });
    let leaf2 = JournalLeaf {
        leaf_id: 0,
        timestamp: leaf2_ts,
        container_id: container_id.clone(),
        delta_payload: leaf2_payload.clone(),
        leaf_hash: [0u8;32],
    };
    time_manager.add_leaf(&leaf2, Utc::now()).await.unwrap();

    // Add leaf3 (will be in a new active L0 page)
    let leaf3_ts = now - Duration::seconds(10);
    let leaf3_payload = json!({ "event": "e3", "status": "active" });
    let leaf3 = JournalLeaf {
        leaf_id: 0,
        timestamp: leaf3_ts,
        container_id: container_id.clone(),
        delta_payload: leaf3_payload.clone(),
        leaf_hash: [0u8;32],
    };
    time_manager.add_leaf(&leaf3, Utc::now()).await.unwrap();

    // Ensure L0 page is finalized by checking storage (indirectly)
    // We expect one finalized L0 page.
    let finalized_l0_pages = storage.list_finalized_pages_summary(0).await.unwrap();
    assert_eq!(finalized_l0_pages.len(), 1, "Expected one finalized L0 page");
    let finalized_l0_page_summary = finalized_l0_pages.first().unwrap();

    // Create snapshot *after* L0 finalization and including leaf3 from active L0
    let as_of_timestamp = now; 
    let result = snapshot_manager.create_snapshot(as_of_timestamp, None).await;
    assert!(result.is_ok(), "Snapshot creation failed: {:?}", result.err());
    let snapshot_page_hash = result.unwrap();

    // Verify the snapshot page
    let snapshot_level = config.snapshot.dedicated_level;
    let stored_snapshot_page_result = storage.load_page_by_hash(snapshot_page_hash).await;
    assert!(stored_snapshot_page_result.is_ok(), "Failed to load stored snapshot page");
    let stored_snapshot_page = stored_snapshot_page_result.unwrap().expect("Snapshot page should exist");

    if let PageContent::Snapshot(payload) = stored_snapshot_page.content {
        assert_eq!(payload.as_of_timestamp, as_of_timestamp);
        assert_eq!(payload.container_states.len(), 1, "Should be 1 container state");

        // Verify container state (merging leaf1, leaf2 from finalized, and leaf3 from active)
        let state = payload.container_states.first().expect("Container state not found");
        assert_eq!(state.container_id, container_id);
        
        // Expected state merges all three leaves
        let expected_merged_payload = json!({
            "event": "e3",
            "value": 10,
            "status": "active"
        });
        let deserialized_state: serde_json::Value = serde_json::from_slice(&state.state_payload).expect("Failed to deserialize state");
        assert_eq!(deserialized_state, expected_merged_payload);
        assert_eq!(state.last_leaf_timestamp_applied, leaf3_ts);

        assert_eq!(payload.preceding_journal_page_hash, Some(finalized_l0_page_summary.page_hash), "Preceding journal page hash mismatch");
        assert_eq!(payload.previous_snapshot_page_hash_on_snapshot_level, None);

        // Verify Merkle root
        let mut sorted_states_for_hashing = payload.container_states.clone();
        sorted_states_for_hashing.sort_by(|a, b| a.container_id.cmp(&b.container_id));
        let state_hashes: Vec<[u8; 32]> = sorted_states_for_hashing.iter().map(|s| {
            let serialized_state = serde_json::to_vec(s).expect("Failed to serialize state for Merkle root test");
            Sha256::digest(&serialized_state).into()
        }).collect();
        let expected_merkle_tree = MerkleTree::new(state_hashes).unwrap();
        assert_eq!(payload.container_states_merkle_root, expected_merkle_tree.get_root().unwrap_or([0u8; 32]));

    } else {
        panic!("Stored page content is not a Snapshot");
    }
}

#[tokio::test]
async fn test_create_snapshot_with_preceding_snapshot() {
    let (snapshot_manager, storage, time_manager, config) = setup_snapshot_manager(None);
    let initial_time = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();

    let container_id_1 = "multi_snap_container_1".to_string();

    // Add initial leaf for snapshot 1
    let leaf1_ts = initial_time + Duration::minutes(1);
    let leaf1 = JournalLeaf {
        leaf_id: 0, timestamp: leaf1_ts, container_id: container_id_1.clone(), 
        delta_payload: json!({ "value": 10 }), leaf_hash: [1u8;32]
    };
    time_manager.add_leaf(&leaf1.clone(), Utc::now()).await.unwrap();

    // Create first snapshot
    let snapshot1_as_of = initial_time + Duration::minutes(2);
    let result1 = snapshot_manager.create_snapshot(snapshot1_as_of, None).await;
    assert!(result1.is_ok(), "Snapshot 1 creation failed: {:?}", result1.err());
    let snapshot1_page_hash = result1.unwrap();

    // Verify snapshot 1 briefly (ensure it's there)
    let snapshot_level = config.snapshot.dedicated_level;
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
        leaf_id: 0, timestamp: leaf2_ts, container_id: container_id_1.clone(),
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
        let mut sorted_states_for_hashing2 = payload2.container_states.clone();
        sorted_states_for_hashing2.sort_by(|a, b| a.container_id.cmp(&b.container_id));
        let state_hashes2: Vec<[u8; 32]> = sorted_states_for_hashing2.iter().map(|s| {
            let serialized_state = serde_json::to_vec(s).expect("Failed to serialize state for Merkle root test (Snapshot 2)");
            Sha256::digest(&serialized_state).into()
        }).collect();
        let expected_merkle_tree2 = MerkleTree::new(state_hashes2).unwrap();
        assert_eq!(payload2.container_states_merkle_root, expected_merkle_tree2.get_root().unwrap_or([0u8; 32]));

    } else {
        panic!("Stored page content for Snapshot 2 is not a Snapshot");
    }
}


