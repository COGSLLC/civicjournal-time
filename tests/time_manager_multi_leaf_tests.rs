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
use chrono::Utc;
use serde_json::json;

// Helper to create a test config with specific max_leaves_per_page for L1
fn create_multi_leaf_test_config(l1_max_leaves: u32) -> Config {
    Config {
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                // L0: Small pages that will fill up quickly
                TimeLevel {
                    name: "seconds".to_string(),
                    duration_seconds: 1,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 1,  // Small pages to trigger rollups quickly
                        max_page_age_seconds: 60,  // Not the focus of this test
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                },
                // L1: Configured to accumulate multiple thrall hashes
                TimeLevel {
                    name: "minutes".to_string(),
                    duration_seconds: 60,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: l1_max_leaves as usize,  // Will accumulate multiple thrall hashes
                        max_page_age_seconds: 3600,  // Long enough to not interfere with test
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                },
            ],
        },
        force_rollup_on_shutdown: false,
        storage: StorageConfig {
            storage_type: StorageType::Memory,
            base_path: "./test_data".to_string(),
            max_open_files: 1000,
        },
        retention: RetentionConfig {
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
        },
    }
}

#[tokio::test]
async fn test_multi_leaf_parent_rollup() {
    // Configure L1 to accumulate 3 thrall hashes before finalizing
    let l1_max_leaves = 3;
    let config = Arc::new(create_multi_leaf_test_config(l1_max_leaves));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config, storage.clone());

    let now = Utc::now();
    
    // Add leaves to create multiple L0 pages that will roll up to L1
    for i in 0..5 {
        let leaf_ts = now + chrono::Duration::milliseconds(i * 100);
        let leaf = JournalLeaf::new(
            leaf_ts,
            None,
            format!("container_{}", i),
            json!({ "id": i })
        ).unwrap();
        
        manager.add_leaf(&leaf, leaf.timestamp).await.unwrap();
        
        // Check L0 state after each leaf
        let l0_pages = storage.list_finalized_pages_summary(0).await.unwrap();
        let active_pages = manager.active_pages.lock().await;
        
        // After each leaf, we should have i+1 L0 pages (since max_items_per_page=1)
        assert_eq!(
            l0_pages.len() as i32 + if active_pages.get(&0).is_some() { 1 } else { 0 },
            (i + 1) as i32,
            "Incorrect number of L0 pages after {} leaves", i+1
        );
        
        // Check L1 state
        let l1_pages = storage.list_finalized_pages_summary(1).await.unwrap();
        let active_l1 = active_pages.get(&1).is_some();
        
        // L1 should only finalize after accumulating l1_max_leaves thrall hashes
        let expected_finalized_l1_pages = (i + 1) / l1_max_leaves as i64;
        assert_eq!(
            l1_pages.len() as i64,
            expected_finalized_l1_pages,
            "Incorrect number of finalized L1 pages after {} leaves", i+1
        );
        
        // If we haven't filled up the L1 page yet, it should be active
        assert_eq!(
            active_l1,
            ((i + 1) as u32 % l1_max_leaves) != 0,
            "L1 active state incorrect after {} leaves", i+1
        );
    }
    
    // Verify final state
    let l1_pages = storage.list_finalized_pages_summary(1).await.unwrap();
    assert_eq!(l1_pages.len(), 1, "Should have 1 finalized L1 page");
    
    // The last L1 page might still be active if not filled up
    let active_pages = manager.active_pages.lock().await;
    let active_l1 = active_pages.get(&1).is_some();
    assert!(active_l1, "Last L1 page should still be active");
    
    // Debug: Print active pages and finalized pages
    println!("Active pages: {:?}", active_pages.keys().collect::<Vec<_>>());
    println!("Finalized L1 pages: {:?}", storage.list_finalized_pages_summary(1).await.unwrap());
    
    // Check the content of the L1 page
    let l1_page = active_pages.get(&1).unwrap().clone();
    match &l1_page.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len() as u32, 5 % l1_max_leaves, "Active L1 page should have 2 hashes (5 % 3)");
        }
        _ => panic!("Expected ThrallHashes for active L1 page"),
    }
    
    // Check the finalized L1 page
    let finalized_pages = storage.list_finalized_pages_summary(1).await.unwrap();
    println!("Finalized pages details: {:?}", finalized_pages);
    // Get the first (and should be only) finalized L1 page
    let page_id = finalized_pages[0].page_id;
    let finalized_l1_page = storage.load_page(1, page_id).await.unwrap().unwrap();
    match &finalized_l1_page.content {
        PageContent::ThrallHashes(hashes) => {
            assert_eq!(hashes.len() as u32, l1_max_leaves, "Finalized L1 page should have {} hashes", l1_max_leaves);
        }
        _ => panic!("Expected ThrallHashes for finalized L1 page"),
    }
}
