use civicjournal_time::query::QueryEngine;
use civicjournal_time::storage::memory::MemoryStorage;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::page::JournalPage;
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::core::time_manager::TimeHierarchyManager;
use civicjournal_time::config::Config;
use chrono::{Utc, Duration};
use serde_json::json;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};
use std::sync::Arc;

#[tokio::test]
async fn test_reconstruct_and_delta_report() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let storage = Arc::new(MemoryStorage::new());
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let t0 = Utc::now();
    // three leaves for same container
    let l1 = JournalLeaf::new(t0, None, "c1".into(), json!({"a":1})).unwrap();
    let l2 = JournalLeaf::new(t0 + Duration::seconds(1), Some(l1.leaf_hash), "c1".into(), json!({"b":2})).unwrap();
    let l3 = JournalLeaf::new(t0 + Duration::seconds(2), Some(l2.leaf_hash), "c1".into(), json!({"a":3})).unwrap();

    let mut page1 = JournalPage::new(0, None, t0, &config);
    if let civicjournal_time::core::page::PageContent::Leaves(ref mut v) = page1.content { v.push(l1.clone()); v.push(l2.clone()); }
    page1.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page1).await.unwrap();

    let mut page2 = JournalPage::new(0, Some(page1.page_hash), t0 + Duration::seconds(2), &config);
    if let civicjournal_time::core::page::PageContent::Leaves(ref mut v) = page2.content { v.push(l3.clone()); }
    page2.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page2).await.unwrap();

    let state = engine.reconstruct_container_state("c1", t0 + Duration::seconds(1)).await.unwrap();
    assert_eq!(state.state_data["a"], 1);
    assert_eq!(state.state_data["b"], 2);

    let report = engine.get_delta_report("c1", t0, t0 + Duration::seconds(3)).await.unwrap();
    assert_eq!(report.deltas.len(), 3);
}

#[tokio::test]
async fn test_page_chain_integrity() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let storage = Arc::new(MemoryStorage::new());
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let t0 = Utc::now();
    let mut page1 = JournalPage::new(0, None, t0, &config);
    page1.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page1).await.unwrap();

    let mut page2 = JournalPage::new(0, Some(page1.page_hash), t0 + Duration::seconds(1), &config);
    page2.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page2).await.unwrap();

    let reports = engine.get_page_chain_integrity(0, Some(page1.page_id), Some(page2.page_id)).await.unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(|r| r.is_valid));
}
