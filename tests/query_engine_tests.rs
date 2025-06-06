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

#[tokio::test]
async fn test_leaf_inclusion_proof_not_found() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let storage = Arc::new(MemoryStorage::new());
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let fake_hash = [1u8; 32];
    let result = engine.get_leaf_inclusion_proof(&fake_hash).await;
    assert!(matches!(result, Err(civicjournal_time::query::types::QueryError::LeafNotFound(_))));
}

#[tokio::test]
async fn test_delta_report_invalid_params() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let storage = Arc::new(MemoryStorage::new());
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let t0 = Utc::now();
    let result = engine.get_delta_report("c1", t0 + Duration::seconds(1), t0).await;
    assert!(matches!(result, Err(civicjournal_time::query::types::QueryError::InvalidParameters(_))));
}

#[tokio::test]
async fn test_reconstruct_container_state_not_found() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let storage = Arc::new(MemoryStorage::new());
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let t0 = Utc::now();
    let result = engine.reconstruct_container_state("missing", t0).await;
    assert!(matches!(result, Err(civicjournal_time::query::types::QueryError::ContainerNotFound(_))));
}

#[tokio::test]
async fn test_page_chain_integrity_invalid_range() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let storage = Arc::new(MemoryStorage::new());
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let res = engine.get_page_chain_integrity(0, Some(5), Some(3)).await;
    assert!(matches!(res, Err(civicjournal_time::query::types::QueryError::InvalidParameters(_))));
}

#[tokio::test]
async fn test_delta_report_container_not_found() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let storage = Arc::new(MemoryStorage::new());
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let t0 = Utc::now();
    let res = engine.get_delta_report("missing", t0, t0 + Duration::seconds(1)).await;
    assert!(matches!(res, Err(civicjournal_time::query::types::QueryError::ContainerNotFound(_))));
}

#[tokio::test]
async fn test_page_chain_integrity_detects_mismatch() {
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

    let mut page2 = JournalPage::new(0, None, t0 + Duration::seconds(1), &config);
    page2.recalculate_merkle_root_and_page_hash();
    // Intentionally set incorrect prev_page_hash
    page2.prev_page_hash = Some([9u8; 32]);
    storage.store_page(&page2).await.unwrap();

    let reports = engine
        .get_page_chain_integrity(0, Some(page1.page_id), Some(page2.page_id))
        .await
        .unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports[1].issues.iter().any(|i| i.contains("prev_page_hash")));
    assert!(!reports[1].is_valid);
}

#[tokio::test]
async fn test_get_delta_report_partial_range_sorted() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let storage = Arc::new(MemoryStorage::new());
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let t0 = Utc::now();
    let l1 = JournalLeaf::new(t0, None, "c1".into(), json!({"a":1})).unwrap();
    let l2 = JournalLeaf::new(t0 + Duration::seconds(1), Some(l1.leaf_hash), "c1".into(), json!({"b":2})).unwrap();
    let l3 = JournalLeaf::new(t0 + Duration::seconds(2), Some(l2.leaf_hash), "c1".into(), json!({"c":3})).unwrap();

    let mut page1 = JournalPage::new(0, None, t0, &config);
    if let civicjournal_time::core::page::PageContent::Leaves(ref mut v) = page1.content { v.push(l1.clone()); v.push(l2.clone()); }
    page1.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page1).await.unwrap();

    let mut page2 = JournalPage::new(0, Some(page1.page_hash), t0 + Duration::seconds(2), &config);
    if let civicjournal_time::core::page::PageContent::Leaves(ref mut v) = page2.content { v.push(l3.clone()); }
    page2.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page2).await.unwrap();

    let report = engine.get_delta_report("c1", t0 + Duration::seconds(1), t0 + Duration::seconds(2)).await.unwrap();
    assert_eq!(report.deltas.len(), 2);
    assert_eq!(report.deltas[0].leaf_hash, l2.leaf_hash);
    assert_eq!(report.deltas[1].leaf_hash, l3.leaf_hash);
}

#[tokio::test]
async fn test_delta_report_container_not_found_no_leaves_in_range() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let storage = Arc::new(MemoryStorage::new());
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let t0 = Utc::now();
    let l1 = JournalLeaf::new(t0, None, "c1".into(), json!({"a":1})).unwrap();
    let l2 = JournalLeaf::new(t0 + Duration::seconds(1), Some(l1.leaf_hash), "c1".into(), json!({"b":2})).unwrap();

    let mut page1 = JournalPage::new(0, None, t0, &config);
    if let civicjournal_time::core::page::PageContent::Leaves(ref mut v) = page1.content { v.push(l1.clone()); v.push(l2.clone()); }
    page1.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page1).await.unwrap();

    let res = engine.get_delta_report("c1", t0 + Duration::seconds(3), t0 + Duration::seconds(4)).await;
    assert!(matches!(res, Err(civicjournal_time::query::types::QueryError::ContainerNotFound(_))));
}
