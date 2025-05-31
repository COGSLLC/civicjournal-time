// src/core/page.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use crate::core::merkle::MerkleTree;
use crate::config::Config; // Added for calculating end_time

/// Type alias for a Merkle root, which is a SHA256 hash ([u8; 32]).
/// This is consistent with the `MerkleRoot` type in `crate::core::merkle`.
pub type MerkleRoot = [u8; 32];

/// Atomic counter for generating unique page IDs.
/// Public for testing purposes to allow resetting the counter.
pub(crate) static NEXT_PAGE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
/// Represents the type of content hash stored within a `JournalPage`.
///
/// A `JournalPage` can contain hashes of individual `JournalLeaf` entries
/// or hashes of subordinate `JournalPage` instances (thrall pages).
pub enum PageContentHash {
    /// A SHA256 hash of a `JournalLeaf`'s content.
    LeafHash([u8; 32]),
    /// A SHA256 hash (specifically, the `page_hash`) of a subordinate `JournalPage`.
    ThrallPageHash([u8; 32]),
}

/// A summary of a JournalPage, used for lightweight listings, e.g., for retention policies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalPageSummary {
    /// TODO: Document this field
    pub page_id: u64,
    /// TODO: Document this field
    pub level: u8,
    /// TODO: Document this field
    pub creation_timestamp: DateTime<Utc>, // Changed from start_time to match JournalPage
    /// TODO: Document this field
    pub end_time: DateTime<Utc>,
    /// TODO: Document this field
    pub page_hash: [u8; 32],
    // Consider adding num_items if useful for retention logic without loading full page
    // pub num_items: usize, 
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
/// Represents a page in the CivicJournal, a collection of content hashes covering a specific time window.
///
/// A `JournalPage` aggregates hashes of `JournalLeaf` entries or other `JournalPage` instances (thrall pages)
/// within its defined time range (`start_time` to `end_time`) and hierarchy `level`.
/// It includes a Merkle root of its contents and a unique, atomically generated `page_id`.
pub struct JournalPage {
    /// A unique, atomically generated identifier for this page (e.g., 42).
    pub page_id: u64,
    /// The hierarchical level of this page in the time-based aggregation (0-6).
    pub level: u8,
    /// The inclusive start timestamp of the time window covered by this page.
    pub creation_timestamp: DateTime<Utc>,
    /// The exclusive end timestamp of the time window covered by this page.
    pub end_time: DateTime<Utc>,
    /// A vector of `PageContentHash` items (leaf or thrall page hashes) aggregated by this page.
    pub content_hashes: Vec<PageContentHash>,
    /// The Merkle root calculated from the `content_hashes` of this page.
    pub merkle_root: MerkleRoot,
    /// An optional hash of the preceding page at the same level.
    pub prev_page_hash: Option<PageContentHash>,
    /// Timestamp of the last leaf added to this page. None if no leaves yet.
    pub last_leaf_timestamp: Option<DateTime<Utc>>,
    /// The SHA256 hash of this `JournalPage`'s identifying fields, including its `merkle_root`.
    pub page_hash: [u8; 32],
    /// An optional external timestamp proof for the Merkle root of this page.
    pub ts_proof: Option<Vec<u8>>,
}

impl JournalPage {
    /// Creates a new `JournalPage`.
    ///
    /// Initializes a page with a unique `page_id`, calculates its `merkle_root` from the provided
    /// `content_hashes`, and computes the overall `page_hash`.
    ///
    /// # Arguments
    ///
    /// * `level` - The hierarchical level of this page (0-6).
    /// * `prev_page_hash` - An optional hash of the preceding page at the same level.
    /// * `time_window_start` - The start of the time window.
    /// * `current_time` - The current time.
    ///
    /// # Panics
    ///
    /// This function currently uses `eprintln!` for errors during Merkle tree construction
    /// and defaults to a zeroed Merkle root. This behavior might change to return a `Result`
    /// in the future.
    pub fn new(level: u8, prev_page_hash_arg: Option<PageContentHash>, time_window_start: DateTime<Utc>, current_time: DateTime<Utc>, config: &Config) -> Self {
        let page_id = NEXT_PAGE_ID.fetch_add(1, Ordering::SeqCst);

        // Calculate end_time based on level duration
        let level_duration_seconds = config.time_hierarchy.levels
            .get(level as usize)
            .map_or(0, |lvl_config| lvl_config.duration_seconds);
        let end_time = time_window_start + chrono::Duration::seconds(level_duration_seconds as i64);

        // Merkle root for a new, empty page is default
        let merkle_root_val: MerkleRoot = [0u8; 32];

        // Calculate initial page_hash
        let mut hasher = Sha256::new();
        hasher.update(page_id.to_be_bytes());
        hasher.update(level.to_be_bytes());
        hasher.update(current_time.to_rfc3339().as_bytes()); // creation_timestamp
        hasher.update(end_time.to_rfc3339().as_bytes());
        hasher.update(merkle_root_val);
        if let Some(ref ph) = prev_page_hash_arg {
            match ph {
                PageContentHash::LeafHash(h) => hasher.update(h),
                PageContentHash::ThrallPageHash(h) => hasher.update(h),
            }
        }
        let page_hash_val: [u8; 32] = hasher.finalize().into();

        Self {
            page_id,
            level,
            creation_timestamp: current_time,
            end_time,
            content_hashes: Vec::new(),
            merkle_root: merkle_root_val,
            prev_page_hash: prev_page_hash_arg,
            last_leaf_timestamp: None,
            page_hash: page_hash_val,
            ts_proof: None,
        }
    }

    /// Adds a content hash to the page. This is a simplified helper for internal management.
    /// The main logic for adding leaves and recalculating hashes is in TimeHierarchyManager.
    pub(crate) fn add_content_hash(&mut self, hash: PageContentHash, leaf_timestamp: DateTime<Utc>) {
        self.content_hashes.push(hash);
        self.last_leaf_timestamp = Some(leaf_timestamp);
        // IMPORTANT: Merkle root and page_hash should be recalculated by the caller (TimeHierarchyManager)
        // after all modifications for a given operation are done.
    }

    /// Recalculates the Merkle root from `content_hashes` and then updates `page_hash`.
    pub fn recalculate_merkle_root_and_page_hash(&mut self) {
        let actual_leaf_hashes: Vec<[u8; 32]> = self.content_hashes
            .iter()
            .map(|ch| match ch {
                PageContentHash::LeafHash(h) => *h,
                PageContentHash::ThrallPageHash(h) => *h,
            })
            .collect();

        self.merkle_root = if actual_leaf_hashes.is_empty() {
            [0u8; 32]
        } else {
            MerkleTree::new(actual_leaf_hashes)
                .map_err(|e| {
                    eprintln!("Error creating Merkle tree: {:?}", e);
                    e
                })
                .ok()
                .and_then(|tree| tree.get_root())
                .unwrap_or([0u8; 32])
        };

        let mut hasher = Sha256::new();
        hasher.update(self.page_id.to_be_bytes());
        hasher.update(self.level.to_be_bytes());
        hasher.update(self.creation_timestamp.to_rfc3339().as_bytes());
        hasher.update(self.end_time.to_rfc3339().as_bytes());
        hasher.update(self.merkle_root);
        if let Some(ref ph) = self.prev_page_hash {
            match ph {
                PageContentHash::LeafHash(h) => hasher.update(h),
                PageContentHash::ThrallPageHash(h) => hasher.update(h),
            }
        }
        // Note: last_leaf_timestamp is not part of page_hash as it's too dynamic for page identity.
        self.page_hash = hasher.finalize().into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Utc, Duration};

    // Removed local PAGE_TEST_MUTEX, lazy_static, and unused Mutex/Ordering imports

    use crate::types::time::{TimeHierarchyConfig, TimeLevel as TypeTimeLevel, RollupConfig}; // Renamed to avoid conflict
    use crate::core::{SHARED_TEST_ID_MUTEX, reset_global_ids}; // Import shared test items

    use crate::config::{Config, StorageConfig, CompressionConfig, LoggingConfig, MetricsConfig, RetentionConfig};
    use crate::StorageType;

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
            storage: StorageConfig { storage_type: StorageType::Memory, base_path: "./cjtmp_page_test".to_string(), max_open_files: 100 },
            compression: CompressionConfig::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
            retention: RetentionConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_journal_page_creation_and_id_increment() {
        // Lock the shared mutex before resetting IDs and running the test
        let _guard = SHARED_TEST_ID_MUTEX.lock().await; // Changed to .await for Tokio Mutex
        reset_global_ids(); // Reset IDs at the beginning of the test
        let now = Utc::now();
        let config = get_test_config();
        let page1 = JournalPage::new(0, None, now, now, &config);
        let page2 = JournalPage::new(1, Some(PageContentHash::LeafHash(page1.page_hash)), now + Duration::seconds(2), now + Duration::seconds(2), &config);

        assert!(page2.page_id > page1.page_id, "page2_id ({}) should be greater than page1_id ({})", page2.page_id, page1.page_id);
        assert_eq!(page1.level, 0);
        assert_eq!(page2.level, 1);
        assert_eq!(page2.prev_page_hash, Some(PageContentHash::LeafHash(page1.page_hash)));
        assert_eq!(page1.merkle_root, [0u8; 32], "Merkle root for empty content should be default");
        assert!(page1.end_time > page1.creation_timestamp, "End time should be after creation time");
        assert_eq!(page1.end_time, page1.creation_timestamp + Duration::seconds(config.time_hierarchy.levels[0].duration_seconds as i64));

    }

    #[tokio::test]
    async fn test_journal_page_hash_uniqueness() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();
        let now = Utc::now();
        let config = get_test_config();
        let page1 = JournalPage::new(0, None, now, now, &config);
        let page2 = JournalPage::new(0, None, now + Duration::seconds(10), now + Duration::seconds(10), &config);
        let page3 = JournalPage::new(1, None, now, now, &config);
        let page4 = JournalPage::new(0, Some(PageContentHash::ThrallPageHash([1u8; 32])), now, now, &config);
        
        assert_ne!(page1.page_hash, page2.page_hash, "Page hash should differ for different time_window_start");
        assert_ne!(page1.page_hash, page3.page_hash, "Page hash should differ for different level");
        assert_ne!(page1.page_hash, page4.page_hash, "Page hash should differ for different prev_page_hash");

        let mut page5 = JournalPage::new(0, None, now, now, &config);
        page5.add_content_hash(PageContentHash::LeafHash([10u8; 32]), now + Duration::milliseconds(100));
        page5.recalculate_merkle_root_and_page_hash();

        let mut page6 = JournalPage::new(0, None, now, now, &config);
        page6.add_content_hash(PageContentHash::LeafHash([10u8; 32]), now + Duration::milliseconds(100));
        page6.add_content_hash(PageContentHash::LeafHash([20u8; 32]), now + Duration::milliseconds(200));
        page6.recalculate_merkle_root_and_page_hash();

        let mut page7 = JournalPage::new(0, None, now, now, &config);
        page7.add_content_hash(PageContentHash::LeafHash([20u8; 32]), now + Duration::milliseconds(100));
        page7.add_content_hash(PageContentHash::LeafHash([10u8; 32]), now + Duration::milliseconds(200));
        page7.recalculate_merkle_root_and_page_hash();

        assert_ne!(page1.page_hash, page5.page_hash, "Page hash should differ for different content_hashes (empty vs one)");
        assert_ne!(page5.page_hash, page6.page_hash, "Page hash should differ for different content_hashes (one vs two)");
        assert_ne!(page6.page_hash, page7.page_hash, "Page hash should differ for different order of content_hashes");

        let expected_merkle_root_page5 = MerkleTree::new(vec![[10u8; 32]]).unwrap().get_root().unwrap();
        assert_eq!(page5.merkle_root, expected_merkle_root_page5, "Merkle root for page5 is incorrect");
        assert_eq!(page1.merkle_root, [0u8; 32], "Merkle root for empty content should be default");
        assert!(page5.last_leaf_timestamp.is_some());
    }
}
