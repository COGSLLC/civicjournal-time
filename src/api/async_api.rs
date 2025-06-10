// src/api/async_api.rs

use crate::config::Config;
use crate::core::leaf::JournalLeaf;
use crate::core::page::JournalPage;
use crate::core::time_manager::TimeHierarchyManager;
use crate::error::{CJError, Result as CJResult};
use crate::storage::create_storage_backend;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Hash reference for appended leaves or thrall pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageContentHash {
    /// Hash of a journal leaf
    LeafHash([u8; 32]),
    /// Hash of a rolled up page
    ThrallPageHash([u8; 32]),
}

/// Provides an asynchronous API for interacting with the CivicJournal.
pub struct Journal {
    /// Exposed for integration testing and demo utilities.
    pub manager: Arc<TimeHierarchyManager>,
    /// Query engine used by the journal.
    pub query: crate::query::QueryEngine,
    /// Tracks the hash of the most recently appended leaf.
    last_leaf_hash: Arc<Mutex<Option<[u8; 32]>>>,
}

impl Journal {
    /// Creates a new Journal instance.
    ///
    /// Initializes the `TimeHierarchyManager` with the storage backend specified
    /// in the provided configuration.
    ///
    /// # Arguments
    /// * `config` - A static reference to the global `Config`.
    pub async fn new(config: &'static Config) -> CJResult<Self> {
        let storage_backend = create_storage_backend(config)
            .await
            .map_err(|e| CJError::new(format!("Failed to create storage backend: {}", e)))?;

        let storage_arc: Arc<dyn crate::storage::StorageBackend> = Arc::from(storage_backend);
        let manager = Arc::new(TimeHierarchyManager::new(
            Arc::new(config.clone()),
            storage_arc.clone(),
        ));
        let query = crate::query::QueryEngine::new(
            storage_arc.clone(),
            manager.clone(),
            Arc::new(config.clone()),
        );
        let last_leaf_hash = Arc::new(Mutex::new(None));

        Ok(Self {
            manager,
            query,
            last_leaf_hash,
        })
    }

    /// Appends a new leaf (an individual record or event) to the journal.
    ///
    /// This is the primary method for adding data to the CivicJournal. The method constructs
    /// a `JournalLeaf` from the provided arguments, then delegates to the `TimeHierarchyManager`
    /// to add it to the appropriate time-based page. This may trigger page finalization
    /// and roll-up processes based on the journal's configuration.
    ///
    /// # Arguments
    ///
    /// * `timestamp` - `DateTime<Utc>`: The timestamp for when the event or data represented by this leaf occurred.
    ///   This is crucial for placing the leaf into the correct time-based page.
    /// * `parent_hash` - `Option<PageContentHash>`: An optional hash of a preceding `JournalLeaf`'s content
    ///   (as `PageContentHash::LeafHash`) or a related `JournalPage`'s hash (as `PageContentHash::ThrallPageHash`).
    ///   This is used to establish a chain or link between related entries. If `None`, the leaf is considered
    ///   to not have a direct predecessor in this manner.
    /// * `container_id` - `String`: An identifier for the data container, source, or entity this leaf belongs to
    ///   (e.g., "user_activity:user_123", "system_logs:module_A").
    /// * `data` - `serde_json::Value`: The actual data payload for the leaf. This should be a valid JSON value.
    ///
    /// # Returns
    ///
    /// On success, returns `Ok(PageContentHash::LeafHash(hash))`, where `hash` is the unique SHA256 hash
    /// of the newly created and appended `JournalLeaf`.
    ///
    /// # Errors
    ///
    /// Returns `CJError` if:
    /// * The `delta_payload` (derived from `data`) cannot be serialized to bytes for hashing by `JournalLeaf::new`.
    /// * The `TimeHierarchyManager` fails to add the leaf.
    pub async fn append_leaf(
        &self,
        timestamp: DateTime<Utc>,
        parent_hash: Option<PageContentHash>,
        container_id: String,
        data: Value,
    ) -> CJResult<PageContentHash> {
        let mut last_hash_guard = self.last_leaf_hash.lock().await;
        let prev_hash_bytes = match parent_hash {
            Some(PageContentHash::LeafHash(h)) => Some(h),
            Some(PageContentHash::ThrallPageHash(h)) => Some(h),
            None => *last_hash_guard,
        };

        let leaf = JournalLeaf::new(timestamp, prev_hash_bytes, container_id, data)?;
        self.manager.add_leaf(&leaf, timestamp).await?;
        *last_hash_guard = Some(leaf.leaf_hash);
        Ok(PageContentHash::LeafHash(leaf.leaf_hash))
    }
    /// Retrieves a specific `JournalPage` from storage.
    ///
    /// # Arguments
    /// * `level` - The level of the page.
    /// * `page_id` - The ID of the page.
    ///
    /// # Returns
    /// On success, returns `Ok(JournalPage)`.
    ///
    /// # Errors
    /// Returns `CJError::PageNotFound` if the page does not exist.
    /// Returns `CJError::StorageError` if there was an issue loading from storage.
    pub async fn get_page(&self, level: u8, page_id: u64) -> CJResult<JournalPage> {
        match self.manager.load_page_from_storage(level, page_id).await {
            Ok(Some(page)) => Ok(page),
            Ok(None) => {
                if let Some(active) = self.manager.get_active_page_by_id(level, page_id).await {
                    Ok(active)
                } else {
                    Err(CJError::PageNotFound { level, page_id })
                }
            }
            Err(e) => Err(CJError::StorageError(format!(
                "Failed to load page L{}P{}: {}",
                level, page_id, e
            ))),
        }
    }

    /// Retrieves a leaf inclusion proof for the given hash.
    pub async fn get_leaf_inclusion_proof(
        &self,
        leaf_hash: &[u8; 32],
    ) -> CJResult<crate::query::types::LeafInclusionProof> {
        self.query
            .get_leaf_inclusion_proof(leaf_hash)
            .await
            .map_err(Into::into)
    }

    /// Retrieves a leaf inclusion proof with an optional page hint to speed up lookups.
    pub async fn get_leaf_inclusion_proof_with_hint(
        &self,
        leaf_hash: &[u8; 32],
        page_id_hint: Option<(u8, u64)>,
    ) -> CJResult<crate::query::types::LeafInclusionProof> {
        self.query
            .get_leaf_inclusion_proof_with_hint(leaf_hash, page_id_hint)
            .await
            .map_err(Into::into)
    }

    /// Reconstructs the state of a container at a timestamp.
    pub async fn reconstruct_container_state(
        &self,
        container_id: &str,
        at: DateTime<Utc>,
    ) -> CJResult<crate::query::types::ReconstructedState> {
        self.query
            .reconstruct_container_state(container_id, at)
            .await
            .map_err(Into::into)
    }

    /// Gets a delta report for a container between two timestamps.
    pub async fn get_delta_report(
        &self,
        container_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> CJResult<crate::query::types::DeltaReport> {
        self.query
            .get_delta_report(container_id, from, to)
            .await
            .map_err(Into::into)
    }

    /// Gets a paginated delta report for a container between two timestamps.
    pub async fn get_delta_report_paginated(
        &self,
        container_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        offset: usize,
        limit: usize,
    ) -> CJResult<crate::query::types::DeltaReport> {
        self.query
            .get_delta_report_paginated(container_id, from, to, offset, limit)
            .await
            .map_err(Into::into)
    }

    /// Checks integrity of a range of pages.
    pub async fn get_page_chain_integrity(
        &self,
        level: u8,
        from: Option<u64>,
        to: Option<u64>,
    ) -> CJResult<Vec<crate::query::types::PageIntegrityReport>> {
        self.query
            .get_page_chain_integrity(level, from, to)
            .await
            .map_err(Into::into)
    }

    /// Applies retention policies immediately. This can be used by demo utilities to force
    /// clean up or roll up finalized pages according to the configured policies.
    pub async fn apply_retention_policies(&self) -> CJResult<()> {
        self.manager
            .apply_retention_policies()
            .await
            .map_err(Into::into)
    }

    /// Creates a snapshot as of the specified timestamp using the same internal state as the journal.
    pub async fn create_snapshot(
        &self,
        as_of_timestamp: DateTime<Utc>,
        container_ids: Option<Vec<String>>,
    ) -> CJResult<[u8; 32]> {
        use crate::core::snapshot_manager::SnapshotManager;

        let snapshot_manager = SnapshotManager::new(
            self.query.config(),
            self.query.storage(),
            self.query.time_manager(),
        );

        snapshot_manager
            .create_snapshot(as_of_timestamp, container_ids)
            .await
            .map_err(|e| CJError::new(format!("Snapshot error: {:?}", e)))
    }
}

#[cfg(feature = "async_api")]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config; // Ensure Config is imported for get_rollup_test_config
    use crate::storage::memory::MemoryStorage;
    use crate::test_utils::get_test_config; // Use the shared test config
    use crate::test_utils::{reset_global_ids, SHARED_TEST_ID_MUTEX};
    use crate::types::time::LevelRollupConfig;
    use crate::StorageType;
    use crate::TimeLevel;
    use chrono::Duration;
    use serde_json::json;
    use std::sync::{Arc, OnceLock};

    // Specific config for rollup tests
    fn get_rollup_test_config() -> &'static Config {
        static ROLLUP_TEST_CONFIG: OnceLock<Config> = OnceLock::new();
        ROLLUP_TEST_CONFIG.get_or_init(|| {
            let mut config = Config::default();
            config.storage.storage_type = StorageType::Memory;
            config.storage.base_path = "".to_string();
            config.time_hierarchy.levels = vec![
                TimeLevel {
                    name: "L0_rollup_test".to_string(),
                    duration_seconds: 60,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 2,
                        max_page_age_seconds: 300,
                        content_type: crate::types::time::RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                },
                TimeLevel {
                    name: "L1_rollup_test".to_string(),
                    duration_seconds: 300,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 10,
                        max_page_age_seconds: 86400,
                        content_type: crate::types::time::RollupContentType::ChildHashes,
                    },
                    retention_policy: None,
                },
            ];
            config
        })
    }

    #[tokio::test]
    async fn test_journal_new_and_append_single() {
        let config = get_test_config();

        let journal_result = Journal::new(config).await;
        assert!(
            journal_result.is_ok(),
            "Failed to create Journal: {:?}",
            journal_result.err()
        );
        let journal = journal_result.unwrap();

        let timestamp = Utc::now();
        let container_id = "test_container_single_async".to_string();
        let data = json!({ "value": 123 });

        let result = journal
            .append_leaf(timestamp, None, container_id, data)
            .await;
        assert!(result.is_ok(), "Failed to append leaf: {:?}", result.err());

        let leaf_hash = result.unwrap();
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash {
            assert_eq!(
                hash_bytes.len(),
                32,
                "Leaf hash should be 32 bytes for SHA256"
            );
        } else {
            panic!("Expected LeafHash variant, got {:?}", leaf_hash);
        }
        println!("Appended single leaf with hash (async): {:?}", leaf_hash);
    }

    #[tokio::test]
    async fn test_journal_append_multiple_leaves() {
        let config = get_test_config();
        let journal = Journal::new(config)
            .await
            .expect("Failed to create Journal for multi-append test");

        let timestamp1 = Utc::now();
        let leaf_hash1 = journal
            .append_leaf(
                timestamp1,
                None,
                "multi_test_async".to_string(),
                json!({"id": 1}),
            )
            .await
            .expect("Append 1 failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash1 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!(
                "Expected LeafHash variant for leaf_hash1, got {:?}",
                leaf_hash1
            );
        }

        let timestamp2 = timestamp1 + Duration::milliseconds(10);
        let leaf_hash2 = journal
            .append_leaf(
                timestamp2,
                None,
                "multi_test_async".to_string(),
                json!({"id": 2}),
            )
            .await
            .expect("Append 2 failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash2 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!(
                "Expected LeafHash variant for leaf_hash2, got {:?}",
                leaf_hash2
            );
        }
        assert_ne!(leaf_hash1, leaf_hash2, "Leaf hashes should be unique");

        let timestamp3 = timestamp2 + Duration::milliseconds(10);
        let leaf_hash3 = journal
            .append_leaf(
                timestamp3,
                None,
                "multi_test_async".to_string(),
                json!({"id": 3}),
            )
            .await
            .expect("Append 3 failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash3 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!(
                "Expected LeafHash variant for leaf_hash3, got {:?}",
                leaf_hash3
            );
        }
        assert_ne!(leaf_hash2, leaf_hash3, "Leaf hashes should be unique");

        println!(
            "Appended multiple leaves (async): {:?}, {:?}, {:?}",
            leaf_hash1, leaf_hash2, leaf_hash3
        );
    }

    #[tokio::test]
    async fn test_journal_append_triggers_rollup() {
        let config = get_rollup_test_config();
        let journal = Journal::new(config)
            .await
            .expect("Failed to create Journal for rollup test");

        let base_timestamp = Utc::now();

        let leaf_hash1 = journal
            .append_leaf(
                base_timestamp,
                None,
                "rollup_trigger_async".to_string(),
                json!({"event": 1}),
            )
            .await
            .expect("Append 1 (rollup test) failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash1 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!(
                "Expected LeafHash variant for leaf_hash1, got {:?}",
                leaf_hash1
            );
        }

        let timestamp2 = base_timestamp + Duration::milliseconds(10);
        let leaf_hash2 = journal
            .append_leaf(
                timestamp2,
                None,
                "rollup_trigger_async".to_string(),
                json!({"event": 2}),
            )
            .await
            .expect("Append 2 (rollup test) failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash2 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!(
                "Expected LeafHash variant for leaf_hash2, got {:?}",
                leaf_hash2
            );
        }

        let timestamp3 = base_timestamp + Duration::milliseconds(20);
        let leaf_hash3 = journal
            .append_leaf(
                timestamp3,
                None,
                "rollup_trigger_async".to_string(),
                json!({"event": 3}),
            )
            .await
            .expect("Append 3 (rollup test) failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash3 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!(
                "Expected LeafHash variant for leaf_hash3, got {:?}",
                leaf_hash3
            );
        }

        println!(
            "Successfully appended leaves that should trigger rollup (async): {:?}, {:?}, {:?}",
            leaf_hash1, leaf_hash2, leaf_hash3
        );
    }

    #[tokio::test]
    async fn test_journal_append_leaf_storage_error() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;

        let mut config = get_test_config().clone();
        if !config.time_hierarchy.levels.is_empty() {
            config.time_hierarchy.levels[0]
                .rollup_config
                .max_items_per_page = 1;
        } else {
            panic!("Test config has no time hierarchy levels defined!");
        }

        let memory_storage = MemoryStorage::new();
        memory_storage.set_fail_on_store(0, None);

        reset_global_ids();

        let storage_backend: Arc<dyn crate::storage::StorageBackend> = Arc::new(memory_storage);
        let manager =
            TimeHierarchyManager::new(Arc::new(config.clone()), Arc::clone(&storage_backend));
        let manager_arc = Arc::new(manager);
        let query = crate::query::QueryEngine::new(
            storage_backend.clone(),
            manager_arc.clone(),
            Arc::new(config.clone()),
        );
        let journal = Journal {
            manager: manager_arc,
            query,
        };

        let timestamp = Utc::now();
        let append_result = journal
            .append_leaf(
                timestamp,
                None,
                "error_test_async".to_string(),
                json!({ "event": "storage_should_fail" }),
            )
            .await;

        assert!(
            append_result.is_err(),
            "append_leaf should have returned an error"
        );
        match append_result.err().unwrap() {
            CJError::StorageError(msg) => {
                assert_eq!(
                    msg,
                    format!(
                        "Simulated MemoryStorage write failure for any page on L{}",
                        0
                    ),
                    "Error message mismatch. Got: {}",
                    msg
                );
            }
            e => panic!("Expected StorageError, got {:?}", e),
        }
        println!("Successfully verified storage error propagation in append_leaf (async).");
    }

    #[tokio::test]
    async fn test_journal_get_non_existent_page() {
        let config = get_test_config();
        let journal = Journal::new(config)
            .await
            .expect("Failed to create Journal for get_page test");

        let level = 0u8;
        let page_id = 9999u64; // An ID that is very unlikely to exist

        let result = journal.get_page(level, page_id).await;

        assert!(
            result.is_err(),
            "get_page for non-existent page should return an error. Got: {:?}",
            result
        );
        match result.err().unwrap() {
            CJError::PageNotFound {
                level: err_level,
                page_id: err_page_id,
            } => {
                assert_eq!(err_level, level, "Error level mismatch");
                assert_eq!(err_page_id, page_id, "Error page_id mismatch");
            }
            e => panic!("Expected PageNotFound error, got {:?}", e),
        }
        println!(
            "Successfully verified PageNotFound for non-existent page L{}P{} (async).",
            level, page_id
        );
    }
}
