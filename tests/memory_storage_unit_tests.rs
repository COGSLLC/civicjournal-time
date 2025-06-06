use civicjournal_time::storage::memory::MemoryStorage;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::page::{JournalPage, PageContent};
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::error::CJError;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids, get_test_config};
use chrono::Utc;

use civicjournal_time::config::{TimeLevel, LevelRollupConfig};

fn two_level_config() -> civicjournal_time::config::Config {
    let mut cfg = get_test_config().clone();
    cfg.time_hierarchy.levels.push(TimeLevel {
        name: "L1_test".to_string(),
        duration_seconds: 300,
        rollup_config: LevelRollupConfig::default(),
        retention_policy: None,
    });
    cfg
}

#[tokio::test]
async fn test_new_starts_empty() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    assert!(storage.is_empty());
}

#[tokio::test]
async fn test_store_and_load_multiple_pages() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let now = Utc::now();
    let page1 = JournalPage::new(0, None, now, cfg);
    let page2 = JournalPage::new(0, None, now + chrono::Duration::seconds(1), cfg);
    storage.store_page(&page1).await.unwrap();
    storage.store_page(&page2).await.unwrap();
    let loaded1 = storage.load_page(0, page1.page_id).await.unwrap().unwrap();
    let loaded2 = storage.load_page(0, page2.page_id).await.unwrap().unwrap();
    assert_eq!(loaded1, page1);
    assert_eq!(loaded2, page2);
}

#[tokio::test]
async fn test_page_exists_before_and_after_store() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let page = JournalPage::new(0, None, Utc::now(), cfg);
    assert!(!storage.page_exists(0, page.page_id).await.unwrap());
    storage.store_page(&page).await.unwrap();
    assert!(storage.page_exists(0, page.page_id).await.unwrap());
}

#[tokio::test]
async fn test_clear_removes_all_pages() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let now = Utc::now();
    let page1 = JournalPage::new(0, None, now, cfg);
    let page2 = JournalPage::new(0, None, now + chrono::Duration::seconds(1), cfg);
    storage.store_page(&page1).await.unwrap();
    storage.store_page(&page2).await.unwrap();
    assert!(!storage.is_empty());
    storage.clear();
    assert!(storage.is_empty());
    assert!(!storage.page_exists(0, page1.page_id).await.unwrap());
    assert!(!storage.page_exists(0, page2.page_id).await.unwrap());
}

#[tokio::test]
async fn test_fail_on_store_and_recover() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let page = JournalPage::new(0, None, Utc::now(), cfg);
    storage.set_fail_on_store(0, None);
    let err = storage.store_page(&page).await.unwrap_err();
    if let CJError::StorageError(msg) = err {
        assert!(msg.contains("Simulated MemoryStorage write failure"));
    } else {
        panic!("Unexpected error type");
    }
    storage.clear_fail_on_store();
    storage.store_page(&page).await.unwrap();
}

#[tokio::test]
async fn test_list_finalized_pages_summary() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = two_level_config();
    let now = Utc::now();
    let page1 = JournalPage::new(0, None, now, &cfg);
    let page2 = JournalPage::new(0, None, now + chrono::Duration::seconds(1), &cfg);
    let mut page3 = JournalPage::new(1, None, now, &cfg);
    if let PageContent::ThrallHashes(ref mut v) = page3.content { v.push([1u8;32]); }
    page3.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page1).await.unwrap();
    storage.store_page(&page2).await.unwrap();
    storage.store_page(&page3).await.unwrap();
    let l0 = storage.list_finalized_pages_summary(0).await.unwrap();
    assert_eq!(l0.len(), 2);
    for s in &l0 { assert_eq!(s.level, 0); }
    let l1 = storage.list_finalized_pages_summary(1).await.unwrap();
    assert_eq!(l1.len(), 1);
    assert_eq!(l1[0].page_id, page3.page_id);
}

#[tokio::test]
async fn test_backup_noop_and_restore_error() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let page = JournalPage::new(0, None, Utc::now(), cfg);
    storage.store_page(&page).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let backup_path = dir.path().join("backup.zip");
    storage.backup_journal(&backup_path).await.unwrap();
    assert!(storage.page_exists(0, page.page_id).await.unwrap());
    let res = storage.restore_journal(&backup_path, dir.path()).await;
    assert!(matches!(res.unwrap_err(), CJError::NotSupported(_)));
    assert!(storage.page_exists(0, page.page_id).await.unwrap());
}

#[tokio::test]
async fn test_load_page_by_hash() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let mut page = JournalPage::new(0, None, Utc::now(), cfg);
    page.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page).await.unwrap();
    let loaded = storage.load_page_by_hash(page.page_hash).await.unwrap().unwrap();
    assert_eq!(loaded.page_id, page.page_id);
}

#[tokio::test]
async fn test_load_leaf_by_hash_behavior() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = two_level_config();
    let now = Utc::now();
    let leaf1 = JournalLeaf::new(now, None, "c".into(), serde_json::json!({"a":1})).unwrap();
    let leaf2 = JournalLeaf::new(now + chrono::Duration::seconds(1), Some(leaf1.leaf_hash), "c".into(), serde_json::json!({"b":2})).unwrap();
    let mut l0 = JournalPage::new(0, None, now, &cfg);
    if let PageContent::Leaves(ref mut v) = l0.content { v.push(leaf1.clone()); v.push(leaf2.clone()); }
    l0.recalculate_merkle_root_and_page_hash();
    storage.store_page(&l0).await.unwrap();
    let mut l1 = JournalPage::new(1, None, now, &cfg);
    if let PageContent::ThrallHashes(ref mut v) = l1.content { v.push([9u8;32]); }
    l1.recalculate_merkle_root_and_page_hash();
    storage.store_page(&l1).await.unwrap();
    for lf in [&leaf1, &leaf2] {
        let found = storage.load_leaf_by_hash(&lf.leaf_hash).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().leaf_hash, lf.leaf_hash);
    }
    let thrall_hash = [9u8;32];
    let none_leaf = storage.load_leaf_by_hash(&thrall_hash).await.unwrap();
    assert!(none_leaf.is_none());
    let missing_hash = [42u8;32];
    assert!(storage.load_leaf_by_hash(&missing_hash).await.unwrap().is_none());
}

