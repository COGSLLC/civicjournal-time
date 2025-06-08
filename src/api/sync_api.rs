// src/api/sync_api.rs

use crate::config::Config;
use crate::core::page::JournalPage;
use crate::core::time_manager::TimeHierarchyManager;
use crate::error::{CJError, Result as CJResult};
use crate::storage::create_storage_backend;
use std::sync::Arc;
use tokio::runtime::Runtime;
use chrono::{DateTime, Utc};

/// Provides a synchronous API for interacting with the CivicJournal.
#[derive(Debug)]
pub struct Journal {
    manager: Arc<TimeHierarchyManager>,
    query: crate::query::QueryEngine,
    rt: Runtime, // Tokio runtime for executing async operations
}

impl Journal {
    /// Creates a new synchronous Journal instance.
    ///
    /// Initializes the `TimeHierarchyManager` with the storage backend specified
    /// in the provided configuration. This involves blocking on some async operations.
    ///
    /// # Arguments
    /// * `config` - A static reference to the global `Config`.
    ///
    /// # Panics
    /// Panics if a Tokio runtime cannot be created or if storage backend creation fails.
    pub fn new(config: &'static Config) -> CJResult<Self> {
        let rt = Runtime::new().map_err(|e| CJError::new(format!("Failed to create Tokio runtime: {}", e)))?;

        let storage_backend = rt.block_on(create_storage_backend(config))
            .map_err(|e| CJError::new(format!("Failed to create storage backend: {}", e)))?;

        let storage_arc: Arc<dyn crate::storage::StorageBackend> = Arc::from(storage_backend);
        let manager = Arc::new(TimeHierarchyManager::new(Arc::new(config.clone()), storage_arc.clone()));
        let query = crate::query::QueryEngine::new(storage_arc.clone(), manager.clone(), Arc::new(config.clone()));

        Ok(Self { manager, query, rt })
    }

    /// Retrieves a specific `JournalPage` by its level and ID.
    ///
    /// This method blocks the current thread until the page is retrieved from storage
    /// or an error occurs.
    ///
    /// # Arguments
    /// * `level` - The hierarchy level of the page.
    /// * `page_id` - The ID of the page to retrieve.
    ///
    /// # Returns
    /// Returns `Ok(JournalPage)` if the page is found.
    /// Returns `CJError::PageNotFound` if the page does not exist.
    /// Returns `CJError::StorageError` if there was an issue loading from storage.
    pub fn get_page(&self, level: u8, page_id: u64) -> CJResult<JournalPage> {
        match self.rt.block_on(self.manager.load_page_from_storage(level, page_id)) {
            Ok(Some(page)) => Ok(page),
            Ok(None) => Err(CJError::PageNotFound { level, page_id }),
            Err(e) => {
                // The error from get_page_from_storage is already a CJError::StorageError
                // with a formatted message, so we can just propagate it.
                // If it were a different error type, we might need to map it here.
                Err(e)
            }
        }
    }

    /// Retrieves a leaf inclusion proof for the given leaf hash.
    ///
    /// This is a blocking wrapper around [`QueryEngine::get_leaf_inclusion_proof`]
    /// for use in synchronous contexts.
    pub fn get_leaf_inclusion_proof(&self, leaf_hash: &[u8; 32]) -> CJResult<crate::query::types::LeafInclusionProof> {
        self.rt.block_on(self.query.get_leaf_inclusion_proof(leaf_hash)).map_err(Into::into)
    }

    /// Retrieves a leaf inclusion proof with an optional page hint to speed up lookups.
    ///
    /// `page_id_hint` may contain the level and page ID where the caller believes
    /// the leaf resides, avoiding a full search if correct.
    pub fn get_leaf_inclusion_proof_with_hint(
        &self,
        leaf_hash: &[u8; 32],
        page_id_hint: Option<(u8, u64)>,
    ) -> CJResult<crate::query::types::LeafInclusionProof> {
        self
            .rt
            .block_on(self.query.get_leaf_inclusion_proof_with_hint(leaf_hash, page_id_hint))
            .map_err(Into::into)
    }

    /// Reconstructs the state of a container at the specified timestamp.
    pub fn reconstruct_container_state(&self, container_id: &str, at: DateTime<Utc>) -> CJResult<crate::query::types::ReconstructedState> {
        self.rt.block_on(self.query.reconstruct_container_state(container_id, at)).map_err(Into::into)
    }

    /// Retrieves all deltas for a container between two timestamps.
    pub fn get_delta_report(&self, container_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> CJResult<crate::query::types::DeltaReport> {
        self.rt.block_on(self.query.get_delta_report(container_id, from, to)).map_err(Into::into)
    }

    /// Retrieves a paginated delta report for a container between two timestamps.
    ///
    /// `offset` specifies how many matching deltas to skip before returning results
    /// and `limit` caps the number of deltas returned.
    pub fn get_delta_report_paginated(
        &self,
        container_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        offset: usize,
        limit: usize,
    ) -> CJResult<crate::query::types::DeltaReport> {
        self
            .rt
            .block_on(self.query.get_delta_report_paginated(container_id, from, to, offset, limit))
            .map_err(Into::into)
    }

    /// Verifies integrity of a range of pages at the given level.
    ///
    /// Returns a list of [`PageIntegrityReport`] entries describing any
    /// inconsistencies.
    pub fn get_page_chain_integrity(&self, level: u8, from: Option<u64>, to: Option<u64>) -> CJResult<Vec<crate::query::types::PageIntegrityReport>> {
        self.rt.block_on(self.query.get_page_chain_integrity(level, from, to)).map_err(Into::into)
    }
}

// pub struct SyncApi {
    // storage: Box<dyn StorageBackend>,
    // time_hierarchy: TimeHierarchyManager, // Or similar
// }

// impl SyncApi {
//     pub fn new(/* dependencies */) -> Self {
//         // ...
//     }
// }

// impl CivicJournalApi for SyncApi {
    // fn append_delta(&mut self, container_id: &str, payload: &serde_json::Value) -> Result<JournalLeaf, CJError> {
    //     // 1. Determine timestamp
    //     // 2. Get/create appropriate Level 0 page via TimeHierarchyManager
    //     // 3. Construct JournalLeaf (calculate PrevHash, LeafHash)
    //     // 4. Store leaf via StorageBackend
    //     // 5. Buffer leaf hash in page
    //     // 6. Handle page flushing if necessary
    //     unimplemented!()
    // }
    // ... other API methods
// }

#[cfg(test)]
mod tests {
    // Removed local OnceLock, Config, RollupConfig, TimeLevel, StorageType imports as they are now handled by test_utils or not directly needed here.
    // Imports for items used directly in tests (like CJError or the Journal struct itself) remain.
    use crate::error::CJError;
    use crate::test_utils::get_test_config; // Use the shared test config

    #[test]
    fn it_works_sync_api() {
        // Basic new test
        let config = get_test_config();
        let journal_result = super::Journal::new(config);
        assert!(journal_result.is_ok(), "Failed to create sync journal: {:?}", journal_result.err());
    }

    #[test]
    fn test_journal_get_non_existent_page_sync() {
        let config = get_test_config();
        let journal = super::Journal::new(config).expect("Failed to create sync journal");

        let level = 0;
        let page_id = 99; // Assuming this page does not exist

        match journal.get_page(level, page_id) {
            Err(CJError::PageNotFound { level: l, page_id: p }) => {
                assert_eq!(l, level);
                assert_eq!(p, page_id);
            }
            Ok(_) => panic!("Expected PageNotFound error, but got Ok"),
            Err(e) => panic!("Expected PageNotFound error, but got other error: {:?}", e),
        }
    }
}

