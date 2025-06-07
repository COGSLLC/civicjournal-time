use civicjournal_time::core::time_manager::TimeHierarchyManager;
use civicjournal_time::storage::memory::MemoryStorage;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::core::page::{JournalPage, PageContent, PageIdGenerator};
use civicjournal_time::types::time::{RollupContentType, RollupRetentionPolicy};
use civicjournal_time::config::{Config, TimeHierarchyConfig, TimeLevel, LevelRollupConfig, StorageConfig, CompressionConfig, LoggingConfig, MetricsConfig, RetentionConfig};
use civicjournal_time::StorageType;
use chrono::{Utc, Duration, DateTime, TimeZone};
use serde_json::json;
use std::sync::Arc;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};

fn build_single_level_config(max_items: usize, retention: Option<RollupRetentionPolicy>, retention_enabled: bool, retention_secs: u64) -> Config {
    Config {
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![TimeLevel {
                name: "L0".to_string(),
                duration_seconds: 60,
                rollup_config: LevelRollupConfig {
                    max_items_per_page: max_items,
                    max_page_age_seconds: 1000,
                    content_type: RollupContentType::ChildHashes,
                },
                retention_policy: retention,
            }],
        },
        snapshot: Default::default(),
        force_rollup_on_shutdown: false,
        storage: StorageConfig { storage_type: StorageType::Memory, base_path: "".to_string(), max_open_files: 100 },
        compression: CompressionConfig::default(),
        logging: LoggingConfig::default(),
        metrics: MetricsConfig::default(),
        retention: RetentionConfig { enabled: retention_enabled, period_seconds: retention_secs, cleanup_interval_seconds: 300 },
    }
}

fn build_two_level_config(l0_max: usize, l1_max: usize) -> Config {
    Config {
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                TimeLevel {
                    name: "L0".to_string(),
                    duration_seconds: 60,
                    rollup_config: LevelRollupConfig { max_items_per_page: l0_max, max_page_age_seconds: 1000, content_type: RollupContentType::ChildHashes },
                    retention_policy: None,
                },
                TimeLevel {
                    name: "L1".to_string(),
                    duration_seconds: 60,
                    rollup_config: LevelRollupConfig { max_items_per_page: l1_max, max_page_age_seconds: 1000, content_type: RollupContentType::ChildHashes },
                    retention_policy: None,
                },
            ],
        },
        force_rollup_on_shutdown: false,
        storage: StorageConfig { storage_type: StorageType::Memory, base_path: "".to_string(), max_open_files: 100 },
        compression: CompressionConfig::default(),
        logging: LoggingConfig::default(),
        metrics: MetricsConfig::default(),
        retention: RetentionConfig::default(),
        snapshot: Default::default(), // Added missing field
    }
}

fn create_retention_test_page(level: u8, page_id_offset: u64, window_start: DateTime<Utc>, config: &Config) -> JournalPage {
    let gen = PageIdGenerator::new();
    for _ in 0..page_id_offset { gen.next(); }
    JournalPage::new_with_id(gen.next(), level, None, window_start, config)
}

#[tokio::test]
async fn test_page_assignment_new_page_when_full() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(build_single_level_config(2, None, false, 0));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let base = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 30).unwrap();
    let leaf1 = JournalLeaf::new(base, None, "c".into(), json!({"id":1})).unwrap();
    manager.add_leaf(&leaf1, leaf1.timestamp).await.unwrap();
    assert_eq!(manager.get_current_active_page_id(0).await, Some(0));
    assert_eq!(manager.active_pages.lock().await.get(&0).unwrap().content_len(), 1);

    let leaf2 = JournalLeaf::new(base + Duration::seconds(10), None, "c".into(), json!({"id":2})).unwrap();
    manager.add_leaf(&leaf2, leaf2.timestamp).await.unwrap();
    assert!(storage.load_page(0, 0).await.unwrap().is_some());
    assert!(manager.active_pages.lock().await.get(&0).is_none());
    let stored = storage.load_page(0, 0).await.unwrap().unwrap();
    assert_eq!(stored.content_len(), 2);

    let leaf3 = JournalLeaf::new(base + Duration::seconds(20), None, "c".into(), json!({"id":3})).unwrap();
    manager.add_leaf(&leaf3, leaf3.timestamp).await.unwrap();
    let new_id = manager.get_current_active_page_id(0).await.unwrap();
    assert!(new_id > 0);
    assert_eq!(manager.active_pages.lock().await.get(&0).unwrap().content_len(), 1);
    assert!(storage.load_page(0, new_id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_new_page_on_time_boundary() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    // Allow multiple leaves per page so boundary alone triggers a new page
    let config = Arc::new(build_single_level_config(10, None, false, 0));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    // Choose a fixed base time not aligned to a minute boundary
    let base = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 59).unwrap();
    let leaf1 = JournalLeaf::new(base, None, "c".into(), json!({"id": 1})).unwrap();
    manager.add_leaf(&leaf1, leaf1.timestamp).await.unwrap();

    let leaf2_ts = base + Duration::seconds(1); // exactly at 00:01:00 boundary
    let leaf2 = JournalLeaf::new(leaf2_ts, None, "c".into(), json!({"id": 2})).unwrap();
    manager.add_leaf(&leaf2, leaf2.timestamp).await.unwrap();

    // First page should be finalized and stored with only leaf1
    let page0 = storage.load_page(0, 0).await.unwrap().unwrap();
    assert_eq!(page0.content_len(), 1);
    match &page0.content {
        PageContent::Leaves(v) => assert_eq!(v[0].leaf_hash, leaf1.leaf_hash),
        _ => panic!("expected leaves"),
    }

    // Second leaf should be in a new active page
    assert_eq!(manager.get_current_active_page_id(0).await.unwrap(), 1);
    let active = manager.active_pages.lock().await.get(&0).unwrap().clone();
    assert_eq!(active.content_len(), 1);
    match &active.content {
        PageContent::Leaves(v) => assert_eq!(v[0].leaf_hash, leaf2.leaf_hash),
        _ => panic!("expected leaves"),
    }
}

#[tokio::test]
async fn test_rollup_to_parent_on_overflow() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(build_two_level_config(2, 10));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let base = Utc::now();
    let leaf1 = JournalLeaf::new(base, None, "c".into(), json!({"id":1})).unwrap();
    manager.add_leaf(&leaf1, leaf1.timestamp).await.unwrap();
    let leaf2 = JournalLeaf::new(base + Duration::seconds(1), None, "c".into(), json!({"id":2})).unwrap();
    manager.add_leaf(&leaf2, leaf2.timestamp).await.unwrap();

    let stored_l0 = storage.load_page(0, 0).await.unwrap().unwrap();
    assert_eq!(stored_l0.content_len(), 2);
    let active_l1 = manager.active_pages.lock().await.get(&1u8).cloned().expect("L1 page active");
    match &active_l1.content { PageContent::ThrallHashes(h) => assert_eq!(h.len(), 1), _ => panic!("expected thrall hashes") }
    ;

    let leaf3 = JournalLeaf::new(base + Duration::seconds(2), None, "c".into(), json!({"id":3})).unwrap();
    manager.add_leaf(&leaf3, leaf3.timestamp).await.unwrap();
    let leaf4 = JournalLeaf::new(base + Duration::seconds(3), None, "c".into(), json!({"id":4})).unwrap();
    manager.add_leaf(&leaf4, leaf4.timestamp).await.unwrap();

    let stored_l0_p1 = storage.load_page(0, 2).await.unwrap().unwrap();
    assert_eq!(stored_l0_p1.content_len(), 2);
    let active_l1 = manager.active_pages.lock().await.get(&1u8).cloned().expect("L1 page active");
    match &active_l1.content { PageContent::ThrallHashes(h) => assert_eq!(h.len(), 2), _ => panic!("expected thrall hashes") }
}

#[tokio::test]
async fn test_retention_deletes_old_pages_via_summary() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(build_single_level_config(2, None, true, 30));
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

    let now = Utc::now();
    let p1 = create_retention_test_page(0, 0, now - Duration::seconds(120), &config);
    let p2 = create_retention_test_page(0, 1, now - Duration::seconds(90), &config);
    let p3 = create_retention_test_page(0, 2, now - Duration::seconds(70), &config);
    storage.store_page(&p1).await.unwrap();
    storage.store_page(&p2).await.unwrap();
    storage.store_page(&p3).await.unwrap();

    assert_eq!(storage.list_finalized_pages_summary(0).await.unwrap().len(), 3);
    manager.apply_retention_policies().await.unwrap();
    let summaries = storage.list_finalized_pages_summary(0).await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].page_id, p3.page_id);
}

