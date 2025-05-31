// src/api/async_api.rs

use crate::config::Config;
use crate::TimeLevel;
use crate::types::time::RollupConfig; // Keep for now, may be unused directly but part of Config
use crate::StorageType;
use crate::core::leaf::JournalLeaf;
use crate::core::page::PageContentHash;
use crate::core::time_manager::TimeHierarchyManager;
use crate::error::{CJError, Result as CJResult};
use crate::storage::create_storage_backend;
use chrono::{DateTime, Utc, Duration};
use serde_json::Value;
use std::sync::{Arc, OnceLock}; 

/// Provides an asynchronous API for interacting with the CivicJournal.
pub struct Journal {
    manager: TimeHierarchyManager,
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
        let storage_config = &config.storage;
        let storage_backend = create_storage_backend(storage_config)
            .await
            .map_err(|e| CJError::new(format!("Failed to create storage backend: {}", e)))?;

        // Assuming Config derives Clone for Arc::new(config.clone())
        // TimeHierarchyManager::new now returns Self, not Result, so remove map_err
        let manager = TimeHierarchyManager::new(Arc::new(config.clone()), Arc::from(storage_backend));

        Ok(Self { manager })
    }

    /// Appends a new leaf to the journal.
    ///
    /// # Arguments
    /// * `timestamp` - The timestamp for the new leaf.
    /// * `parent_hash` - Optional hash of a parent content (e.g., another leaf) this leaf is related to.
    /// * `container_id` - An identifier for the container or entity this leaf belongs to.
    /// * `data` - The JSON data payload for the leaf.
    ///
    /// # Returns
    /// The `LeafHash` of the newly appended leaf if successful.
    pub async fn append_leaf(
        &self,
        timestamp: DateTime<Utc>,
        parent_hash: Option<PageContentHash>,
        container_id: String,
        data: Value,
    ) -> CJResult<PageContentHash> {
        // Convert Option<PageContentHash> to Option<[u8; 32]> for JournalLeaf::new
        let prev_hash_bytes = parent_hash.map(|content_hash| match content_hash {
            PageContentHash::LeafHash(hash) => hash, 
            PageContentHash::ThrallPageHash(hash) => hash, 
            // Add other PageContentHash variants if they can be direct parents of a leaf
            // and their inner hash can be represented as [u8; 32]
            // e.g. PageContentHash::MerkleRoot(mh) => mh.0,
        });

        let leaf = JournalLeaf::new(timestamp, prev_hash_bytes, container_id, data)?;
        self.manager.add_leaf(&leaf).await?;
        Ok(PageContentHash::LeafHash(leaf.leaf_hash))
    }
}

#[cfg(feature = "async_api")]
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json; 
    // Config, RollupConfig, StorageType, TimeLevelConfig are brought in from crate::config at the top level of the file.
    // Duration, json, OnceLock are also imported at the top level of the file.

    // Helper to get a static config for general tests
    fn get_test_config() -> &'static Config {
        static TEST_CONFIG: OnceLock<Config> = OnceLock::new();
        TEST_CONFIG.get_or_init(|| {
            let mut config = Config::default();
            config.storage.storage_type = StorageType::Memory;
            config.storage.base_path = "".to_string(); 
            config.time_hierarchy.levels = vec![TimeLevel {
                name: "L0_general".to_string(),
                duration_seconds: 60,
                rollup_config: RollupConfig {
                    max_leaves_per_page: 10,
                    max_page_age_seconds: 300, 
                    force_rollup_on_shutdown: false, 
                },
                retention_policy: None,
            }];
            config
        })
    }

    // Helper to get a static config specifically for rollup tests
    fn get_rollup_test_config() -> &'static Config {
        static ROLLUP_TEST_CONFIG: OnceLock<Config> = OnceLock::new();
        ROLLUP_TEST_CONFIG.get_or_init(|| {
            let mut config = Config::default();
            config.storage.storage_type = StorageType::Memory;
            config.storage.base_path = "".to_string(); 
            config.time_hierarchy.levels = vec![
                TimeLevel { // L0
                    name: "L0_rollup_test".to_string(),
                    duration_seconds: 60,
                    rollup_config: RollupConfig {
                        max_leaves_per_page: 2, 
                        max_page_age_seconds: 300,
                        force_rollup_on_shutdown: false
                    },
                    retention_policy: None,
                },
                TimeLevel { // L1
                    name: "L1_rollup_test".to_string(),
                    duration_seconds: 300,
                    rollup_config: RollupConfig {
                        max_leaves_per_page: 10, 
                        max_page_age_seconds: 86400,
                        force_rollup_on_shutdown: false
                    },
                    retention_policy: None,
                }
            ];
            config
        })
    }

    #[tokio::test]
    async fn test_journal_new_and_append_single() {
        let config = get_test_config();

        let journal_result = Journal::new(config).await;
        assert!(journal_result.is_ok(), "Failed to create Journal: {:?}", journal_result.err());
        let journal = journal_result.unwrap();

        let timestamp = Utc::now();
        let container_id = "test_container_single_async".to_string();
        let data = json!({ "value": 123 });

        let result = journal.append_leaf(timestamp, None, container_id, data).await;
        assert!(result.is_ok(), "Failed to append leaf: {:?}", result.err());
        
        let leaf_hash = result.unwrap();
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash {
            assert_eq!(hash_bytes.len(), 32, "Leaf hash should be 32 bytes for SHA256");
        } else {
            panic!("Expected LeafHash variant, got {:?}", leaf_hash);
        }
        println!("Appended single leaf with hash (async): {:?}", leaf_hash);
    }

    #[tokio::test]
    async fn test_journal_append_multiple_leaves() {
        let config = get_test_config();
        let journal = Journal::new(config).await.expect("Failed to create Journal for multi-append test");

        let timestamp1 = Utc::now();
        let leaf_hash1 = journal.append_leaf(timestamp1, None, "multi_test_async".to_string(), json!({"id": 1})).await.expect("Append 1 failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash1 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!("Expected LeafHash variant for leaf_hash1, got {:?}", leaf_hash1);
        }

        let timestamp2 = timestamp1 + Duration::milliseconds(10);
        let leaf_hash2 = journal.append_leaf(timestamp2, None, "multi_test_async".to_string(), json!({"id": 2})).await.expect("Append 2 failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash2 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!("Expected LeafHash variant for leaf_hash2, got {:?}", leaf_hash2);
        }
        assert_ne!(leaf_hash1, leaf_hash2, "Leaf hashes should be unique");

        let timestamp3 = timestamp2 + Duration::milliseconds(10);
        let leaf_hash3 = journal.append_leaf(timestamp3, None, "multi_test_async".to_string(), json!({"id": 3})).await.expect("Append 3 failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash3 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!("Expected LeafHash variant for leaf_hash3, got {:?}", leaf_hash3);
        }
        assert_ne!(leaf_hash2, leaf_hash3, "Leaf hashes should be unique");

        println!("Appended multiple leaves (async): {:?}, {:?}, {:?}", leaf_hash1, leaf_hash2, leaf_hash3);
    }

    #[tokio::test]
    async fn test_journal_append_triggers_rollup() {
        let config = get_rollup_test_config(); // Uses max_leaves_per_page = 2 for L0
        let journal = Journal::new(config).await.expect("Failed to create Journal for rollup test");

        let base_timestamp = Utc::now();

        // Leaf 1: Goes into L0P0
        let leaf_hash1 = journal.append_leaf(base_timestamp, None, "rollup_trigger_async".to_string(), json!({"event": 1})).await.expect("Append 1 (rollup test) failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash1 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!("Expected LeafHash variant for leaf_hash1, got {:?}", leaf_hash1);
        }

        // Leaf 2: Fills L0P0, should trigger finalization of L0P0 and rollup to L1
        let timestamp2 = base_timestamp + Duration::milliseconds(10);
        let leaf_hash2 = journal.append_leaf(timestamp2, None, "rollup_trigger_async".to_string(), json!({"event": 2})).await.expect("Append 2 (rollup test) failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash2 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!("Expected LeafHash variant for leaf_hash2, got {:?}", leaf_hash2);
        }

        // Leaf 3: Goes into a new L0 page (L0P1, if L0 page IDs are 0-indexed and increment)
        let timestamp3 = base_timestamp + Duration::milliseconds(20);
        let leaf_hash3 = journal.append_leaf(timestamp3, None, "rollup_trigger_async".to_string(), json!({"event": 3})).await.expect("Append 3 (rollup test) failed");
        if let PageContentHash::LeafHash(hash_bytes) = leaf_hash3 {
            assert_eq!(hash_bytes.len(), 32);
        } else {
            panic!("Expected LeafHash variant for leaf_hash3, got {:?}", leaf_hash3);
        }

        // The primary assertion here is that all appends succeed without error, even when rollups occur.
        // Verifying the internal state of TimeHierarchyManager (e.g., specific page IDs or last_finalized_page_ids)
        // is beyond the scope of the current public Journal API and is covered by core time_manager tests.
        println!("Successfully appended leaves that should trigger rollup (async): {:?}, {:?}, {:?}", leaf_hash1, leaf_hash2, leaf_hash3);
    }
}
