// src/storage/memory.rs

use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use crate::core::page::{JournalPage, JournalPageSummary};
use crate::core::leaf::JournalLeaf;
use crate::error::CJError;
use crate::storage::StorageBackend;
use std::path::Path;
use log::warn;

/// An in-memory storage backend for journal pages, primarily for testing or ephemeral use.
///
/// Pages are stored in a `DashMap` for thread-safe concurrent access.
/// The key for the map is a tuple `(level: u8, page_id: u64)`.
use std::sync::Mutex;

type FailCondition = Option<(u8, Option<u64>)>;

/// An in-memory storage backend for `JournalPage`s.
///
/// This implementation is primarily used for testing and development purposes.
/// Pages are stored in a `DashMap` keyed by `(level, page_id)` for thread-safe
/// concurrent access.
///
/// It also includes a mechanism to simulate storage failures for testing
/// error handling paths, configurable via `set_fail_on_store`.
#[derive(Debug, Clone)]
pub struct MemoryStorage {
    pages: Arc<DashMap<(u8, u64), JournalPage>>,
    fail_on_store_for: Arc<Mutex<FailCondition>>,
}

impl MemoryStorage {
    /// Creates a new, empty `MemoryStorage` instance.
    pub fn new() -> Self {
        Self {
            pages: Arc::new(DashMap::new()),
            fail_on_store_for: Arc::new(Mutex::new(None as FailCondition)),
        }
    }

    /// Configures this `MemoryStorage` instance to simulate a failure
    /// when `store_page` is called for the specified `level` and `page_id`.
    pub fn set_fail_on_store(&self, level: u8, page_id: Option<u64>) {
        let mut fail_guard = self.fail_on_store_for.lock().unwrap();
        *fail_guard = Some((level, page_id)) as FailCondition;
    }

    /// Clears any previously set failure condition for `store_page`.
    pub fn clear_fail_on_store(&self) {
        let mut guard = self.fail_on_store_for.lock().unwrap();
        *guard = None as FailCondition;
    }

    /// Checks if the storage contains no pages.
    ///
    /// # Returns
    /// `true` if there are no pages stored, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Removes all pages from the storage.
    pub fn clear(&self) {
        self.pages.clear();
    }

    /// Helper to create a boxed version of `MemoryStorage`.
    /// This is useful when a `Box<dyn StorageBackend>` is needed.
    pub fn boxed(self) -> Box<dyn StorageBackend> {
        Box::new(self)
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StorageBackend for MemoryStorage {
    async fn store_page(&self, page: &JournalPage) -> Result<(), CJError> {
        if let Some((fail_level, maybe_fail_page_id)) = *self.fail_on_store_for.lock().unwrap() {
            if page.level == fail_level {
                match maybe_fail_page_id {
                    Some(fail_page_id) => {
                        if page.page_id == fail_page_id {
                            return Err(CJError::StorageError(
                                format!("Simulated MemoryStorage write failure for page L{}/P{}", page.level, page.page_id)
                            ));
                        }
                    }
                    None => {
                        return Err(CJError::StorageError(
                            format!("Simulated MemoryStorage write failure for any page on L{}", page.level)
                        ));
                    }
                }
            }
        }
        self.pages.insert((page.level, page.page_id), page.clone());
        Ok(())
    }

    async fn load_page(&self, level: u8, page_id: u64) -> Result<Option<JournalPage>, CJError> {
        Ok(self.pages.get(&(level, page_id)).map(|entry| entry.value().clone()))
    }

    async fn page_exists(&self, level: u8, page_id: u64) -> Result<bool, CJError> {
        Ok(self.pages.contains_key(&(level, page_id)))
    }

    async fn delete_page(&self, level: u8, page_id: u64) -> Result<(), CJError> {
        self.pages.remove(&(level, page_id));
        Ok(())
    }

    async fn list_finalized_pages_summary(&self, level: u8) -> Result<Vec<JournalPageSummary>, CJError> {
        let mut summaries = Vec::new();
        for entry in self.pages.iter() {
            if entry.key().0 == level {
                summaries.push(JournalPageSummary {
                    page_id: entry.value().page_id,
                    level: entry.value().level,
                    creation_timestamp: entry.value().creation_timestamp,
                    end_time: entry.value().end_time,
                    page_hash: entry.value().page_hash,
                });
            }
        }
        summaries.sort_by_key(|s| s.page_id);
        Ok(summaries)
    }

    async fn backup_journal(&self, backup_path: &Path) -> Result<(), CJError> {
        warn!(
            "Backup operation is a no-op for MemoryStorage. Data not persisted to {:?}.",
            backup_path
        );
        Ok(())
    }

    async fn restore_journal(&self, backup_path: &Path, target_journal_dir: &Path) -> Result<(), CJError> {
        warn!(
            "`restore_journal` called on MemoryStorage with backup_path '{}' and target_journal_dir '{}'. This operation is not supported by MemoryStorage.",
            backup_path.display(),
            target_journal_dir.display()
        );
        Err(CJError::not_supported("restore_journal is not implemented for MemoryStorage".to_string()))
    }

    async fn load_page_by_hash(&self, page_hash: [u8; 32]) -> Result<Option<JournalPage>, CJError> {
        for entry in self.pages.iter() {
            if entry.value().page_hash == page_hash {
                return Ok(Some(entry.value().clone()));
            }
        }
        Ok(None)
    }

    async fn load_leaf_by_hash(&self, leaf_hash: &[u8; 32]) -> Result<Option<JournalLeaf>, CJError> {
        for entry in self.pages.iter() {
            let page = entry.value();
            if page.level == 0 { // Only check L0 pages for leaves
                if let crate::core::page::PageContent::Leaves(leaves) = &page.content {
                    for leaf_in_page in leaves {
                        if leaf_in_page.leaf_hash == *leaf_hash {
                            return Ok(Some(leaf_in_page.clone()));
                        }
                    }
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::page::JournalPage; // For creating dummy pages
    use chrono::{DateTime, Utc};
    use crate::core::page::PageIdGenerator;
    use crate::config::{Config, StorageConfig, CompressionConfig, LoggingConfig, MetricsConfig, RetentionConfig};
    use crate::{core::leaf::JournalLeaf, StorageType, core::page::PageContent};
    use crate::types::time::{TimeHierarchyConfig, TimeLevel, LevelRollupConfig, RollupContentType};
    use crate::core::leaf::{LeafData, LeafDataV1}; // Added missing imports

    fn get_test_config() -> Config {
        Config {
            time_hierarchy: TimeHierarchyConfig {
                levels: vec![
                    TimeLevel {
                            rollup_config: LevelRollupConfig::default(),
                            retention_policy: None, name: "second".to_string(), duration_seconds: 1 },
                    TimeLevel {
                            rollup_config: LevelRollupConfig::default(),
                            retention_policy: None, name: "minute".to_string(), duration_seconds: 60 },
                ]
            },
            force_rollup_on_shutdown: false,
            storage: StorageConfig { storage_type: StorageType::Memory, base_path: "./cjtmp_mem_test".to_string(), max_open_files: 100 },
            compression: CompressionConfig::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
            retention: RetentionConfig::default(),
        }
    }

    // Helper to create a JournalPage for testing
    // Note: page_id is internally managed by JournalPage's atomic counter now.
    // We pass creation_timestamp and prev_page_hash for control.
    fn create_test_page(level: u8, creation_timestamp: DateTime<Utc>, prev_hash: Option<[u8; 32]>, config: &Config) -> JournalPage {
        let page = JournalPage::new(level, prev_hash, creation_timestamp, config);
        // Add a dummy hash to make merkle_root and page_hash non-default for some tests if needed
        // For basic store/load, an empty page is fine. If tests rely on specific hashes, they might need adjustment.
        // page.add_content_hash(PageContentHash::LeafHash([0u8; 32]), creation_timestamp);
        page
    }

    #[tokio::test]
    async fn test_new_memory_storage() {
        let storage = MemoryStorage::new();
        assert!(storage.is_empty());
    }

    #[tokio::test]
    async fn test_store_and_load_page() {
                let storage = MemoryStorage::new();
        let now = Utc::now();
        let config = get_test_config();
        let page_to_store = create_test_page(0, now, None, &config);

        let store_result = storage.store_page(&page_to_store).await;
        assert!(store_result.is_ok());
        assert!(!storage.is_empty());

        let loaded_page_opt = storage.load_page(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(loaded_page_opt.is_some());
        let loaded_page = loaded_page_opt.unwrap();
        assert_eq!(loaded_page.page_id, page_to_store.page_id);
        assert_eq!(loaded_page.level, page_to_store.level);
        assert_eq!(loaded_page.merkle_root, page_to_store.merkle_root);
    }

    #[tokio::test]
    async fn test_load_non_existent_page() {
        let storage = MemoryStorage::new();
        let loaded_page_opt = storage.load_page(0, 99).await.unwrap();
        assert!(loaded_page_opt.is_none());
    }

    #[tokio::test]
    async fn test_page_exists() {
                let storage = MemoryStorage::new();
        let now = Utc::now();
        let config = get_test_config();
        let page_to_store = create_test_page(1, now, None, &config);

        let exists_before_store = storage.page_exists(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(!exists_before_store);

        storage.store_page(&page_to_store).await.unwrap();

        let exists_after_store = storage.page_exists(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(exists_after_store);
    }

    #[tokio::test]
    async fn test_clear_storage() {
                let config = get_test_config();
        let storage = MemoryStorage::new();
        let now = Utc::now();
        let page1 = create_test_page(0, now, None, &config);
        let page2 = create_test_page(0, now + chrono::Duration::seconds(10), None, &config);

        storage.store_page(&page1).await.unwrap();
        storage.store_page(&page2).await.unwrap();

        assert!(!storage.is_empty());
        assert!(storage.page_exists(page1.level, page1.page_id).await.unwrap());
        assert!(storage.page_exists(page2.level, page2.page_id).await.unwrap());

        storage.clear();

        assert!(storage.is_empty());
        assert!(!storage.page_exists(page1.level, page1.page_id).await.unwrap());
        assert!(!storage.page_exists(page2.level, page2.page_id).await.unwrap());
    }

    #[tokio::test]
    async fn test_default_constructor() {
        let storage: MemoryStorage = Default::default();
        assert!(storage.is_empty());
    }

    #[tokio::test]
    async fn test_boxed_constructor() {
                let config = get_test_config();
        let now = Utc::now();
        
        let boxed_storage: Box<dyn StorageBackend> = MemoryStorage::new().boxed();
        let page_to_store = create_test_page(0, now, None, &config);

        boxed_storage.store_page(&page_to_store).await.unwrap();
        
        let loaded_page_opt = boxed_storage.load_page(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(loaded_page_opt.is_some());
        let loaded_page = loaded_page_opt.unwrap();
        assert_eq!(loaded_page.page_id, page_to_store.page_id);
    }

    #[tokio::test]
    async fn test_load_leaf_by_hash() {
                let storage = MemoryStorage::new();
        let config = get_test_config();
        let now = Utc::now();

        // Create some leaves
        let leaf_data_1 = LeafDataV1 { timestamp: now, content_type: "text/plain".to_string(), content: b"delta1".to_vec(), author: "mem_test".to_string(), signature: "sig_mem1".to_string() };
        let leaf1 = JournalLeaf::new(now, None, "container1".to_string(), serde_json::to_value(LeafData::V1(leaf_data_1)).expect("Serialize LdV1 L1")).expect("Create L1");
        let leaf_data_2 = LeafDataV1 { timestamp: now + chrono::Duration::seconds(1), content_type: "text/plain".to_string(), content: b"delta2".to_vec(), author: "mem_test".to_string(), signature: "sig_mem2".to_string() };
        let leaf2 = JournalLeaf::new(now + chrono::Duration::seconds(1), Some(leaf1.leaf_hash), "container1".to_string(), serde_json::to_value(LeafData::V1(leaf_data_2)).expect("Serialize LdV1 L2")).expect("Create L2");
        let leaf_data_3 = LeafDataV1 { timestamp: now + chrono::Duration::seconds(2), content_type: "text/plain".to_string(), content: b"delta3".to_vec(), author: "mem_test".to_string(), signature: "sig_mem3".to_string() };
        let leaf3 = JournalLeaf::new(now + chrono::Duration::seconds(2), None, "container2".to_string(), serde_json::to_value(LeafData::V1(leaf_data_3)).expect("Serialize LdV1 L3")).expect("Create L3");

        // Create L0 page 1 and add leaves 1 and 2
        let mut page1_l0 = JournalPage::new(0, None, now, &config);
        page1_l0.add_leaf(leaf1.clone());
        page1_l0.add_leaf(leaf2.clone());
        page1_l0.recalculate_merkle_root_and_page_hash();
        storage.store_page(&page1_l0).await.unwrap();

        // Create L0 page 2 and add leaf 3
        let mut page2_l0 = JournalPage::new(0, Some(page1_l0.page_hash), now + chrono::Duration::seconds(5), &config);
        page2_l0.add_leaf(leaf3.clone());
        page2_l0.recalculate_merkle_root_and_page_hash();
        storage.store_page(&page2_l0).await.unwrap();

        // Create L1 page (should not be searched for leaves)
        let mut page1_l1 = JournalPage::new(1, None, now, &config);
        // JournalPage::new for L1 creates PageContent::ThrallHashes. Add a hash to it.
        if let PageContent::ThrallHashes(ref mut hashes) = page1_l1.content {
            hashes.push([8u8; 32]);
        } else {
            panic!("L1 page content is not ThrallHashes as expected");
        }
        page1_l1.recalculate_merkle_root_and_page_hash();
        storage.store_page(&page1_l1).await.unwrap();

        // Test loading existing leaves
        let loaded_leaf1_opt = storage.load_leaf_by_hash(&leaf1.leaf_hash).await.unwrap();
        assert!(loaded_leaf1_opt.is_some(), "Leaf1 should be found");
        let loaded_leaf1 = loaded_leaf1_opt.unwrap();
        assert_eq!(loaded_leaf1.leaf_hash, leaf1.leaf_hash);
        assert_eq!(loaded_leaf1.delta_payload, leaf1.delta_payload);

        let loaded_leaf3_opt = storage.load_leaf_by_hash(&leaf3.leaf_hash).await.unwrap();
        assert!(loaded_leaf3_opt.is_some(), "Leaf3 should be found");
        let loaded_leaf3 = loaded_leaf3_opt.unwrap();
        assert_eq!(loaded_leaf3.leaf_hash, leaf3.leaf_hash);
        assert_eq!(loaded_leaf3.delta_payload, leaf3.delta_payload);

        // Test loading a non-existent leaf
        let non_existent_hash = [99u8; 32];
        let not_found_leaf_opt = storage.load_leaf_by_hash(&non_existent_hash).await.unwrap();
        assert!(not_found_leaf_opt.is_none(), "Non-existent leaf should not be found");
    }
}
