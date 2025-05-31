// src/api/mod.rs

/// Synchronous API implementations for CivicJournal operations.
pub mod sync_api;

#[cfg(feature = "async_api")]
pub mod async_api;

// Define public API traits
/*
pub trait CivicJournalApi {
    // Define methods like append_delta, get_leaf_proof, etc.
    // fn append_delta(&mut self, container_id: &str, payload: &serde_json::Value) -> Result<crate::core::leaf::JournalLeaf, crate::error::CJError>;
}
*/

// Re-export API implementations or traits
// pub use sync_api::SyncApi;
