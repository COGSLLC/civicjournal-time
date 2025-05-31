// src/storage/file.rs

use std::path::{Path, PathBuf};
use async_trait::async_trait;
use tokio::fs;

use crate::core::page::{JournalPage, JournalPageSummary};
use crate::error::CJError;
use crate::storage::StorageBackend;

const MARKER_FILE_NAME: &str = ".civicjournal-time";
const JOURNAL_SUBDIR: &str = "journal";

/// A storage backend that persists journal pages to the file system.
///
/// Pages are stored in a structured directory format:
/// `base_path/journal/level_<L>/page_<ID>.json`
#[derive(Debug)]
pub struct FileStorage {
    base_path: PathBuf,
}

impl FileStorage {
    /// Creates a new `FileStorage` instance.
    ///
    /// This will create the base directory and a marker file (`.civicjournal-time`)
    /// if they don't already exist.
    ///
    /// # Arguments
    ///
    /// * `base_path` - The root directory where journal data will be stored.
    pub async fn new<P: AsRef<Path>>(base_path: P) -> Result<Self, CJError> {
        let path = base_path.as_ref().to_path_buf();

        // Create the base path if it doesn't exist
        fs::create_dir_all(&path).await.map_err(|e| CJError::StorageError(format!("Failed to create base path '{}': {}", path.display(), e)))?;

        // Create the marker file
        let marker_path = path.join(MARKER_FILE_NAME);
        if !fs::try_exists(&marker_path).await.map_err(|e| CJError::StorageError(format!("Failed to check marker file existence '{}': {}", marker_path.display(), e)))? {
            fs::File::create(&marker_path).await.map_err(|e| CJError::StorageError(format!("Failed to create marker file '{}': {}", marker_path.display(), e)))?;
        }

        Ok(Self { base_path: path })
    }

    /// Constructs the path for a specific journal page file.
    /// Path: `base_path/journal/level_<L>/page_<ID>.json`
    /// Helper to create a boxed version of `FileStorage`.
    /// This is useful when a `Box<dyn StorageBackend>` is needed.
    pub fn boxed(self) -> Box<dyn StorageBackend> {
        Box::new(self)
    }

    /// Constructs the path for a specific journal page file.
    /// Path: `base_path/journal/level_<L>/page_<ID>.json`
    fn get_page_path(&self, level: u8, page_id: u64) -> PathBuf {
        self.base_path
            .join(JOURNAL_SUBDIR)
            .join(format!("level_{}", level))
            .join(format!("page_{}.json", page_id))
    }
}

#[async_trait]
impl StorageBackend for FileStorage {
    async fn store_page(&self, page: &JournalPage) -> Result<(), CJError> {
        let page_path = self.get_page_path(page.level, page.page_id);

        if let Some(parent_dir) = page_path.parent() {
            fs::create_dir_all(parent_dir).await.map_err(|e| CJError::StorageError(format!("Failed to create parent directory for page '{}': {}", page_path.display(), e)))?;
        }

        let page_json = serde_json::to_string_pretty(page)
            .map_err(CJError::from)?;

        fs::write(&page_path, page_json).await.map_err(|e| CJError::StorageError(format!("Failed to write page '{}': {}", page_path.display(), e)))?;

        Ok(())
    }

    async fn load_page(&self, level: u8, page_id: u64) -> Result<Option<JournalPage>, CJError> {
        let page_path = self.get_page_path(level, page_id);

        if !fs::try_exists(&page_path).await.map_err(|e| CJError::StorageError(format!("Failed to check page existence '{}': {}", page_path.display(), e)))? {
            return Ok(None);
        }

        let page_json = fs::read_to_string(&page_path).await.map_err(|e| CJError::StorageError(format!("Failed to read page '{}': {}", page_path.display(), e)))?;
        
        let page: JournalPage = serde_json::from_str(&page_json)
            .map_err(CJError::from)?;

        Ok(Some(page))
    }

    async fn page_exists(&self, level: u8, page_id: u64) -> Result<bool, CJError> {
        let page_path = self.get_page_path(level, page_id);
        fs::try_exists(&page_path).await.map_err(|e| CJError::StorageError(format!("Failed to check page existence '{}': {}", page_path.display(), e)))
    }

    async fn delete_page(&self, level: u8, page_id: u64) -> Result<(), CJError> {
        let path = self.get_page_path(level, page_id);
        if fs::try_exists(&path).await.map_err(|e| CJError::StorageError(format!("Failed to check page existence for deletion '{}': {}", path.display(), e)))? {
            tokio::fs::remove_file(path).await.map_err(|e| {
                CJError::StorageError(format!("Failed to delete page file L{}/{}: {}", level, page_id, e))
            })?
        }
        Ok(())
    }

    async fn list_finalized_pages_summary(&self, level: u8) -> Result<Vec<JournalPageSummary>, CJError> {
        let level_path = self.base_path.join(JOURNAL_SUBDIR).join(format!("level_{}", level));
        if !fs::try_exists(&level_path).await.map_err(|e| CJError::StorageError(format!("Failed to check level directory existence '{}': {}", level_path.display(), e)))? {
            return Ok(Vec::new()); // No directory for this level, so no pages
        }

        let mut summaries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(level_path).await.map_err(|e| {
            CJError::StorageError(format!("Failed to read level directory L{}: {}", level, e))
        })?;

        while let Some(entry) = read_dir.next_entry().await.map_err(|e| {
            CJError::StorageError(format!("Failed to read directory entry in L{}: {}", level, e))
        })? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem.starts_with("page_") {
                        if let Ok(page_id) = stem.trim_start_matches("page_").parse::<u64>() {
                            match self.load_page(level, page_id).await {
                                Ok(Some(page)) => {
                                    summaries.push(JournalPageSummary {
                                        page_id: page.page_id,
                                        level: page.level,
                                        creation_timestamp: page.creation_timestamp,
                                        end_time: page.end_time,
                                        page_hash: page.page_hash,
                                    });
                                }
                                Ok(None) => {
                                    eprintln!("Warning: Found page file {} but failed to load it as JournalPage for summary.", path.display());
                                }
                                Err(e) => {
                                    eprintln!("Warning: Error loading page {} for summary: {}", path.display(), e);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    use chrono::{DateTime, Utc};
    use tempfile::tempdir;
    use crate::core::page::{JournalPage, PageContentHash};
    use crate::core::{SHARED_TEST_ID_MUTEX, reset_global_ids}; // Import shared test utilities
    
    use crate::config::{Config, StorageConfig, CompressionConfig, LoggingConfig, MetricsConfig, RetentionConfig};
    use crate::types::time::{RollupConfig, TimeLevel as TypeTimeLevel};
    use crate::{StorageType, TimeHierarchyConfig};

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
            // For file storage tests, we might still use Memory for config simplicity unless test needs file specifics
            storage: StorageConfig { storage_type: StorageType::Memory, base_path: "./cjtmp_file_test".to_string(), max_open_files: 100 }, 
            compression: CompressionConfig::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
            retention: RetentionConfig::default(),
        }
    }

    fn create_test_page(level: u8, creation_timestamp: DateTime<Utc>, prev_hash: Option<PageContentHash>, config: &Config) -> JournalPage {
        let mut page = JournalPage::new(level, prev_hash, creation_timestamp, creation_timestamp, config);
        // Add some content to make hashes non-default for tests that might check them.
        let content_hashes_data: Vec<[u8;32]> = vec![[0u8;32],[1u8;32]];
        for (i, ch_data) in content_hashes_data.iter().enumerate() {
            page.add_content_hash(PageContentHash::LeafHash(*ch_data), creation_timestamp + chrono::Duration::nanoseconds(i as i64 + 1));
        }
        page
    }

    #[tokio::test]
    async fn test_new_file_storage() {
        let dir = tempdir().unwrap();
        let storage = FileStorage::new(dir.path()).await;
        assert!(storage.is_ok());
        assert!(dir.path().join(MARKER_FILE_NAME).exists());
    }

    #[tokio::test]
    async fn test_store_and_load_page() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();
        let dir = tempdir().unwrap();
        let storage = FileStorage::new(dir.path()).await.unwrap();
        let config = get_test_config();
        let now = Utc::now();
        let page_to_store = create_test_page(0, now, None, &config);

        let store_result = storage.store_page(&page_to_store).await;
        assert!(store_result.is_ok());

        let loaded_page_opt = storage.load_page(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(loaded_page_opt.is_some());
        let loaded_page = loaded_page_opt.unwrap();
        assert_eq!(loaded_page.page_id, page_to_store.page_id);
        assert_eq!(loaded_page.level, page_to_store.level);
        assert_eq!(loaded_page.merkle_root, page_to_store.merkle_root);
    }

    #[tokio::test]
    async fn test_load_non_existent_page() {
        let dir = tempdir().unwrap();
        let storage = FileStorage::new(dir.path()).await.unwrap();
        let loaded_page_opt = storage.load_page(0, 99).await.unwrap();
        assert!(loaded_page_opt.is_none());
    }

    #[tokio::test]
    async fn test_page_exists() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();
        let dir = tempdir().unwrap();
        let storage = FileStorage::new(dir.path()).await.unwrap();
        let config = get_test_config();
        let now = Utc::now();
        let page_to_store = create_test_page(1, now, None, &config);

        let exists_before_store = storage.page_exists(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(!exists_before_store);

        storage.store_page(&page_to_store).await.unwrap();

        let exists_after_store = storage.page_exists(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(exists_after_store);
    }
}
