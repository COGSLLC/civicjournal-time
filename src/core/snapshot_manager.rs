use std::sync::Arc;
use chrono::{DateTime, Utc};
use crate::config::Config;
use crate::storage::StorageBackend;
use crate::core::time_manager::TimeHierarchyManager;
use crate::core::leaf::JournalLeaf;
use crate::core::page::{JournalPage, PageContent};
use crate::core::snapshot::{SnapshotPagePayload, SnapshotContainerState};
use crate::core::merkle::MerkleTree;
use sha2::Digest; // For Sha256::new()
use std::collections::{HashMap, HashSet};
use log; // For logging errors and warnings

/// Errors that can occur during snapshot operations.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("Configuration error: {0}")]
    /// Indicates an error related to snapshot configuration.
    ConfigError(String),
    #[error("Storage backend error: {0}")]
    /// Indicates an error originating from the storage backend during snapshot operations.
    StorageError(String),
    #[error("Time hierarchy manager error: {0}")]
    /// Indicates an error originating from the TimeHierarchyManager.
    TimeManagerError(String),
    #[error("State reconstruction failed: {0}")]
    /// Indicates an error during the state reconstruction phase of snapshot creation.
    ReconstructionError(String),
    #[error("Failed to create snapshot page: {0}")]
    /// Indicates an error when creating the snapshot journal page.
    PageCreationError(String),
    #[error("An underlying I/O error occurred: {0}")]
    /// An underlying I/O error occurred.
    IoError(#[from] std::io::Error),
    #[error("An unexpected internal error: {0}")]
    /// An unexpected internal error occurred.
    InternalError(String),
}

impl From<crate::error::CJError> for SnapshotError {
    fn from(err: crate::error::CJError) -> Self {
        // You might want to match on err to provide more specific SnapshotError variants
        // For now, a general conversion:
        SnapshotError::InternalError(format!("An underlying system error occurred: {}", err))
    }
}

// Placeholder for QueryError if needed for reconstruction logic
// impl From<crate::query::QueryError> for SnapshotError {
//     fn from(err: crate::query::QueryError) -> Self {
//         SnapshotError::ReconstructionError(err.to_string())
//     }
// }

/// Manages the creation and storage of system snapshots.
///
/// This struct orchestrates the process of capturing the state of containers
/// at a specific point in time and persisting this state as a snapshot page.
#[allow(dead_code)] // Fields will be used as implementation progresses
pub struct SnapshotManager {
    config: Arc<Config>,
    storage: Arc<dyn StorageBackend + Send + Sync>,
    time_manager: Arc<TimeHierarchyManager>, // Assuming TimeHierarchyManager is Arc-wrappable
}

impl SnapshotManager {
    /// Creates a new `SnapshotManager`.
    ///
    /// # Arguments
    /// * `config` - Shared application configuration.
    /// * `storage` - Shared storage backend instance.
    /// * `time_manager` - Shared time hierarchy manager instance.
    pub fn new(
        config: Arc<Config>,
        storage: Arc<dyn StorageBackend + Send + Sync>,
        time_manager: Arc<TimeHierarchyManager>,
    ) -> Self {
        SnapshotManager {
            config,
            storage,
            time_manager,
        }
    }

    /// Creates a snapshot of the system state or specified containers as of a given timestamp.
    /// 
    /// # Arguments
    /// * `as_of_timestamp` - The point-in-time for which to capture the state.
    /// * `container_ids` - An optional list of container IDs to include in the snapshot.
    ///   If `None`, all containers will be included.
    ///
    /// # Returns
    /// The page hash of the newly created snapshot page, or a `SnapshotError`.
    pub async fn create_snapshot(
        &self,
        as_of_timestamp: DateTime<Utc>,
        container_ids: Option<Vec<String>>,
    ) -> Result<[u8; 32], SnapshotError> {
        if !self.config.snapshot.enabled {
            return Err(SnapshotError::ConfigError("Snapshot feature is not enabled.".to_string()));
        }

        log::info!(
            "[SnapshotManager] Initiating snapshot creation for as_of_timestamp: {}, containers: {:?}",
            as_of_timestamp, container_ids
        );

        // 1. Determine Journal Scope (finalized pages + active L0 leaves)
        let mut finalized_pages: Vec<JournalPage> = Vec::new();
        let num_levels = self.config.time_hierarchy.levels.len();

        for level_idx in 0..num_levels {
            let level = level_idx as u8;
            log::debug!("[SnapshotManager] Fetching finalized pages for level {}", level);
            match self.storage.list_finalized_pages_summary(level).await {
                Ok(summaries) => {
                    for summary in summaries {
                        // Check if the page's time window starts before or at the snapshot timestamp.
                        // Note: A page's `creation_timestamp` marks the beginning of its time window.
                        // We are interested in pages that *could* contain data relevant to the snapshot time.
                        // A more precise filter might also consider `summary.end_time > as_of_timestamp`
                        // if we only want pages that strictly bracket or precede the snapshot time.
                        // For now, including pages that start before or at `as_of_timestamp` is a safe bet.
                        if summary.creation_timestamp <= as_of_timestamp {
                            match self.storage.load_page(level, summary.page_id).await {
                                Ok(Some(page)) => {
                                    finalized_pages.push(page);
                                }
                                Ok(None) => {
                                    log::warn!(
                                        "[SnapshotManager] Page summary found for L{}/{} but page data not found. Skipping.",
                                        level, summary.page_id
                                    );
                                }
                                Err(e) => {
                                    log::error!(
                                        "[SnapshotManager] Error loading page L{}/{}: {}. Skipping.",
                                        level, summary.page_id, e
                                    );
                                    // Optionally, could return an error here if any page load fails
                                    // return Err(SnapshotError::StorageError(format!("Failed to load page L{}/{}: {}", level, summary.page_id, e)));
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!(
                        "[SnapshotManager] Error listing finalized page summaries for level {}: {}. Skipping level.",
                        level, e
                    );
                    // Optionally, could return an error here
                    // return Err(SnapshotError::StorageError(format!("Failed to list summaries for L{}: {}", level, e)));
                }
            }
        }
        log::debug!("[SnapshotManager] Retrieved {} finalized pages relevant to snapshot as_of_timestamp: {}.", finalized_pages.len(), as_of_timestamp);

        // Get a consistent read of active L0 page data via TimeHierarchyManager.
        let active_l0_leaves = match self.time_manager.get_active_l0_leaves_up_to(as_of_timestamp).await {
            Ok(leaves) => {
                log::debug!("[SnapshotManager] Retrieved {} active L0 leaves up to {}.", leaves.len(), as_of_timestamp);
                leaves
            }
            Err(e) => {
                log::error!("[SnapshotManager] Error retrieving active L0 leaves: {}. Proceeding without them.", e);
                // Depending on policy, we might want to return an error here instead of proceeding.
                // For now, proceed with an empty set, which might lead to an incomplete snapshot if active L0 data is critical.
                return Err(SnapshotError::TimeManagerError(format!("Failed to get active L0 leaves: {}", e)));
            }
        };

        // 2. State Reconstruction for each container

        // Helper function to apply deltas
        fn apply_delta_to_state(state: &mut HashMap<String, serde_json::Value>, delta: &serde_json::Value) {
            if let serde_json::Value::Object(delta_map) = delta {
                for (key, value) in delta_map {
                    if value.is_null() {
                        state.remove(key);
                    } else {
                        state.insert(key.clone(), value.clone());
                    }
                }
            }
        }

        let target_container_ids: HashSet<String> = {
            if let Some(ref ids) = container_ids {
                ids.iter().cloned().collect()
            } else {
                let mut all_ids = HashSet::new();
                for page in &finalized_pages {
                    match &page.content {
                        PageContent::Leaves(leaves) => {
                            for leaf in leaves {
                                if leaf.timestamp <= as_of_timestamp { // Only consider leaves up to snapshot time
                                    all_ids.insert(leaf.container_id.clone());
                                }
                            }
                        }
                        PageContent::NetPatches(patches_map) => {
                             // NetPatches page's end_time should be <= as_of_timestamp
                            if page.end_time <= as_of_timestamp {
                                for container_id_in_patch in patches_map.keys() {
                                    all_ids.insert(container_id_in_patch.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                for leaf in &active_l0_leaves {
                     if leaf.timestamp <= as_of_timestamp { // Only consider leaves up to snapshot time
                        all_ids.insert(leaf.container_id.clone());
                    }
                }
                all_ids
            }
        };

        if target_container_ids.is_empty() && container_ids.is_none() {
            log::debug!("[SnapshotManager] No active containers found to snapshot based on available data up to {}.", as_of_timestamp);
        }

        let mut snapshot_container_states: Vec<SnapshotContainerState> = Vec::new();

        for container_id_str in target_container_ids {
            log::debug!("[SnapshotManager] Reconstructing state for container: {}", container_id_str);
            let mut current_container_state_map: HashMap<String, serde_json::Value> = HashMap::new();
            
            let mut container_specific_leaves: Vec<JournalLeaf> = Vec::new();
            for page in &finalized_pages {
                if let PageContent::Leaves(leaves) = &page.content {
                    for leaf in leaves {
                        if leaf.container_id == container_id_str && leaf.timestamp <= as_of_timestamp {
                            container_specific_leaves.push(leaf.clone());
                        }
                    }
                }
            }
            for leaf in &active_l0_leaves {
                if leaf.container_id == container_id_str && leaf.timestamp <= as_of_timestamp {
                    container_specific_leaves.push(leaf.clone());
                }
            }
            container_specific_leaves.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then_with(|| a.leaf_id.cmp(&b.leaf_id)));

            let mut actual_last_leaf_timestamp = None;
            let mut actual_last_leaf_hash = None;
            if let Some(last_leaf) = container_specific_leaves.last() {
                actual_last_leaf_timestamp = Some(last_leaf.timestamp);
                actual_last_leaf_hash = Some(last_leaf.leaf_hash);
            }

            let mut baseline_applied_at_timestamp = DateTime::<Utc>::MIN_UTC;
            let mut sorted_finalized_pages_for_netpatch = finalized_pages.clone();
            sorted_finalized_pages_for_netpatch.sort_by(|a, b| b.end_time.cmp(&a.end_time)); // Latest end_time first

            for page in &sorted_finalized_pages_for_netpatch {
                if page.end_time <= as_of_timestamp {
                    if let PageContent::NetPatches(patches_map) = &page.content {
                        if let Some(patches_for_container) = patches_map.get(&container_id_str) {
                            current_container_state_map.clear(); // Start from this net patch baseline
                            for (field, value) in patches_for_container {
                                if value.is_null() {
                                    current_container_state_map.remove(field);
                                } else {
                                    current_container_state_map.insert(field.clone(), value.clone());
                                }
                            }
                            baseline_applied_at_timestamp = page.end_time;
                            break; 
                        }
                    }
                }
            }

            for leaf in &container_specific_leaves {
                if leaf.timestamp > baseline_applied_at_timestamp && leaf.timestamp <= as_of_timestamp {
                    apply_delta_to_state(&mut current_container_state_map, &leaf.delta_payload);
                }
            }
            
            let state_payload_vec = match serde_json::to_vec(&current_container_state_map) {
                Ok(vec) => vec,
                Err(e) => {
                    log::error!("[SnapshotManager] Failed to serialize state for container {}: {}. Skipping.", container_id_str, e);
                    continue; 
                }
            };

            // If no leaves were found for this container at all, but it was requested or discovered (e.g. via an empty NetPatch entry),
            // then actual_last_leaf_timestamp/hash will be None. Use fallbacks.
            let final_ts = actual_last_leaf_timestamp.unwrap_or_else(|| {
                // If state exists (from netpatch) but no leaves, use baseline_applied_at_timestamp if it's not MIN_UTC
                if !current_container_state_map.is_empty() && baseline_applied_at_timestamp != DateTime::<Utc>::MIN_UTC {
                    baseline_applied_at_timestamp
                } else {
                    as_of_timestamp // Ultimate fallback: state is as of snapshot time, details unknown
                }
            });
            let final_hash = actual_last_leaf_hash.unwrap_or_else(|| [0u8; 32]); // Fallback zero hash

            snapshot_container_states.push(SnapshotContainerState {
                container_id: container_id_str.clone(),
                state_payload: state_payload_vec,
                last_leaf_hash_applied: final_hash,
                last_leaf_timestamp_applied: final_ts,
            });
        }

        let container_states = snapshot_container_states;

        // 3. Snapshot Data Assembly

        // Calculate actual container_states_merkle_root.
        let container_state_hashes: Vec<[u8; 32]> = container_states.iter().map(|cs| {
            let mut hasher = sha2::Sha256::new();
            hasher.update(cs.container_id.as_bytes());
            hasher.update(&cs.state_payload);
            hasher.update(&cs.last_leaf_hash_applied);
            hasher.update(&cs.last_leaf_timestamp_applied.timestamp_micros().to_le_bytes());
            let result: [u8; 32] = hasher.finalize().into();
            result
        }).collect();

        let container_states_merkle_root = if container_state_hashes.is_empty() {
            [0u8; 32] // Default empty Merkle root
        } else {
            MerkleTree::new(container_state_hashes)
                .map_err(|e| SnapshotError::InternalError(format!("Failed to build Merkle tree for container states: {}", e)))?
                .get_root()
                .unwrap_or([0u8; 32]) // Should not happen if tree construction succeeded with items
        };

        // Determine preceding_journal_page_hash (from regular journal levels).
        let mut potential_preceding_pages = finalized_pages.clone(); // finalized_pages are already filtered by as_of_timestamp for their start
        // Further filter to ensure their end_time is also <= as_of_timestamp for preceding link
        potential_preceding_pages.retain(|p| p.end_time <= as_of_timestamp);
        potential_preceding_pages.sort_by(|a, b| 
            b.end_time.cmp(&a.end_time) // Primary: latest end_time
            .then_with(|| b.level.cmp(&a.level)) // Secondary: highest level
            .then_with(|| b.page_id.cmp(&a.page_id)) // Tertiary: highest page_id
        );
        let preceding_journal_page_hash = potential_preceding_pages.first().map(|p| p.page_hash);

        // Determine previous_snapshot_page_hash_on_snapshot_level.
        let snapshot_dedicated_level = self.config.snapshot.dedicated_level;
        let previous_snapshot_page_hash_on_snapshot_level = 
            match self.storage.list_finalized_pages_summary(snapshot_dedicated_level).await {
                Ok(mut snapshot_summaries) => {
                    snapshot_summaries.sort_by(|a, b| b.creation_timestamp.cmp(&a.creation_timestamp) // Most recent first
                                            .then_with(|| b.page_id.cmp(&a.page_id)));
                    snapshot_summaries.first().map(|s| s.page_hash)
                }
                Err(e) => {
                    log::warn!("[SnapshotManager] Could not list previous snapshots on level {}: {}. Assuming no previous snapshot.", snapshot_dedicated_level, e);
                    None
                }
            };

        let created_at_timestamp = Utc::now();

        let snapshot_payload = SnapshotPagePayload {
            snapshot_id: uuid::Uuid::new_v4().to_string(), // Generate a unique ID
            as_of_timestamp, // Function argument
            container_states, // Populated earlier
            preceding_journal_page_hash, // Populated from finalized journal pages
            previous_snapshot_page_hash_on_snapshot_level, // Populated from previous snapshots on SNAPSHOT_LEVEL
            container_states_merkle_root, // Calculated earlier
            created_at_timestamp, // Timestamp for this specific payload creation
        };

        // 4. Storing the Snapshot
        let mut snapshot_journal_page = JournalPage::new(
            snapshot_dedicated_level, // Use the dedicated snapshot level from config
            previous_snapshot_page_hash_on_snapshot_level, // Link to previous snapshot on this level
            created_at_timestamp, // The creation time of this snapshot page (same as payload's for consistency)
            &self.config,
        );

        // Set the content of the page to be the snapshot payload.
        // JournalPage::new initializes with empty content based on level and config.
        // We need to explicitly set it for snapshots.
        snapshot_journal_page.content = PageContent::Snapshot(snapshot_payload.clone()); 
        // The page's merkle_root and page_hash are calculated based on its *entire* content, including the snapshot_payload.
        // The container_states_merkle_root is *within* the snapshot_payload.
        snapshot_journal_page.recalculate_merkle_root_and_page_hash(); // Ensure hashes are up-to-date with new content

        match self.storage.store_page(&snapshot_journal_page).await {
            Ok(()) => {
                log::info!(
                    "[SnapshotManager] Successfully created and stored snapshot ID: {} with page hash: {:?}",
                    snapshot_payload.snapshot_id,
                    snapshot_journal_page.page_hash
                );
                Ok(snapshot_journal_page.page_hash)
            }
            Err(e) => {
                log::error!(
                    "[SnapshotManager] Failed to store snapshot page for ID {}: {}",
                    snapshot_payload.snapshot_id, e
                );
                Err(SnapshotError::StorageError(format!(
                    "Failed to store snapshot page: {}",
                    e
                )))
            }
        }
    }
}
