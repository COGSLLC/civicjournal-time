use civicjournal_time::core::time_manager::TimeHierarchyManager;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::page::PageContent;
use civicjournal_time::types::time::RollupContentType;
use civicjournal_time::storage::memory::MemoryStorage;
use civicjournal_time::config::{
    Config, TimeHierarchyConfig, TimeLevel, LevelRollupConfig, StorageConfig, LoggingConfig, MetricsConfig,
    RetentionConfig, CompressionConfig
};
use civicjournal_time::StorageType;
use civicjournal_time::LogLevel;
use civicjournal_time::core::leaf::JournalLeaf;
use std::sync::Arc;
use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::time::{sleep, Duration as TokioDuration};
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};

// Helper to create a test config with specific age-based rollup settings
fn create_age_based_test_config(
    l0_max_age_secs: u64,
    l1_max_age_secs: u64,
) -> Config {
    Config {
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                // L0: Configured to roll up based on age
                TimeLevel {
                    name: "seconds".to_string(),
                    duration_seconds: 1,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 100,  // High enough to not trigger item-based rollup
                        max_page_age_seconds: l0_max_age_secs,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                },
                // L1: Also configured for age-based rollup
                TimeLevel {
                    name: "minutes".to_string(),
                    duration_seconds: 60,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 100,  // High enough to not trigger item-based rollup
                        max_page_age_seconds: l1_max_age_secs,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                },
                // L2: Top level, no rollup
                TimeLevel {
                    name: "hours".to_string(),
                    duration_seconds: 3600,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 100,
                        max_page_age_seconds: 0,  // No rollup for top level
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                },
            ],
        },
        retention: Default::default(),
        snapshot: Default::default(), // Added missing field
        force_rollup_on_shutdown: false,
        storage: StorageConfig {
            storage_type: StorageType::Memory,
            base_path: "./test_data".to_string(),
            max_open_files: 1000,
        },
            enabled: false,
            period_seconds: 0,
            cleanup_interval_seconds: 300,
        },
        compression: CompressionConfig::default(),
        logging: LoggingConfig {
            level: LogLevel::Info,
            console: true,
            file: false,
            file_path: "./test.log".to_string(),
        },
        metrics: MetricsConfig {
            enabled: false,
            endpoint: "".to_string(),
            push_interval_seconds: 15,
        }
    }

#[tokio::test]
async fn test_age_based_rollup() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    // Configure short age thresholds for testing (in seconds)
    let l0_max_age = 2;  // L0 pages older than 2 seconds should roll up
    let l1_max_age = 5;  // L1 pages older than 5 seconds should roll up
    
    let config = Arc::new(create_age_based_test_config(l0_max_age, l1_max_age));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config, storage.clone());
    
    let start_time = Utc::now();
    
    // Add initial leaf to create an L0 page
    let leaf1 = JournalLeaf::new(
        start_time,
        None,
        "container_1".to_string(),
        json!({ "id": 1 })
    ).unwrap();
    manager.add_leaf(&leaf1, leaf1.timestamp).await.unwrap();
    
    // Verify L0 page was created and is active
    let active_pages = manager.active_pages.lock().await;
    assert!(active_pages.get(&0).is_some(), "L0 page should be active after first leaf");
    assert!(active_pages.get(&1).is_none(), "L1 page should not be active yet");
    drop(active_pages);
    
    // Wait for L0 page to exceed max age
    sleep(TokioDuration::from_secs(l0_max_age + 1)).await;
    
    // Add another leaf to trigger rollup check
    let leaf2 = JournalLeaf::new(
        Utc::now(),
        None,
        "container_2".to_string(),
        json!({ "id": 2 })
    ).unwrap();
    manager.add_leaf(&leaf2, leaf2.timestamp).await.unwrap();
    
    // Verify L0 page was rolled up to L1
    let l0_pages = storage.list_finalized_pages_summary(0).await.unwrap();
    assert_eq!(l0_pages.len(), 1, "Should have 1 finalized L0 page");
    
    // L1 page might be active or already finalized, depending on timing
    let active_pages = manager.active_pages.lock().await;
    let l1_pages = storage.list_finalized_pages_summary(1).await.unwrap();
    
    if let Some(l1_page) = active_pages.get(&1) {
        // L1 page is still active
        match &l1_page.content {
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 1, "Active L1 page should have 1 thrall hash");
            }
            _ => panic!("Expected ThrallHashes for active L1 page"),
        }
    } else {
        // L1 page was already finalized
        assert_eq!(l1_pages.len(), 1, "Should have 1 finalized L1 page");
        let l1_page = storage.load_page(1, l1_pages[0].page_id).await.unwrap().unwrap();
        match &l1_page.content {
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 1, "Finalized L1 page should have 1 thrall hash");
            }
            _ => panic!("Expected ThrallHashes for finalized L1 page"),
        }
    }
    drop(active_pages);
    
    // Wait for L1 page to exceed max age
    sleep(TokioDuration::from_secs(l1_max_age - l0_max_age + 1)).await;
    
    // Add another leaf to trigger L1 rollup check
    let leaf3 = JournalLeaf::new(
        Utc::now(),
        None,
        "container_3".to_string(),
        json!({ "id": 3 })
    ).unwrap();
    manager.add_leaf(&leaf3, leaf3.timestamp).await.unwrap();
    
    // Verify L1 page was rolled up to L2
    let mut l1_pages = storage.list_finalized_pages_summary(1).await.unwrap();
    if l1_pages.is_empty() {
        sleep(TokioDuration::from_secs(1)).await;
        l1_pages = storage.list_finalized_pages_summary(1).await.unwrap();
    }
    assert_eq!(l1_pages.len(), 1, "Should have 1 finalized L1 page");
    
    // L2 page might be active or already finalized
    let active_pages = manager.active_pages.lock().await;
    let l2_pages = storage.list_finalized_pages_summary(2).await.unwrap();
    
    // Check L2 page content (either active or finalized)
    let l2_page = if let Some(page) = active_pages.get(&2) {
        // L2 page is still active
        page.clone()
    } else {
        // L2 page was finalized
        assert_eq!(l2_pages.len(), 1, "Should have 1 finalized L2 page");
        storage.load_page(2, l2_pages[0].page_id).await.unwrap().unwrap()
    };
    
    match &l2_page.content {
        PageContent::ThrallHashes(hashes) => {
            // We expect 2 hashes because both L1 pages have been rolled up to this L2 page
            assert_eq!(hashes.len(), 2, "L2 page should have 2 thrall hashes (one from each L1 page)");
        }
        _ => panic!("Expected ThrallHashes for L2 page"),
    }
}

#[tokio::test]
async fn test_cascading_age_based_rollup() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    // Configure very short age thresholds for testing cascading rollups
    let l0_max_age = 1;
    let l1_max_age = 2;
    
    let config = Arc::new(create_age_based_test_config(l0_max_age, l1_max_age));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config, storage.clone());
    
    // Add initial leaf to create an L0 page
    let leaf1 = JournalLeaf::new(
        Utc::now(),
        None,
        "container_1".to_string(),
        json!({ "id": 1 })
    ).unwrap();
    manager.add_leaf(&leaf1, leaf1.timestamp).await.unwrap();
    
    // Wait for L0 page to exceed max age
    sleep(TokioDuration::from_secs(l0_max_age + 1)).await;
    
    // Add second leaf to trigger L0 rollup
    let leaf2 = JournalLeaf::new(
        Utc::now(),
        None,
        "container_2".to_string(),
        json!({ "id": 2 })
    ).unwrap();
    manager.add_leaf(&leaf2, leaf2.timestamp).await.unwrap();
    
    // Verify L0 and L1 pages were rolled up to L2
    let l0_pages = storage.list_finalized_pages_summary(0).await.unwrap();
    let l1_pages = storage.list_finalized_pages_summary(1).await.unwrap();
    
    // We might have more than one L0 page due to timing
    assert!(l0_pages.len() >= 1, "Should have at least 1 finalized L0 page");
    
    // L1 page should be either active or finalized
    let active_pages = manager.active_pages.lock().await;
    let l2_active = active_pages.get(&2).is_some();
    
    if l2_active {
        // L2 page is active, so L1 page should be finalized
        assert_eq!(l1_pages.len(), 1, "Should have 1 finalized L1 page");
    } else {
        // L2 page might be finalized
        let l2_pages = storage.list_finalized_pages_summary(2).await.unwrap();
        assert_eq!(l2_pages.len(), 1, "Should have 1 finalized L2 page");
    }
    
    // Get the most recent L0 and L1 pages
    let l0_page = storage.load_page(0, l0_pages[0].page_id).await.unwrap().unwrap();
    let l1_page = storage.load_page(1, l1_pages[0].page_id).await.unwrap().unwrap();
    
    // Get L2 page (either active or finalized)
    let l2_page = if let Some(page) = active_pages.get(&2) {
        page.clone()
    } else {
        let l2_pages = storage.list_finalized_pages_summary(2).await.unwrap();
        storage.load_page(2, l2_pages[0].page_id).await.unwrap().unwrap()
    };
    
    match (&l1_page.content, &l2_page.content) {
        (PageContent::ThrallHashes(l1_hashes), PageContent::ThrallHashes(l2_hashes)) => {
            assert_eq!(l1_hashes[0], l0_page.page_hash, "L1 should contain L0's hash");
            assert_eq!(l2_hashes[0], l1_page.page_hash, "L2 should contain L1's hash");
        }
        _ => panic!("Unexpected page content types"),
    }
}
