// src/storage/mod.rs

/// In-memory storage backend for journal data (primarily for testing or ephemeral use).
pub mod memory;
/// File-based storage backend for persisting journal data to disk.
pub mod file;

// Define a common storage trait

use crate::config::Config;
use crate::StorageType;
use crate::core::page::{JournalPage, JournalPageSummary};
use crate::core::leaf::JournalLeaf;
use crate::error::CJError;
use crate::CJResult;
use async_trait::async_trait;
use std::path::Path;

pub use memory::MemoryStorage;
pub use file::FileStorage;

/// Defines the interface for storage backends capable of persisting and retrieving journal pages.
///
/// This trait is designed to be implemented by various storage mechanisms, such as file-based storage
/// or in-memory storage, allowing for flexibility in how journal data is managed.
#[async_trait]
pub trait StorageBackend: Send + Sync + std::fmt::Debug + 'static {
    /// Stores a journal page in the backend.
    ///
    /// # Arguments
    /// * `page` - A reference to the `JournalPage` to be stored.
    ///
    /// # Errors
    /// Returns a `CJError` if the page cannot be stored.
    async fn store_page(&self, page: &JournalPage) -> Result<(), CJError>;

    /// Loads a specific journal page from the backend.
    ///
    /// # Arguments
    /// * `level` - The time hierarchy level of the page.
    /// * `page_id` - The unique identifier of the page.
    ///
    /// # Returns
    /// An `Option<JournalPage>` which is `Some` if the page is found, or `None` otherwise.
    ///
    /// # Errors
    /// Returns a `CJError` if there's an issue accessing the storage.
    async fn load_page(&self, level: u8, page_id: u64) -> Result<Option<JournalPage>, CJError>;

    /// Checks if a specific journal page exists in the backend.
    ///
    /// # Arguments
    /// * `level` - The time hierarchy level of the page.
    /// * `page_id` - The unique identifier of the page.
    ///
    /// # Returns
    /// `true` if the page exists, `false` otherwise.
    ///
    /// # Errors
    /// Returns a `CJError` if there's an issue accessing the storage.
    async fn page_exists(&self, level: u8, page_id: u64) -> Result<bool, CJError>;

    /// Deletes a specific page from the storage backend.
    ///
    /// # Arguments
    /// * `level` - The hierarchy level of the page to delete.
    /// * `page_id` - The ID of the page to delete.
    async fn delete_page(&self, level: u8, page_id: u64) -> Result<(), CJError>;

    /// Lists summaries of all finalized pages at a specific level.
    ///
    /// This is used by the retention policy to identify pages eligible for deletion.
    /// It should only return pages that are considered finalized and safe to potentially delete.
    ///
    /// # Arguments
    /// * `level` - The hierarchy level for which to list page summaries.
    ///
    /// # Returns
    /// A `Result` containing a vector of `JournalPageSummary` or a `CJError`.
        async fn list_finalized_pages_summary(&self, level: u8) -> Result<Vec<JournalPageSummary>, CJError>;

    /// Backs up the entire journal to the specified path.
    ///
    /// The exact format of the backup (e.g., archive, directory structure) is implementation-defined.
    ///
    /// # Arguments
    /// * `backup_path` - The file system path where the backup should be stored.
    ///
    /// # Errors
    /// Returns a `CJError` if the backup operation fails.
    async fn backup_journal(&self, backup_path: &Path) -> Result<(), CJError>;

    /// Restores the entire journal from a backup at the specified path.
    ///
    /// The journal data will be restored to the `target_journal_dir`.
    /// Existing data in `target_journal_dir` may be overwritten or cleared.
    ///
    /// # Arguments
    /// * `backup_path` - The file system path from which the backup should be read.
    /// * `target_journal_dir` - The directory where the journal data should be restored.
    ///
    /// # Errors
    /// Returns a `CJError` if the restore operation fails.
    async fn restore_journal(&self, backup_path: &Path, target_journal_dir: &Path) -> Result<(), CJError>;

    /// Loads a specific journal leaf by its hash from the backend.
    ///
    /// # Arguments
    /// * `leaf_hash` - The SHA256 hash of the leaf to retrieve.
    ///
    /// # Returns
    /// An `Option<JournalLeaf>` which is `Some` if the leaf is found, or `None` otherwise.
    ///
    /// Loads a specific journal page by its hash from the backend.
    ///
    /// # Arguments
    /// * `page_hash` - The SHA256 hash of the page to retrieve.
    ///
    /// # Returns
    /// An `Option<JournalPage>` which is `Some` if the page is found, or `None` otherwise.
    ///
    /// # Errors
    /// Returns a `CJError` if there's an issue accessing the storage.
    async fn load_page_by_hash(&self, page_hash: [u8; 32]) -> Result<Option<JournalPage>, CJError>;

    /// Loads a specific journal leaf by its hash from the backend.
    ///
    /// # Arguments
    /// * `leaf_hash` - The SHA256 hash of the leaf to retrieve.
    ///
    /// # Returns
    /// An `Option<JournalLeaf>` which is `Some` if the leaf is found, or `None` otherwise.
    ///
    /// # Errors
    /// Returns a `CJError` if there's an issue accessing the storage.
    async fn load_leaf_by_hash(&self, leaf_hash: &[u8; 32]) -> Result<Option<JournalLeaf>, CJError>;
}

/// Creates a storage backend based on the provided configuration.
///
/// # Arguments
/// * `config` - A reference to the `StorageConfig` specifying the type and settings for the backend.
///
/// # Returns
/// A `CJResult` containing a `Box<dyn StorageBackend>` on success, or a `CJError` on failure.
pub async fn create_storage_backend(config: &Config) -> CJResult<Box<dyn StorageBackend>> {
    match config.storage.storage_type {
        StorageType::Memory => {
            Ok(MemoryStorage::new().boxed())
        }
        StorageType::File => {
            if config.storage.base_path.is_empty() {
                return Err(CJError::invalid_input(
                    "FileStorage requires a non-empty base_path.".to_string()
                ));
            }
            let file_storage = FileStorage::new(&config.storage.base_path, config.compression.clone()).await?;
            Ok(file_storage.boxed())
        }
        // If StorageType were non_exhaustive or had more variants, a catch-all might be needed:
        // _ => Err(CJError::ConfigurationError(format!("Unsupported storage type: {:?}", config.storage_type))),
    }
}
