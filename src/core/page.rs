// src/core/page.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use crate::core::merkle::MerkleTree;
use crate::core::leaf::JournalLeaf;
use crate::config::Config; // Added for calculating end_time
use crate::types::time::RollupContentType;

/// Type alias for a Merkle root, which is a SHA256 hash ([u8; 32]).
/// This is consistent with the `MerkleRoot` type in `crate::core::merkle`.
pub type MerkleRoot = [u8; 32];

/// Atomic counter for generating unique page IDs.
/// Public for testing purposes to allow resetting the counter.
pub static NEXT_PAGE_ID: AtomicU64 = AtomicU64::new(0);

use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum PageContent {
    Leaves(Vec<JournalLeaf>),
    ThrallHashes(Vec<[u8; 32]>),
    NetPatches(HashMap<String, HashMap<String, serde_json::Value>>), // ObjectID -> FieldName -> Value
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
    pub content: PageContent,
    /// The Merkle root calculated from the `content` of this page.
    pub merkle_root: MerkleRoot,
    /// An optional hash of the preceding page at the same level.
    pub prev_page_hash: Option<[u8; 32]>,
    /// Timestamp of the last leaf added to this page. None if no leaves yet.
    pub last_leaf_timestamp: Option<DateTime<Utc>>,
    /// The SHA256 hash of this `JournalPage`'s identifying fields, including its `merkle_root`.
    pub page_hash: [u8; 32],
    /// An optional external timestamp proof for the Merkle root of this page.
    pub ts_proof: Option<Vec<u8>>,
    /// Timestamp of the first child item (leaf or rolled-up page) included in this page (for L_N > 0).
    pub first_child_ts: Option<DateTime<Utc>>,
    /// Timestamp of the last child item (leaf or rolled-up page) included in this page (for L_N > 0).
    pub last_child_ts: Option<DateTime<Utc>>,
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
    pub fn new(
        level: u8,
        prev_page_hash_arg: Option<[u8; 32]>,
        time_window_start: DateTime<Utc>,
        config: &Config,) -> Self {
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
        hasher.update(time_window_start.to_rfc3339().as_bytes()); // creation_timestamp
        hasher.update(end_time.to_rfc3339().as_bytes());
        hasher.update(merkle_root_val);
        if let Some(prev_hash_val) = prev_page_hash_arg {
            hasher.update(prev_hash_val);
        }
        let page_hash_val: [u8; 32] = hasher.finalize().into();

        Self {
            page_id,
            level,
            creation_timestamp: time_window_start,
            end_time,
            content: {
                if level == 0 {
                    PageContent::Leaves(Vec::new())
                } else {
                    let level_config = config.time_hierarchy.levels.get(level as usize)
                        .unwrap_or_else(|| panic!("Level {} config missing in JournalPage::new. Config: {:?}", level, config.time_hierarchy.levels));
                    match level_config.rollup_config.content_type {
                        RollupContentType::ChildHashes => PageContent::ThrallHashes(Vec::new()),
                        RollupContentType::NetPatches => PageContent::NetPatches(HashMap::new()),
                    }
                }
            },
            merkle_root: [0u8; 32], // Default for empty content, will be recalculated
            prev_page_hash: prev_page_hash_arg,
            last_leaf_timestamp: None, // Specific to L0 pages with actual leaves
            page_hash: page_hash_val,
            ts_proof: None,
            first_child_ts: None, // For L_N > 0 pages
            last_child_ts: None,  // For L_N > 0 pages
        }
    }

    /// Adds a JournalLeaf to an L0 page.
    /// Panics if called on a non-L0 page or if content is not PageContent::Leaves.
    pub(crate) fn add_leaf(&mut self, leaf: JournalLeaf) {
        if self.level != 0 {
            panic!("add_leaf can only be called on L0 pages.");
        }
        match self.content {
            PageContent::Leaves(ref mut leaves) => {
                self.last_leaf_timestamp = Some(leaf.timestamp); // This is for L0 original leaf timestamp tracking
                if self.first_child_ts.is_none() || leaf.timestamp < self.first_child_ts.unwrap() {
                    self.first_child_ts = Some(leaf.timestamp);
                }
                if self.last_child_ts.is_none() || leaf.timestamp > self.last_child_ts.unwrap() {
                    self.last_child_ts = Some(leaf.timestamp);
                }
                leaves.push(leaf);
            }
            _ => panic!("Attempted to add leaf to a page not containing PageContent::Leaves."),
        }
        // IMPORTANT: Merkle root and page_hash should be recalculated by the caller.
    }

    /// Adds a thrall page's hash to an L1+ page.
    /// Panics if called on an L0 page or if content is not PageContent::ThrallHashes.
    pub(crate) fn add_thrall_hash(&mut self, thrall_page_hash: [u8; 32], content_timestamp: DateTime<Utc>) {
        if self.level == 0 {
            panic!("add_thrall_hash cannot be called on L0 pages.");
        }
        match self.content {
            PageContent::ThrallHashes(ref mut hashes) => {
                println!("[ADD_THRALL_HASH_DBG] L{}P{}: Received content_timestamp = {}, current self.first_child_ts = {:?}", self.level, self.page_id, content_timestamp, self.first_child_ts);
                if self.first_child_ts.is_none() {
                    self.first_child_ts = Some(content_timestamp);
                }
                println!("[ADD_THRALL_HASH_DBG] L{}P{}: After update logic, self.first_child_ts = {:?}", self.level, self.page_id, self.first_child_ts);
                if self.last_child_ts.is_none() || content_timestamp > self.last_child_ts.unwrap() {
                    self.last_child_ts = Some(content_timestamp);
                }
                hashes.push(thrall_page_hash);
            }
            _ => panic!("Attempted to add thrall_hash to a page not containing PageContent::ThrallHashes."),
        }
        // IMPORTANT: Merkle root and page_hash should be recalculated by the caller.
    }

    /// Merges NetPatches into an L1+ page.
    /// Panics if called on an L0 page or if content is not PageContent::NetPatches.
    pub(crate) fn merge_net_patches(&mut self, patches_to_merge: HashMap<String, HashMap<String, serde_json::Value>>, content_timestamp: DateTime<Utc>) {
        if self.level == 0 {
            panic!("merge_net_patches cannot be called on L0 pages.");
        }
        match self.content {
            PageContent::NetPatches(ref mut existing_patches) => {
                for (object_id, field_patches) in patches_to_merge {
                    let entry = existing_patches.entry(object_id).or_insert_with(HashMap::new);
                    for (field_name, value) in field_patches {
                        entry.insert(field_name, value);
                    }
                }

                println!("[MERGE_NET_PATCHES_DBG] L{}P{}: Received content_timestamp = {}, current self.first_child_ts = {:?}", self.level, self.page_id, content_timestamp, self.first_child_ts);
                if self.first_child_ts.is_none() {
                    self.first_child_ts = Some(content_timestamp);
                }
                println!("[MERGE_NET_PATCHES_DBG] L{}P{}: After update logic, self.first_child_ts = {:?}", self.level, self.page_id, self.first_child_ts);
                if self.last_child_ts.is_none() || content_timestamp > self.last_child_ts.unwrap() {
                    self.last_child_ts = Some(content_timestamp);
                }
            }
            _ => panic!("Attempted to merge_net_patches into a page not containing PageContent::NetPatches."),
        }
        // IMPORTANT: Merkle root and page_hash should be recalculated by the caller.
    }


    /// Recalculates the Merkle root from `content_hashes` and then updates `page_hash`.
    pub fn recalculate_merkle_root_and_page_hash(&mut self) {
        let actual_leaf_hashes: Vec<[u8; 32]> = match self.content {
            PageContent::Leaves(ref leaves) => leaves.iter().map(|leaf| leaf.leaf_hash).collect(),
            PageContent::ThrallHashes(ref hashes) => hashes.clone(),
            PageContent::NetPatches(ref patches_map) => {
                if patches_map.is_empty() {
                    Vec::new()
                } else {
                    let mut net_patch_tuples_hashes = Vec::new();
                    // Sort by ObjectID (String key)
                    let mut sorted_object_ids: Vec<_> = patches_map.keys().collect();
                    sorted_object_ids.sort_unstable();

                    for object_id_str in sorted_object_ids {
                        if let Some(field_map) = patches_map.get(object_id_str) {
                            // Sort by FieldName (String key)
                            let mut sorted_field_names: Vec<_> = field_map.keys().collect();
                            sorted_field_names.sort_unstable();

                            for field_name_str in sorted_field_names {
                                if let Some(value_json) = field_map.get(field_name_str) {
                                    // Serialize value to canonical bytes
                                    let value_bytes = serde_json::to_vec(value_json)
                                        .unwrap_or_else(|e| {
                                            eprintln!("Failed to serialize NetPatch value to JSON: {}", e);
                                            Vec::new()
                                        });
                                    let value_hash: [u8; 32] = Sha256::digest(&value_bytes).into();

                                    let mut tuple_hasher = Sha256::new();
                                    tuple_hasher.update(object_id_str.as_bytes());
                                    tuple_hasher.update(field_name_str.as_bytes());
                                    tuple_hasher.update(value_hash);
                                    net_patch_tuples_hashes.push(tuple_hasher.finalize().into());
                                }
                            }
                        }
                    }
                    net_patch_tuples_hashes
                }
            }
        };

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
        if let Some(prev_hash_val) = self.prev_page_hash {
            hasher.update(prev_hash_val);
        }
        if let Some(first_ts) = self.first_child_ts {
            hasher.update(first_ts.to_rfc3339().as_bytes());
        }
        if let Some(last_ts) = self.last_child_ts {
            hasher.update(last_ts.to_rfc3339().as_bytes());
        }
        // Note: last_leaf_timestamp (original L0 leaf time) is distinct from first/last_child_ts (rollup content time range)
        // and is not part of page_hash as it's too dynamic for page identity if we were to update it for L_N > 0.
        self.page_hash = hasher.finalize().into();
    }

    /// Returns the number of items (leaves or thrall hashes) in the page's content.
    pub fn content_len(&self) -> usize {
        match &self.content {
            PageContent::Leaves(leaves) => leaves.len(),
            PageContent::ThrallHashes(hashes) => hashes.len(),
            PageContent::NetPatches(patches) => patches.len(), // Number of ObjectIDs
        }
    }

    /// Checks if the page's content is empty.
    pub fn is_content_empty(&self) -> bool {
        match &self.content {
            PageContent::Leaves(leaves) => leaves.is_empty(),
            PageContent::ThrallHashes(hashes) => hashes.is_empty(),
            PageContent::NetPatches(patches) => patches.is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Utc, Duration};
    use crate::core::leaf::{LeafData, LeafDataV1};

    // Removed local PAGE_TEST_MUTEX, lazy_static, and unused Mutex/Ordering imports

    use crate::types::time::{TimeHierarchyConfig, TimeLevel as TypeTimeLevel}; // Renamed to avoid conflict
    use crate::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids}; // Import shared test items

    use crate::config::{Config, StorageConfig, CompressionConfig, LoggingConfig, MetricsConfig, RetentionConfig};
    use crate::types::time::{TimeLevel, LevelRollupConfig}; // TimeHierarchyConfig removed, already imported
    use crate::StorageType;

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
            storage: StorageConfig { storage_type: StorageType::Memory, base_path: "./cjtmp_page_test".to_string(), max_open_files: 100 },
            compression: CompressionConfig::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
            retention: RetentionConfig::default(),
        }
    }

    // Helper to create a dummy JournalLeaf for testing
    fn create_dummy_leaf(timestamp: DateTime<Utc>, content_bytes: &[u8], container_id_suffix: &str) -> JournalLeaf {
        let leaf_data = LeafDataV1 {
            timestamp,
            content_type: "application/octet-stream".to_string(),
            content: content_bytes.to_vec(),
            author: "page_test".to_string(),
            signature: "sig_page_test".to_string(),
        };
        let payload = serde_json::to_value(LeafData::V1(leaf_data)).expect("Failed to serialize dummy LeafDataV1");
        JournalLeaf::new(
            timestamp,
            None, // prev_leaf_hash for dummy leaf
            format!("dummy_container_{}", container_id_suffix),
            payload
        ).expect("Failed to create dummy JournalLeaf")
    }

    #[tokio::test]
    async fn test_journal_page_creation_and_id_increment() {
        // Lock the shared mutex before resetting IDs and running the test
        let _guard = SHARED_TEST_ID_MUTEX.lock().await; // Changed to .await for Tokio Mutex
        reset_global_ids(); // Reset IDs at the beginning of the test
        let now = Utc::now();
        let config = get_test_config();
        let page1 = JournalPage::new(0, None, now, &config);
        let page2 = JournalPage::new(1, Some(page1.page_hash), now + Duration::seconds(2), &config);

        assert!(page2.page_id > page1.page_id, "page2_id ({}) should be greater than page1_id ({})", page2.page_id, page1.page_id);
        assert_eq!(page1.level, 0);
        assert_eq!(page2.level, 1);
        assert_eq!(page2.prev_page_hash, Some(page1.page_hash));
        assert_eq!(page1.merkle_root, [0u8; 32], "Merkle root for empty content should be default");
        assert!(page1.end_time > page1.creation_timestamp, "End time should be after creation time");
        assert_eq!(page1.end_time, page1.creation_timestamp + Duration::seconds(config.time_hierarchy.levels[0].duration_seconds as i64));

    }

    #[tokio::test]
    async fn test_journal_page_hash_uniqueness() {
        println!("[TEST-DEBUG] Starting test_journal_page_hash_uniqueness");
        let _guard = crate::test_utils::acquire_test_mutex("test_journal_page_hash_uniqueness").await;
        reset_global_ids();
        println!("[TEST-DEBUG] Creating pages with different timestamps");
        let now = Utc::now();
        let config = get_test_config();
        println!("[TEST-DEBUG] Creating page1");
        let page1 = JournalPage::new(0, None, now, &config);
        println!("[TEST-DEBUG] Creating page2");
        let page2 = JournalPage::new(0, None, now + Duration::seconds(10), &config);
        let page3 = JournalPage::new(1, None, now, &config);
        let page4 = JournalPage::new(0, Some([1u8; 32]), now, &config);
        
        assert_ne!(page1.page_hash, page2.page_hash, "Page hash should differ for different time_window_start");
        assert_ne!(page1.page_hash, page3.page_hash, "Page hash should differ for different level");
        assert_ne!(page1.page_hash, page4.page_hash, "Page hash should differ for different prev_page_hash");

        let mut page5 = JournalPage::new(0, None, now, &config);
        let leaf5_1 = create_dummy_leaf(now + Duration::milliseconds(100), &[10u8; 32], "p5_1");
        page5.add_leaf(leaf5_1.clone());
        page5.recalculate_merkle_root_and_page_hash();

        let mut page6 = JournalPage::new(0, None, now, &config);
        let leaf6_1 = create_dummy_leaf(now + Duration::milliseconds(100), &[10u8; 32], "p6_1");
        let leaf6_2 = create_dummy_leaf(now + Duration::milliseconds(200), &[20u8; 32], "p6_2");
        page6.add_leaf(leaf6_1.clone());
        page6.add_leaf(leaf6_2.clone());
        page6.recalculate_merkle_root_and_page_hash();

        let mut page7 = JournalPage::new(0, None, now, &config);
        let leaf7_1 = create_dummy_leaf(now + Duration::milliseconds(100), &[20u8; 32], "p7_1");
        let leaf7_2 = create_dummy_leaf(now + Duration::milliseconds(200), &[10u8; 32], "p7_2");
        page7.add_leaf(leaf7_1.clone());
        page7.add_leaf(leaf7_2.clone());
        page7.recalculate_merkle_root_and_page_hash();

        assert_ne!(page1.page_hash, page5.page_hash, "Page hash should differ for different content_hashes (empty vs one)");
        assert_ne!(page5.page_hash, page6.page_hash, "Page hash should differ for different content_hashes (one vs two)");
        assert_ne!(page6.page_hash, page7.page_hash, "Page hash should differ for different order of content_hashes");

        // With add_leaf, the merkle root is calculated internally based on leaf hashes.
        // We just need to ensure it's not the default empty root.
        let default_merkle_root = [0u8; 32]; // Default/empty Merkle root
        assert_ne!(page5.merkle_root, default_merkle_root, "Merkle root for page5 should not be default after adding a leaf");
        assert_eq!(page1.merkle_root, [0u8; 32], "Merkle root for empty content should be default");
        assert!(page5.last_leaf_timestamp.is_some());
    }
}
