// src/storage/mod.rs

/// In-memory storage backend for journal data (primarily for testing or ephemeral use).
pub mod memory;
/// File-based storage backend for persisting journal data to disk.
pub mod file;

// Define a common storage trait

use crate::core::page::JournalPage;
use crate::error::CJError;
use async_trait::async_trait;

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

    // TODO: Consider adding other methods as needed, e.g.:
    // async fn delete_page(&self, level: u8, page_id: u64) -> Result<(), CJError>;
    // async fn list_pages_at_level(&self, level: u8) -> Result<Vec<u64>, CJError>; // Returns list of PageIDs
}

// Re-export implementations or the trait
// pub use memory::InMemoryStorage;
// pub use file::FileStorage;
