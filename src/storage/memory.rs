// src/storage/memory.rs

use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use crate::core::page::{JournalPage, JournalPageSummary};
use crate::error::CJError;
use crate::storage::StorageBackend;

/// An in-memory storage backend for journal pages, primarily for testing or ephemeral use.
///
/// Pages are stored in a `DashMap` for thread-safe concurrent access.
/// The key for the map is a tuple `(level: u8, page_id: u64)`.
#[derive(Debug, Clone)]
pub struct MemoryStorage {
    pages: Arc<DashMap<(u8, u64), JournalPage>>,
}

impl MemoryStorage {
    /// Creates a new, empty `MemoryStorage` instance.
    pub fn new() -> Self {
        Self {
            pages: Arc::new(DashMap::new()),
        }
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
        let summaries = self.pages
            .iter()
            .filter(|entry| entry.key().0 == level)
            .map(|entry| {
                let page = entry.value();
                JournalPageSummary {
                    page_id: page.page_id,
                    level: page.level,
                    creation_timestamp: page.creation_timestamp,
                    end_time: page.end_time,
                    page_hash: page.page_hash,
                }
            })
            .collect();
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::page::{PageContentHash, JournalPage}; // For creating dummy pages
    use chrono::{DateTime, Utc};
    // Removed Ordering as reset_global_ids handles it
    use crate::core::{SHARED_TEST_ID_MUTEX, reset_global_ids}; // Import shared test utilities
    use crate::config::{Config, StorageConfig, CompressionConfig, LoggingConfig, MetricsConfig, RetentionConfig};
    use crate::{StorageType};
    use crate::types::time::{TimeHierarchyConfig, TimeLevel as TypeTimeLevel, RollupConfig};

    fn get_test_config() -> Config {
        Config {
            time_hierarchy: TimeHierarchyConfig {
                levels: vec![
                    TypeTimeLevel {
                            rollup_config: RollupConfig::default(),
                            retention_policy: None, name: "second".to_string(), duration_seconds: 1 },
                    TypeTimeLevel {
                            rollup_config: RollupConfig::default(),
                            retention_policy: None, name: "minute".to_string(), duration_seconds: 60 },
                ]
            },
            rollup: Default::default(),
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
    fn create_test_page(level: u8, creation_timestamp: DateTime<Utc>, prev_hash: Option<PageContentHash>, config: &Config) -> JournalPage {
        let page = JournalPage::new(level, prev_hash, creation_timestamp, creation_timestamp, config);
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
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();
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
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();
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
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();
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
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();
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
}
