use civicjournal_time::query::QueryEngine;
use civicjournal_time::storage::memory::MemoryStorage;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::page::{JournalPage, JournalPageSummary};
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::core::time_manager::TimeHierarchyManager;
use civicjournal_time::config::Config;
use chrono::{Utc, Duration};
use serde_json::json;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};
use std::sync::Arc;
use civicjournal_time::error::CJError;
use async_trait::async_trait;
use std::path::Path;

#[derive(Clone, Debug)]
struct MemoryStorageWithExtraSummary {
    inner: MemoryStorage,
    summary: JournalPageSummary,
}

impl MemoryStorageWithExtraSummary {
    fn new(inner: MemoryStorage, summary: JournalPageSummary) -> Self {
        Self { inner, summary }
    }
}

#[async_trait]
impl StorageBackend for MemoryStorageWithExtraSummary {
    async fn store_page(&self, page: &JournalPage) -> Result<(), CJError> {
        self.inner.store_page(page).await
    }

    async fn load_page(&self, level: u8, page_id: u64) -> Result<Option<JournalPage>, CJError> {
        self.inner.load_page(level, page_id).await
    }

    async fn page_exists(&self, level: u8, page_id: u64) -> Result<bool, CJError> {
        self.inner.page_exists(level, page_id).await
    }

    async fn delete_page(&self, level: u8, page_id: u64) -> Result<(), CJError> {
        self.inner.delete_page(level, page_id).await
    }

    async fn list_finalized_pages_summary(&self, level: u8) -> Result<Vec<JournalPageSummary>, CJError> {
        let mut list = self.inner.list_finalized_pages_summary(level).await?;
        if self.summary.level == level {
            list.push(self.summary.clone());
        }
        list.sort_by_key(|s| s.page_id);
        Ok(list)
    }

    async fn backup_journal(&self, backup_path: &Path) -> Result<(), CJError> {
        self.inner.backup_journal(backup_path).await
    }

    async fn restore_journal(&self, backup_path: &Path, target_journal_dir: &Path) -> Result<(), CJError> {
        self.inner.restore_journal(backup_path, target_journal_dir).await
    }

    async fn load_page_by_hash(&self, page_hash: [u8; 32]) -> Result<Option<JournalPage>, CJError> {
        self.inner.load_page_by_hash(page_hash).await
    }

    async fn load_leaf_by_hash(&self, leaf_hash: &[u8; 32]) -> Result<Option<JournalLeaf>, CJError> {
        self.inner.load_leaf_by_hash(leaf_hash).await
    }
}

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
async fn test_page_chain_integrity_detects_corrupted_page() {
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
    if let civicjournal_time::core::page::PageContent::Leaves(ref mut v) = page2.content {
        v.push(JournalLeaf::new(t0 + Duration::seconds(2), None, "c".into(), json!({"a":1})).unwrap());
    }
    // Intentionally do not recalculate hashes after modifying content
    storage.store_page(&page2).await.unwrap();

    let reports = engine
        .get_page_chain_integrity(0, Some(page1.page_id), Some(page2.page_id))
        .await
        .unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports[0].is_valid);
    assert!(!reports[1].is_valid);
    assert!(reports[1].issues.iter().any(|i| i.contains("merkle_root")));
    assert!(reports[1].issues.iter().any(|i| i.contains("page_hash")));
}

#[tokio::test]
async fn test_page_chain_integrity_page_missing() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let config = Arc::new(Config::default());
    let base_storage = MemoryStorage::new();
    let t0 = Utc::now();
    let mut page1 = JournalPage::new(0, None, t0, &config);
    page1.recalculate_merkle_root_and_page_hash();
    base_storage.store_page(&page1).await.unwrap();

    let missing_summary = JournalPageSummary {
        page_id: 1,
        level: 0,
        creation_timestamp: t0 + Duration::seconds(1),
        end_time: t0 + Duration::seconds(2),
        page_hash: [0u8; 32],
    };

    let storage = Arc::new(MemoryStorageWithExtraSummary::new(base_storage, missing_summary));
    let tm = Arc::new(TimeHierarchyManager::new(config.clone(), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), tm, config.clone());

    let reports = engine.get_page_chain_integrity(0, None, None).await.unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports[0].is_valid);
    assert!(!reports[1].is_valid);
    assert!(reports[1].issues.iter().any(|i| i.contains("page missing")));
}

