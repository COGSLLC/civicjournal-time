// New imports for integration test context
use civicjournal_time::core::time_manager::TimeHierarchyManager;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::page::{JournalPage, PageContent, PageIdGenerator};
use civicjournal_time::LevelRollupConfig;
use civicjournal_time::storage::memory::MemoryStorage;
use civicjournal_time::config::{
    Config, RetentionConfig, StorageConfig as TestStorageConfig, CompressionConfig, 
    LoggingConfig, MetricsConfig, TimeHierarchyConfig, TimeLevel
};
use civicjournal_time::types::time::RollupRetentionPolicy;
use civicjournal_time::StorageType as TestStorageType;
use civicjournal_time::CompressionAlgorithm;
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::types::time::RollupContentType;
use std::sync::Arc; // Added Arc for shared ownership
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::json;
use tokio::time::{timeout, Duration as TokioDuration};

// Helper to create a config for testing, allowing retention customization
fn create_test_config(
    retention_enabled: bool, 
    retention_period_seconds: u64,
    l0_policy: Option<RollupRetentionPolicy>,
    l1_policy: Option<RollupRetentionPolicy>
) -> Config {
    Config {
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                TimeLevel {
                    name: "seconds".to_string(),
                    duration_seconds: 1,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 1,
                        max_page_age_seconds: 1000,
                        content_type: RollupContentType::ChildHashes, // L0's content represented as hashes in L1
                    },
                    retention_policy: l0_policy
                },
                TimeLevel {
                    name: "minutes".to_string(),
                    duration_seconds: 60,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 1,
                        max_page_age_seconds: 1000,
                        content_type: RollupContentType::ChildHashes, // L1's content represented as hashes in L2 (if L2 exists)
                    },
                    retention_policy: l1_policy
                },
            ],
        },
        force_rollup_on_shutdown: false,
        storage: Default::default(), // Assuming TestStorageConfig will be used or it's okay
        retention: RetentionConfig {
            enabled: retention_enabled,
            period_seconds: retention_period_seconds,
            cleanup_interval_seconds: 300, // Default, not typically used in these specific tests
        },
        compression: Default::default(),
        logging: Default::default(),
        metrics: Default::default(),
    }
}

// Note: This directly creates a JournalPage. For manager tests, you'd typically add leaves.
// However, for retention, we need to control page end_times precisely.
fn create_retention_test_page(level: u8, page_id_offset: u64, time_window_start: DateTime<Utc>, config: &Config) -> JournalPage {
    let gen = PageIdGenerator::new();
    for _ in 0..page_id_offset {
        gen.next();
    }
    JournalPage::new_with_id(
        gen.next(),
        level,
        None,
        time_window_start,
        config,
    )
}

// Helper to create a TimeHierarchyManager with MemoryStorage
fn create_test_manager() -> (TimeHierarchyManager, Arc<MemoryStorage>) {
    // Default to retention disabled for general tests not focused on retention
    let config = Arc::new(create_test_config(false, 0, None, None)); 
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config, storage.clone());
    (manager, storage)
}

#[tokio::test]
async fn test_add_leaf_to_new_page_and_check_active() {
    
    let (manager, storage) = create_test_manager(); // Config from create_test_manager implies mlpp=1 for L0 & L1
    let now = Utc::now();

    // Leaf 1: L0P0 (ID 0) finalizes, L1P1 (ID 1) finalizes. No L2 configured by create_test_manager.
    let leaf1_ts = now;
    let leaf1 = JournalLeaf::new(leaf1_ts, None, "container1".to_string(), json!({"id":1})).unwrap();
    manager.add_leaf(&leaf1, leaf1.timestamp).await.unwrap();

    let stored_l0_p0_leaf1 = storage.load_page(0, 0).await.unwrap().expect("L0P0 (ID 0) should be in storage after 1st leaf");
    assert_eq!(stored_l0_p0_leaf1.page_id, 0);
    assert_eq!(stored_l0_p0_leaf1.content_len(), 1);
    match &stored_l0_p0_leaf1.content {
        PageContent::Leaves(leaves) => {
            assert_eq!(leaves.len(), 1, "L0P0 (ID 0) content length mismatch");
            assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0P0 (ID 0) leaf hash mismatch");
        }
        _ => panic!("Expected Leaves for stored_l0_p0_leaf1, found {:?}", stored_l0_p0_leaf1.content),
    }
    assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active after L0P0 finalization");

    let stored_l1_p1_leaf1 = storage.load_page(1, 1).await.unwrap().expect("L1P1 (ID 1) should be stored after L0P0 rollup and L1 finalization");
    assert_eq!(stored_l1_p1_leaf1.page_id, 1); // L1 page ID is 1
    assert_eq!(stored_l1_p1_leaf1.content_len(), 1, "Stored L1P1 should have 1 thrall hash from L0P0");
    match &stored_l1_p1_leaf1.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "L1P1 (ID 1) content length mismatch");
            assert_eq!(hashes[0], stored_l0_p0_leaf1.page_hash, "L1P1 (ID 1) thrall hash mismatch");
        }
        _ => panic!("Expected ThrallHashes for stored_l1_p1_leaf1, found {:?}", stored_l1_p1_leaf1.content),
    }
    assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "No L1 page should be active after L1P1 finalization (L1 is highest configured level)");
    assert!(manager.active_pages.lock().await.get(&2u8).is_none(), "No L2 page should exist as L2 is not configured by create_test_manager");

    // Leaf 2: Creates L0P2 (ID 2) because L0P0 (ID 0) already finalized. L0P2 finalizes. L1P3 (ID 3) created from L0P2 rollup, finalizes.
    let leaf2_ts = now + Duration::milliseconds(100); // now is from first leaf's setup
    let leaf2 = JournalLeaf::new(leaf2_ts, None, "container1".to_string(), json!({"id":2})).unwrap();
    manager.add_leaf(&leaf2, leaf2.timestamp).await.unwrap();

    // Check L0P0 (ID 0) - should still have only leaf1. stored_l0_p0_leaf1 is from the first leaf's assertions.
    assert_eq!(stored_l0_p0_leaf1.page_id, 0);
    assert_eq!(stored_l0_p0_leaf1.content_len(), 1, "L0P0 (ID 0) should still only have 1 leaf (leaf1)");
    match &stored_l0_p0_leaf1.content { // Re-assertion on L0P0 (ID 0)
        PageContent::Leaves(leaves) => {
            assert_eq!(leaves.len(), 1, "L0P0 (ID 0) re-assertion content length mismatch");
            assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0P0 (ID 0) re-assertion leaf hash mismatch");
        }
        _ => panic!("Expected Leaves for stored_l0_p0_leaf1 (re-assertion), found {:?}", stored_l0_p0_leaf1.content),
    }
    assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active after L0P0 finalization");

    let stored_l0_p2 = storage.load_page(0, 2).await.unwrap().expect("L0P2 (ID 2) should be in storage after leaf2");
    assert_eq!(stored_l0_p2.page_id, 2);
    assert_eq!(stored_l0_p2.content_len(), 1, "L0P2 (ID 2) should have 1 leaf (leaf2)");
    match &stored_l0_p2.content {
        PageContent::Leaves(leaves) => {
            assert_eq!(leaves.len(), 1, "L0P2 (ID 2) content length mismatch");
            assert_eq!(leaves[0].leaf_hash, leaf2.leaf_hash, "L0P2 (ID 2) leaf hash mismatch");
        }
        _ => panic!("Expected Leaves for stored_l0_p2, found {:?}", stored_l0_p2.content),
    }
    assert_eq!(stored_l0_p2.prev_page_hash, Some(stored_l0_p0_leaf1.page_hash), "L0P2 prev_page_hash should be L0P0's hash");
    assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active");

    // Check L1P1 (ID 1) - from L0P0 rollup. stored_l1_p1_leaf1 is from the first leaf's assertions (Page ID 1).
    assert_eq!(stored_l1_p1_leaf1.page_id, 1);
    assert_eq!(stored_l1_p1_leaf1.content_len(), 1, "Stored L1P1 (ID 1) should have 1 thrall hash from L0P0");
    match &stored_l1_p1_leaf1.content { // Re-assertion on L1P1 (ID 1)
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "L1P1 (ID 1) re-assertion content length mismatch");
            assert_eq!(hashes[0], stored_l0_p0_leaf1.page_hash, "L1P1 (ID 1) re-assertion thrall hash mismatch");
        }
        _ => panic!("Expected ThrallHashes for stored_l1_p1_leaf1 (re-assertion), found {:?}", stored_l1_p1_leaf1.content),
    }

    // Check L1P3 (ID 3) - from L0P2 rollup, should be stored
    let stored_l1_p3 = storage.load_page(1, 3).await.unwrap().expect("L1P3 (ID 3) from L0P2 rollup should be stored");
    assert_eq!(stored_l1_p3.page_id, 3);
    assert_eq!(stored_l1_p3.content_len(), 1, "Stored L1P3 should have 1 thrall hash from L0P2");
    match &stored_l1_p3.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "L1P3 (ID 3) content length mismatch");
            assert_eq!(hashes[0], stored_l0_p2.page_hash, "L1P3 (ID 3) thrall hash mismatch");
        }
        _ => panic!("Expected ThrallHashes for stored_l1_p3, found {:?}", stored_l1_p3.content),
    }
    assert_eq!(stored_l1_p3.prev_page_hash, Some(stored_l1_p1_leaf1.page_hash), "L1P3 prev_page_hash should be L1P1's hash");
    assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "No L1 page should be active");
    assert!(manager.active_pages.lock().await.get(&2u8).is_none(), "No L2 page should exist");

    // Leaf 3: Creates L0P4 (ID 4). L0P4 finalizes. L1P5 (ID 5) created from L0P4 rollup, finalizes.
    // Original L0P0 (ID 0) and L0P2 (ID 2) are finalized and stored.
    // Original L1P1 (ID 1) and L1P3 (ID 3) are finalized and stored.
    let leaf3_ts = now + Duration::milliseconds(200);
    let leaf3 = JournalLeaf::new(leaf3_ts, None, "container3".to_string(), json!({"id":3})).unwrap();
    manager.add_leaf(&leaf3, leaf3.timestamp).await.unwrap();

    // Check L0P4 (ID 4) - created for leaf3
    let stored_l0_p4 = storage.load_page(0, 4).await.unwrap().expect("L0P4 (ID 4) should be in storage after leaf3");
    assert_eq!(stored_l0_p4.page_id, 4);
    assert_eq!(stored_l0_p4.content_len(), 1, "L0P4 (ID 4) should have 1 leaf (leaf3)");
    match &stored_l0_p4.content {
        PageContent::Leaves(leaves) => {
            assert_eq!(leaves.len(), 1, "L0P4 (ID 4) content length mismatch");
            assert_eq!(leaves[0].leaf_hash, leaf3.leaf_hash, "L0P4 (ID 4) leaf hash mismatch");
        }
        _ => panic!("Expected Leaves for stored_l0_p4, found {:?}", stored_l0_p4.content),
    }
    assert_eq!(stored_l0_p4.prev_page_hash, Some(stored_l0_p2.page_hash), "L0P4 prev_page_hash should be L0P2's hash");
    assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active");
    
    // Check L1P5 (ID 5) - from L0P4 rollup, should be stored
    let stored_l1_p5 = storage.load_page(1, 5).await.unwrap().expect("L1P5 (ID 5) from L0P4 rollup should be stored");
    assert_eq!(stored_l1_p5.page_id, 5);
    assert_eq!(stored_l1_p5.content_len(), 1, "Stored L1P5 should have 1 thrall hash from L0P4");
    match &stored_l1_p5.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "L1P5 (ID 5) content length mismatch");
            assert_eq!(hashes[0], stored_l0_p4.page_hash, "L1P5 (ID 5) thrall hash mismatch");
        }
        _ => panic!("Expected ThrallHashes for stored_l1_p5, found {:?}", stored_l1_p5.content),
    }
    assert_eq!(stored_l1_p5.prev_page_hash, Some(stored_l1_p3.page_hash), "L1P5 prev_page_hash should be L1P3's hash");
    assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "No L1 page should be active");
}

#[tokio::test]
async fn test_single_rollup_max_items() {
    
    let (manager, storage) = create_test_manager(); // Config from create_test_manager: L0 (1s, mlpp=1), L1 (60s, mlpp=1), no L2.
    let timestamp1 = Utc::now();

    // Leaf 1: L0P0 (ID 0) finalizes, L1P0 (ID 1) finalizes. No L2 configured by create_test_manager.
    let leaf1 = JournalLeaf::new(timestamp1, None, "rollup_container1".to_string(), json!({ "data": "leaf1" })).unwrap();
    manager.add_leaf(&leaf1, leaf1.timestamp).await.unwrap();

    let stored_l0_p0_leaf1 = storage.load_page(0, 0).await.unwrap().expect("L0P0 (ID 0) should be in storage after 1st leaf");
    assert_eq!(stored_l0_p0_leaf1.page_id, 0);
    assert_eq!(stored_l0_p0_leaf1.content_len(), 1);
    match &stored_l0_p0_leaf1.content {
        PageContent::Leaves(leaves) => {
            assert_eq!(leaves.len(), 1, "L0P0 (ID 0) content length mismatch");
            assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0P0 (ID 0) leaf hash mismatch");
        }
        _ => panic!("Expected Leaves for stored_l0_p0_leaf1, found {:?}", stored_l0_p0_leaf1.content),
    }
    assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active after L0P0 finalization");

    let stored_l1_p0_leaf1 = storage.load_page(1, 1).await.unwrap().expect("L1P0 (ID 1) should be stored after L0P0 rollup and L1 finalization");
    assert_eq!(stored_l1_p0_leaf1.page_id, 1);
    assert_eq!(stored_l1_p0_leaf1.content_len(), 1, "Stored L1P0 should have 1 thrall hash from L0P0");
    match &stored_l1_p0_leaf1.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "L1P0 (ID 1) content length mismatch in test_single_rollup_max_items");
            assert_eq!(hashes[0], stored_l0_p0_leaf1.page_hash, "L1P0 (ID 1) thrall hash mismatch in test_single_rollup_max_items");
        }
        _ => panic!("Expected ThrallHashes for stored_l1_p0_leaf1 in test_single_rollup_max_items, found {:?}", stored_l1_p0_leaf1.content),
    }
    assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "No L1 page should be active after L1P0 finalization (L1 is highest configured level)");
    assert!(manager.active_pages.lock().await.get(&2u8).is_none(), "No L2 page should exist as L2 is not configured by create_test_manager");

    // Leaf 2: Creates L0P2 (ID 2) because L0P0 (ID 0) already finalized. L0P2 finalizes. L1P3 (ID 3) created from L0P2 rollup, finalizes.
    let timestamp2 = timestamp1 + Duration::milliseconds(100); // timestamp1 is from first leaf's setup
    let leaf2 = JournalLeaf::new(timestamp2, None, "rollup_container2".to_string(), json!({ "data": "leaf2" })).unwrap();
    manager.add_leaf(&leaf2, leaf2.timestamp).await.unwrap();

    // Check L0P0 (ID 0) - should still have only leaf1. stored_l0_p0_leaf1 is from the first leaf's assertions.
    assert_eq!(stored_l0_p0_leaf1.page_id, 0);
    assert_eq!(stored_l0_p0_leaf1.content_len(), 1, "L0P0 (ID 0) should still only have 1 leaf (leaf1)");
    match &stored_l0_p0_leaf1.content { // Re-assertion for L0P0 after leaf2 added
        PageContent::Leaves(leaves) => {
            assert_eq!(leaves.len(), 1, "L0P0 (ID 0) content length mismatch (re-assertion)");
            assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0P0 (ID 0) leaf hash mismatch (re-assertion)");
        }
        _ => panic!("Expected Leaves for stored_l0_p0_leaf1 (re-assertion), found {:?}", stored_l0_p0_leaf1.content),
    } // leaf1 is from first leaf's setup

    // Check L0P2 (ID 2) - created for leaf2
    let stored_l0_p2 = storage.load_page(0, 2).await.unwrap().expect("L0P2 (ID 2) should be in storage after leaf2");
    assert_eq!(stored_l0_p2.page_id, 2);
    assert_eq!(stored_l0_p2.content_len(), 1, "L0P2 (ID 2) should have 1 leaf (leaf2)");
    match &stored_l0_p2.content {
        PageContent::Leaves(leaves) => {
            assert_eq!(leaves.len(), 1, "L0P2 (ID 2) content length mismatch");
            assert_eq!(leaves[0].leaf_hash, leaf2.leaf_hash, "L0P2 (ID 2) leaf hash mismatch");
        }
        _ => panic!("Expected Leaves for stored_l0_p2, found {:?}", stored_l0_p2.content),
    }
    assert_eq!(stored_l0_p2.prev_page_hash, Some(stored_l0_p0_leaf1.page_hash), "L0P2 prev_page_hash should be L0P0's hash");
    assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active");

    // Check L1P1 (ID 1) - from L0P0 rollup. stored_l1_p0_leaf1 is from the first leaf's assertions (Page ID 1).
    assert_eq!(stored_l1_p0_leaf1.page_id, 1);
    assert_eq!(stored_l1_p0_leaf1.content_len(), 1, "Stored L1P1 (ID 1) should have 1 thrall hash from L0P0");
    match &stored_l1_p0_leaf1.content { // This is page ID 1 (stored_l1_p0_leaf1)
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "Stored L1P1 (ID 1) content length mismatch (re-assertion)");
            assert_eq!(hashes[0], stored_l0_p0_leaf1.page_hash, "Stored L1P1 (ID 1) thrall hash mismatch (re-assertion)");
        }
        _ => panic!("Expected ThrallHashes for stored_l1_p0_leaf1 (re-assertion), found {:?}", stored_l1_p0_leaf1.content),
    }

    // Check L1P3 (ID 3) - from L0P2 rollup, should be stored
    let stored_l1_p3 = storage.load_page(1, 3).await.unwrap().expect("L1P3 (ID 3) from L0P2 rollup should be stored");
    assert_eq!(stored_l1_p3.page_id, 3);
    assert_eq!(stored_l1_p3.content_len(), 1, "Stored L1P3 should have 1 thrall hash from L0P2");
    match &stored_l1_p3.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "Stored L1P3 (ID 3) content length mismatch");
            assert_eq!(hashes[0], stored_l0_p2.page_hash, "Stored L1P3 (ID 3) thrall hash mismatch");
        }
        _ => panic!("Expected ThrallHashes for stored_l1_p3, found {:?}", stored_l1_p3.content),
    }
    assert_eq!(stored_l1_p3.prev_page_hash, Some(stored_l1_p0_leaf1.page_hash), "L1P3 prev_page_hash should be L1P1's hash"); // Corrected variable
    assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "No L1 page should be active");
    assert!(manager.active_pages.lock().await.get(&2u8).is_none(), "No L2 page should exist");

    // Check last finalized for L0
    assert_eq!(manager.last_finalized_page_ids.lock().await.get(&0u8).copied().unwrap(), stored_l0_p2.page_id, "L0 last_finalized_page_id mismatch");
    assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&0u8).copied().unwrap(), stored_l0_p2.page_hash, "L0 last_finalized_page_hash mismatch");
    // Leaf 3: Creates L0P4 (ID 4) because L0P2 (ID 2) already finalized. L0P4 finalizes. L1P5 (ID 5) created from L0P4 rollup, finalizes.
    let timestamp3 = timestamp2 + Duration::milliseconds(100);
    let leaf3 = JournalLeaf::new(timestamp3, None, "rollup_container3".to_string(), json!({ "data": "leaf3" })).unwrap();
    manager.add_leaf(&leaf3, leaf3.timestamp).await.unwrap();

    // Check L0P4 (ID 4) - created for leaf3
    let stored_l0_p4 = storage.load_page(0, 4).await.unwrap().expect("L0P4 (ID 4) should be in storage after leaf3");
    assert_eq!(stored_l0_p4.page_id, 4);
    assert_eq!(stored_l0_p4.content_len(), 1, "L0P4 (ID 4) should have 1 leaf (leaf3)");
    match &stored_l0_p4.content {
        PageContent::Leaves(leaves) => {
            assert_eq!(leaves.len(), 1, "L0P4 (ID 4) content length mismatch");
            assert_eq!(leaves[0].leaf_hash, leaf3.leaf_hash, "L0P4 (ID 4) leaf hash mismatch");
        }
        _ => panic!("Expected Leaves for stored_l0_p4, found {:?}", stored_l0_p4.content),
    }
    assert_eq!(stored_l0_p4.prev_page_hash, Some(stored_l0_p2.page_hash), "L0P4 prev_page_hash should be L0P2's hash"); // Links to L0P2
    assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active");

    // Check L1P5 (ID 5) - from L0P4 rollup, should be stored
    let stored_l1_p5 = storage.load_page(1, 5).await.unwrap().expect("L1P5 (ID 5) from L0P4 rollup should be stored");
    assert_eq!(stored_l1_p5.page_id, 5);
    assert_eq!(stored_l1_p5.content_len(), 1, "Stored L1P5 should have 1 thrall hash from L0P4");
    match &stored_l1_p5.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "Stored L1P5 (ID 5) content length mismatch");
            assert_eq!(hashes[0], stored_l0_p4.page_hash, "Stored L1P5 (ID 5) thrall hash mismatch");
        }
        _ => panic!("Expected ThrallHashes for stored_l1_p5, found {:?}", stored_l1_p5.content),
    }
    assert_eq!(stored_l1_p5.prev_page_hash, Some(stored_l1_p3.page_hash), "L1P5 prev_page_hash should be L1P3's hash"); // Links to L1P3
    assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "No L1 page should be active");
    
    // Check last finalized for L0 and L1 after leaf 3
    assert_eq!(manager.last_finalized_page_ids.lock().await.get(&0u8).copied().unwrap(), stored_l0_p4.page_id, "L0 last_finalized_page_id mismatch after leaf3");
    assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&0u8).copied().unwrap(), stored_l0_p4.page_hash, "L0 last_finalized_page_hash mismatch after leaf3");
    assert_eq!(manager.last_finalized_page_ids.lock().await.get(&1u8).copied().unwrap(), stored_l1_p5.page_id, "L1 last_finalized_page_id mismatch after leaf3");
    assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&1u8).copied().unwrap(), stored_l1_p5.page_hash, "L1 last_finalized_page_hash mismatch after leaf3");
}

fn create_cascading_test_config_and_manager() -> (TimeHierarchyManager, Arc<MemoryStorage>) {
    let config_val = Config {
        force_rollup_on_shutdown: false,
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                TimeLevel {
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 1,
                        max_page_age_seconds: 1000,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None, name: "L0".to_string(), duration_seconds: 1 },
                TimeLevel {
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 1,
                        max_page_age_seconds: 1000,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None, name: "L1".to_string(), duration_seconds: 1 },
                TimeLevel {
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 1,
                        max_page_age_seconds: 1000,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None, name: "L2".to_string(), duration_seconds: 1 },
            ],
        },
        storage: Default::default(),
        retention: RetentionConfig { // Default retention disabled for these rollup tests
            enabled: false,
            period_seconds: 0,
            cleanup_interval_seconds: 300, // Assuming a default, consistent with create_test_config
        },
        compression: Default::default(),
        logging: Default::default(),
        metrics: Default::default(),
    };

    let config = Arc::new(config_val);
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());
    (manager, storage)
}

#[tokio::test]
async fn test_cascading_rollup_max_items() {
    
    let (manager, storage) = create_cascading_test_config_and_manager();
    let timestamp = Utc::now();

    // Leaf 1 - should trigger full cascade through L0, L1, L2
    let leaf1 = JournalLeaf::new(
        timestamp,
        None,
        "cascade_container".to_string(),
        json!({ "data": "leaf1_for_cascade" })
    ).expect("Leaf1 creation failed in test_cascading_rollup_max_items");
    manager.add_leaf(&leaf1, leaf1.timestamp).await.expect("Failed to add leaf1 for cascade");

    // L0P0 (ID 0) assertions
    let stored_l0_page = storage.load_page(0, 0).await.expect("Storage query failed for L0P0 in cascade").expect("L0P0 not found after cascade");
    assert_eq!(stored_l0_page.page_id, 0);
    assert_eq!(stored_l0_page.level, 0);
    assert_eq!(stored_l0_page.content_len(), 1);
    match &stored_l0_page.content {
        PageContent::Leaves(leaves) => {
            assert_eq!(leaves.len(), 1, "L0 page should have 1 leaf");
            assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0 leaf hash mismatch");
        }
        _ => panic!("L0 page content should be Leaves, found {:?}", stored_l0_page.content),
    }

    // L1P1 (ID 1) assertions
    let stored_l1_page = storage.load_page(1, 1).await.expect("Storage query failed for L1P1 in cascade").expect("L1P1 not found after cascade");
    assert_eq!(stored_l1_page.page_id, 1);
    assert_eq!(stored_l1_page.level, 1);
    assert_eq!(stored_l1_page.content_len(), 1);
    match &stored_l1_page.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "L1 page should have 1 thrall hash");
            assert_eq!(hashes[0], stored_l0_page.page_hash, "L1 thrall hash mismatch");
        }
        _ => panic!("L1 page content should be ThrallHashes, found {:?}", stored_l1_page.content),
    }
    assert_eq!(stored_l1_page.prev_page_hash, None, "L1P1 should not have a prev_page_hash (first in its level)");

    // L2P2 (ID 2) assertions (highest level in this config)
    let stored_l2_page = storage.load_page(2, 2).await.expect("Storage query failed for L2P2 in cascade").expect("L2P2 not found after cascade");
    assert_eq!(stored_l2_page.page_id, 2);
    assert_eq!(stored_l2_page.level, 2);
    assert_eq!(stored_l2_page.content_len(), 1);
    match &stored_l2_page.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len(), 1, "L2 page should have 1 thrall hash");
            assert_eq!(hashes[0], stored_l1_page.page_hash, "L2 thrall hash mismatch");
        }
        _ => panic!("L2 page content should be ThrallHashes, found {:?}", stored_l2_page.content),
    }
    assert_eq!(stored_l2_page.prev_page_hash, None, "L2P2 should not have a prev_page_hash (first in its level)");
    
    // Check active pages are empty for L0, L1, L2 as all should have finalized
    assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "L0 should have no active page after cascade");
    assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "L1 should have no active page after cascade");
    assert!(manager.active_pages.lock().await.get(&2u8).is_none(), "L2 should have no active page after cascade (top level finalized)");

    // Check last_finalized_page_ids and last_finalized_page_hashes
    assert_eq!(manager.last_finalized_page_ids.lock().await.get(&0u8).copied().expect("L0 ID not in last_finalized_page_ids after cascade"), 0, "L0 last finalized ID mismatch");
    assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&0u8).copied().expect("L0 hash not in last_finalized_page_hashes after cascade"), stored_l0_page.page_hash, "L0 last finalized hash mismatch");
    
    assert_eq!(manager.last_finalized_page_ids.lock().await.get(&1u8).copied().expect("L1 ID not in last_finalized_page_ids after cascade"), 1, "L1 last finalized ID mismatch");
    assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&1u8).copied().expect("L1 hash not in last_finalized_page_hashes after cascade"), stored_l1_page.page_hash, "L1 last finalized hash mismatch");

    assert_eq!(manager.last_finalized_page_ids.lock().await.get(&2u8).copied().expect("L2 ID not in last_finalized_page_ids after cascade"), 2, "L2 last finalized ID mismatch");
    assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&2u8).copied().expect("L2 hash not in last_finalized_page_hashes after cascade"), stored_l2_page.page_hash, "L2 last finalized hash mismatch");
}

#[tokio::test]
async fn test_retention_disabled() {
    
    let config = Arc::new(create_test_config(false, 60, None, None)); // Retention disabled, 60s period (irrelevant)
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();
    let old_page_time = now - Duration::seconds(120); // 2 minutes ago, older than 60s

    // Create and store an old page
    let old_page = create_retention_test_page(0, 0, old_page_time, &config);
    storage.store_page(&old_page).await.unwrap();
    assert!(storage.page_exists(0, old_page.page_id).await.unwrap(), "Old page should exist before retention");

    manager.apply_retention_policies().await.unwrap();

    assert!(storage.page_exists(0, old_page.page_id).await.unwrap(), "Old page should still exist as retention is disabled");
}

#[tokio::test]
async fn test_retention_period_zero() {
    
    // Retention enabled, but period is 0 seconds
    let config = Arc::new(create_test_config(true, 0, None, None)); 
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();
    let old_page_time = now - Duration::seconds(120); // 2 minutes ago

    let old_page = create_retention_test_page(0, 0, old_page_time, &config);
    storage.store_page(&old_page).await.unwrap();
    assert!(storage.page_exists(0, old_page.page_id).await.unwrap(), "Old page should exist before retention");

    manager.apply_retention_policies().await.unwrap();

    // Page should still exist because a 0-second retention period means nothing is ever old enough to delete
    assert!(storage.page_exists(0, old_page.page_id).await.unwrap(), "Old page should still exist with 0s retention period");
}

#[tokio::test]
async fn test_retention_no_pages_old_enough() {
    
    let retention_period_seconds = 60;
    let config = Arc::new(create_test_config(true, retention_period_seconds, None, None)); 
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();
    // Page is newer than retention period (30s old vs 60s period)
    let newish_page_time = now - Duration::seconds(30); 

    let newish_page = create_retention_test_page(0, 0, newish_page_time, &config);
    storage.store_page(&newish_page).await.unwrap();
    assert!(storage.page_exists(0, newish_page.page_id).await.unwrap(), "Newish page should exist before retention");

    manager.apply_retention_policies().await.unwrap();

    assert!(storage.page_exists(0, newish_page.page_id).await.unwrap(), "Newish page should still exist as it's not old enough");
}

#[tokio::test]
async fn test_retention_some_pages_deleted() {
    
    let retention_period_seconds = 60;
    let config = Arc::new(create_test_config(true, retention_period_seconds, None, None));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();
    let old_page_time = now - Duration::seconds(120); // 2 minutes old, older than 60s period
    let new_page_time = now - Duration::seconds(30);  // 30 seconds old, newer than 60s period

    // Create and store an old page (page_id will be offset by 0 from current NEXT_PAGE_ID)
    let old_page = create_retention_test_page(0, 0, old_page_time, &config);
    storage.store_page(&old_page).await.unwrap();
    let old_page_id = old_page.page_id;
    assert!(storage.page_exists(0, old_page_id).await.unwrap(), "Old page should exist before retention");

    // Create and store a new page (page_id will be offset by 1 from current NEXT_PAGE_ID after old_page)
    let new_page = create_retention_test_page(0, 1, new_page_time, &config); 
    storage.store_page(&new_page).await.unwrap();
    let new_page_id = new_page.page_id;
    assert!(storage.page_exists(0, new_page_id).await.unwrap(), "New page should exist before retention");
    assert_ne!(old_page_id, new_page_id, "Page IDs must be different");

    manager.apply_retention_policies().await.unwrap();

    assert!(!storage.page_exists(0, old_page_id).await.unwrap(), "Old page should have been deleted");
    assert!(storage.page_exists(0, new_page_id).await.unwrap(), "New page should still exist");
}

#[tokio::test]
async fn test_retention_all_pages_deleted() {
    
    let retention_period_seconds = 60;
    let config = Arc::new(create_test_config(true, retention_period_seconds, None, None));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();
    let very_old_page_time = now - Duration::seconds(120); // 2 minutes old
    let also_old_page_time = now - Duration::seconds(90);  // 1.5 minutes old

    let page1 = create_retention_test_page(0, 0, very_old_page_time, &config);
    storage.store_page(&page1).await.unwrap();
    let page1_id = page1.page_id;
    assert!(storage.page_exists(0, page1_id).await.unwrap(), "Page 1 should exist before retention");

    let page2 = create_retention_test_page(0, 1, also_old_page_time, &config);
    storage.store_page(&page2).await.unwrap();
    let page2_id = page2.page_id;
    assert!(storage.page_exists(0, page2_id).await.unwrap(), "Page 2 should exist before retention");

    manager.apply_retention_policies().await.unwrap();

    assert!(!storage.page_exists(0, page1_id).await.unwrap(), "Page 1 should have been deleted");
    assert!(!storage.page_exists(0, page2_id).await.unwrap(), "Page 2 should have been deleted");
}

#[tokio::test]
async fn test_retention_multi_level() {
    
    let retention_period_seconds = 60;
    // Config has 2 levels by default from create_test_config: L0 (1s), L1 (60s)
    let config = Arc::new(create_test_config(true, retention_period_seconds, None, None));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();

    // Level 0 pages
    let l0_old_time = now - Duration::seconds(120); // 2 mins old, > 60s retention
    let l0_new_time = now - Duration::seconds(30);  // 30s old, < 60s retention
    let l0_old_page = create_retention_test_page(0, 0, l0_old_time, &config);
    let l0_new_page = create_retention_test_page(0, 1, l0_new_time, &config);
    storage.store_page(&l0_old_page).await.unwrap();
    storage.store_page(&l0_new_page).await.unwrap();

    // Level 1 pages
    let l1_very_old_time = now - Duration::seconds(180); // Page ended 120s ago. Age is 120s. Should be deleted.
    let l1_new_time = now - Duration::seconds(70);  // Page ended 10s ago. Age is 10s. Should be kept.

    let l1_old_page = create_retention_test_page(1, 2, l1_very_old_time, &config);
    let l1_new_page = create_retention_test_page(1, 3, l1_new_time, &config);
    storage.store_page(&l1_old_page).await.unwrap();
    storage.store_page(&l1_new_page).await.unwrap();

    assert!(storage.page_exists(0, l0_old_page.page_id).await.unwrap(), "L0 old page pre-retention");
    assert!(storage.page_exists(0, l0_new_page.page_id).await.unwrap(), "L0 new page pre-retention");
    assert!(storage.page_exists(1, l1_old_page.page_id).await.unwrap(), "L1 old page pre-retention");
    assert!(storage.page_exists(1, l1_new_page.page_id).await.unwrap(), "L1 new page pre-retention");

    manager.apply_retention_policies().await.unwrap();

    assert!(!storage.page_exists(0, l0_old_page.page_id).await.unwrap(), "L0 old page should be deleted");
    assert!(storage.page_exists(0, l0_new_page.page_id).await.unwrap(), "L0 new page should be kept");
    assert!(!storage.page_exists(1, l1_old_page.page_id).await.unwrap(), "L1 old page should be deleted");
    assert!(storage.page_exists(1, l1_new_page.page_id).await.unwrap(), "L1 new page should be kept");
}

#[tokio::test]
async fn test_retention_l0_keep_n_pages() {
    
    let keep_n = 2;
    let config = Arc::new(create_test_config(
        true, 
        60,   
        Some(RollupRetentionPolicy::KeepNPages(keep_n)), 
        None, 
    ));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();
    let page1_time = now - Duration::seconds(400);
    let page2_time = now - Duration::seconds(300);
    let page3_time = now - Duration::seconds(200);
    let page4_time = now - Duration::seconds(100);

    let page1 = create_retention_test_page(0, 0, page1_time, &config);
    let page2 = create_retention_test_page(0, 1, page2_time, &config);
    let page3 = create_retention_test_page(0, 2, page3_time, &config);
    let page4 = create_retention_test_page(0, 3, page4_time, &config);

    storage.store_page(&page1).await.unwrap();
    storage.store_page(&page2).await.unwrap();
    storage.store_page(&page3).await.unwrap();
    storage.store_page(&page4).await.unwrap();

    assert!(storage.page_exists(0, page1.page_id).await.unwrap(), "L0 Page 1 pre-retention");
    assert!(storage.page_exists(0, page2.page_id).await.unwrap(), "L0 Page 2 pre-retention");
    assert!(storage.page_exists(0, page3.page_id).await.unwrap(), "L0 Page 3 pre-retention");
    assert!(storage.page_exists(0, page4.page_id).await.unwrap(), "L0 Page 4 pre-retention");

    manager.apply_retention_policies().await.unwrap();

    assert!(!storage.page_exists(0, page1.page_id).await.unwrap(), "L0 Page 1 (oldest) should be deleted by KeepNPages(2)");
    assert!(!storage.page_exists(0, page2.page_id).await.unwrap(), "L0 Page 2 (second oldest) should be deleted by KeepNPages(2)");
    assert!(storage.page_exists(0, page3.page_id).await.unwrap(), "L0 Page 3 (second newest) should be kept by KeepNPages(2)");
    assert!(storage.page_exists(0, page4.page_id).await.unwrap(), "L0 Page 4 (newest) should be kept by KeepNPages(2)");
}

#[tokio::test]
async fn test_retention_l0_keep_n_pages_zero() {
    
    let config = Arc::new(create_test_config(
        true, 
        60,   
        Some(RollupRetentionPolicy::KeepNPages(0)), 
        None, 
    ));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();
    let page1_time = now - Duration::seconds(200);
    let page2_time = now - Duration::seconds(100);

    let page1 = create_retention_test_page(0, 0, page1_time, &config);
    let page2 = create_retention_test_page(0, 1, page2_time, &config);

    storage.store_page(&page1).await.unwrap();
    storage.store_page(&page2).await.unwrap();

    assert!(storage.page_exists(0, page1.page_id).await.unwrap(), "L0 Page 1 pre-retention");
    assert!(storage.page_exists(0, page2.page_id).await.unwrap(), "L0 Page 2 pre-retention");

    manager.apply_retention_policies().await.unwrap();

    assert!(!storage.page_exists(0, page1.page_id).await.unwrap(), "L0 Page 1 should be deleted by KeepNPages(0)");
    assert!(!storage.page_exists(0, page2.page_id).await.unwrap(), "L0 Page 2 should be deleted by KeepNPages(0)");
}

#[tokio::test]
async fn test_retention_mixed_policies_l0_keep_n_l1_delete_after_secs() {
    
    let l0_keep_n = 1;
    let l1_delete_after_secs = 30u64;

    let config = Arc::new(create_test_config(
        false, 
        0,     
        Some(RollupRetentionPolicy::KeepNPages(l0_keep_n)), 
        Some(RollupRetentionPolicy::DeleteAfterSecs(l1_delete_after_secs)), 
    ));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();

    let l0_page_old_time = now - Duration::seconds(200);
    let l0_page_new_time = now - Duration::seconds(100);
    let l0_page_old = create_retention_test_page(0, 0, l0_page_old_time, &config);
    let l0_page_new = create_retention_test_page(0, 1, l0_page_new_time, &config);
    storage.store_page(&l0_page_old).await.unwrap();
    storage.store_page(&l0_page_new).await.unwrap();

    let l1_page_old_start_time = now - Duration::seconds(100); 
    let l1_page_new_start_time = now - Duration::seconds(70);  
    
    let l1_page_old = create_retention_test_page(1, 2, l1_page_old_start_time, &config);
    let l1_page_new = create_retention_test_page(1, 3, l1_page_new_start_time, &config);
    storage.store_page(&l1_page_old).await.unwrap();
    storage.store_page(&l1_page_new).await.unwrap();

    assert!(storage.page_exists(0, l0_page_old.page_id).await.unwrap(), "L0 old pre-retention");
    assert!(storage.page_exists(0, l0_page_new.page_id).await.unwrap(), "L0 new pre-retention");
    assert!(storage.page_exists(1, l1_page_old.page_id).await.unwrap(), "L1 old pre-retention");
    assert!(storage.page_exists(1, l1_page_new.page_id).await.unwrap(), "L1 new pre-retention");

    manager.apply_retention_policies().await.unwrap();

    assert!(!storage.page_exists(0, l0_page_old.page_id).await.unwrap(), "L0 old should be deleted by KeepNPages(1)");
    assert!(storage.page_exists(0, l0_page_new.page_id).await.unwrap(), "L0 new should be kept by KeepNPages(1)");

    assert!(!storage.page_exists(1, l1_page_old.page_id).await.unwrap(), "L1 old (age 40s) should be deleted by DeleteAfterSecs(30)");
    assert!(storage.page_exists(1, l1_page_new.page_id).await.unwrap(), "L1 new (age 10s) should be kept by DeleteAfterSecs(30)");
} 

// Helper function to create a configuration suitable for testing age-based rollups
fn create_age_based_rollup_config_and_manager(
    l0_max_age_secs: u64, // Corrected: this was l0_max_leaves in original, should be age
    l0_max_leaves: u32,   // Corrected: this was high_age_limit, should be leaves
    l1_max_age_secs: u64, // Corrected: this was l1_max_leaves, should be age
    l1_max_leaves: u32,   // Corrected: this was high_age_limit, should be leaves
) -> (TimeHierarchyManager, Arc<MemoryStorage>) {
    let config_val = Config {
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                TimeLevel { // Level 0
                    name: "L0_age_test".to_string(),
                    duration_seconds: 60,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: l0_max_leaves as usize,
                        max_page_age_seconds: l0_max_age_secs,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                },
                TimeLevel { // Level 1
                    name: "L1_age_test".to_string(),
                    duration_seconds: 3600,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: l1_max_leaves as usize,
                        max_page_age_seconds: l1_max_age_secs,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                },
            ],
        },
        force_rollup_on_shutdown: false, // Explicitly set for clarity in this test helper
        storage: TestStorageConfig {
            storage_type: TestStorageType::Memory,
            base_path: "".to_string(),
            max_open_files: 100, 
        },
        retention: RetentionConfig {
            enabled: false, 
            period_seconds: 0,
            cleanup_interval_seconds: 300,
        },
        compression: CompressionConfig { enabled: false, algorithm: CompressionAlgorithm::None, level: 0 },
        logging: LoggingConfig::default(),
        metrics: MetricsConfig::default(),
    };
    let config = Arc::new(config_val);
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());
    (manager, storage)
}

#[tokio::test]
async fn test_age_based_rollup_cascade() {
    let test_body = async {
        println!("[TEST-DEBUG] Starting test_age_based_rollup_cascade");
        
        println!("[TEST-DEBUG] Setting up config with l0_max_age={}, l1_max_age={}", 2, 4);
        let l0_max_age = 2; 
        let l1_max_age = 4; 
        let (manager, _storage) = create_age_based_rollup_config_and_manager(l0_max_age, 100, l1_max_age, 100);

        println!("[TEST-DEBUG] Creating initial time and leaves");
        let initial_time = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(); 

        // Leaf 1 (time t)
        let leaf1_ts = initial_time;
        let leaf1 = JournalLeaf::new(leaf1_ts, None, "container1".to_string(), json!({"data": "leaf1"})).unwrap();
        manager.add_leaf(&leaf1, leaf1.timestamp).await.unwrap();

        // Leaf 2 (time t + 3s) - should trigger L0 rollup due to age (2s)
        let leaf2_ts = initial_time + Duration::seconds(3);
        let leaf2 = JournalLeaf::new(leaf2_ts, None, "container1".to_string(), json!({"data": "leaf2"})).unwrap();
        manager.add_leaf(&leaf2, leaf2.timestamp).await.unwrap();

        // Leaf 3 (time t + 61s) - new L0 page
        let leaf3_ts = initial_time + Duration::seconds(61);
        let leaf3 = JournalLeaf::new(leaf3_ts, None, "container1".to_string(), json!({"data": "leaf3"})).unwrap();
        manager.add_leaf(&leaf3, leaf3.timestamp).await.unwrap();

        // Leaf 4 (time t + 63s) - should trigger L0 rollup (age 2s), then L1 rollup (age 4s for L1 page containing L0P0)
        // THIS IS WHERE THE HANG OCCURS
        let leaf4_ts = initial_time + Duration::seconds(63);
        let leaf4 = JournalLeaf::new(leaf4_ts, None, "container1".to_string(), json!({"data": "leaf4"})).unwrap();
        manager.add_leaf(&leaf4, leaf4.timestamp).await.unwrap();

        println!("[TEST-DEBUG] test_age_based_rollup_cascade logic completed (if not hung)");
    };

    match timeout(TokioDuration::from_secs(15), test_body).await {
        Ok(_) => println!("[TEST-DEBUG] Test completed within timeout."),
        Err(_) => panic!("[TEST-DEBUG] Test timed out after 15 seconds!"),
    }
}
#[tokio::test]
async fn test_parent_rollup_when_page_capacity_exceeded() {
    let mut config = create_test_config(false, 0, None, None);
    if config.time_hierarchy.levels.len() > 1 {
        config.time_hierarchy.levels[1].rollup_config.max_items_per_page = 2;
    }
    config.storage.storage_type = TestStorageType::Memory;
    config.storage.base_path = "".to_string();
    let config = Arc::new(config);
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let ts = Utc::now();
    let leaf1 = JournalLeaf::new(ts, None, "c".to_string(), json!({"n":1})).unwrap();
    manager.add_leaf(&leaf1, leaf1.timestamp).await.unwrap();

    assert!(storage.load_page(1, 1).await.unwrap().is_none());
    {
        let guard = manager.active_pages.lock().await;
        let page = guard.get(&1u8).expect("active L1 page after first leaf");
        assert_eq!(page.content_len(), 1);
    }

    let leaf2 = JournalLeaf::new(ts + Duration::seconds(1), None, "c".to_string(), json!({"n":2})).unwrap();
    manager.add_leaf(&leaf2, leaf2.timestamp).await.unwrap();

    let stored_l1_p1 = storage.load_page(1, 1).await.unwrap().expect("L1P1 stored");
    assert!(manager.active_pages.lock().await.get(&1u8).is_none());
    if let PageContent::ThrallHashes(hashes) = &stored_l1_p1.content {
        assert_eq!(hashes.len(), 2);
    } else { panic!("expected thrall hashes"); }

    let leaf3 = JournalLeaf::new(ts + Duration::seconds(2), None, "c".to_string(), json!({"n":3})).unwrap();
    manager.add_leaf(&leaf3, leaf3.timestamp).await.unwrap();

    assert!(storage.load_page(1, 3).await.unwrap().is_none());
    {
        let guard = manager.active_pages.lock().await;
        let page = guard.get(&1u8).expect("new active L1 page");
        assert_eq!(page.page_id, 4); // next page ID after finalized parent
        assert_eq!(page.content_len(), 1);
    }
}

#[tokio::test]
async fn test_age_based_rollup_finalizes_parent_page() {
    let (manager, storage) = create_age_based_rollup_config_and_manager(2, 100, 4, 100);
    let base = Utc::now();

    let l1 = JournalLeaf::new(base, None, "c".to_string(), json!({"v":1})).unwrap();
    manager.add_leaf(&l1, l1.timestamp).await.unwrap();

    let l2 = JournalLeaf::new(base + Duration::seconds(3), None, "c".to_string(), json!({"v":2})).unwrap();
    manager.add_leaf(&l2, l2.timestamp).await.unwrap();

    let l3 = JournalLeaf::new(base + Duration::seconds(8), None, "c".to_string(), json!({"v":3})).unwrap();
    manager.add_leaf(&l3, l3.timestamp).await.unwrap();

    let l1_page = storage.load_page(1, 1).await.unwrap().expect("L1 page finalized");
    if let PageContent::ThrallHashes(hashes) = &l1_page.content {
        assert!(!hashes.is_empty());
    } else {
        panic!("expected thrall hashes");
    }
}
