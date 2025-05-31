// src/core/time_manager.rs

use std::sync::Arc;
use chrono::{DateTime, Utc};
use dashmap::DashMap;

use crate::config::Config;
use crate::storage::StorageBackend;
use crate::core::leaf::JournalLeaf;
use crate::core::page::{JournalPage, PageContentHash};
use crate::error::CJError;

// TODO: Potentially move TimeHierarchyConfig and RollupConfig to a more central types location
// if they are heavily used by the manager and core logic outside of just config.

/// Manages the lifecycle of journal pages across different time hierarchy levels.
///
/// This includes creating new pages for incoming leaves, rolling up pages from
/// lower levels to higher levels, and retrieving pages.
#[derive(Debug)]
/// Manages the time-based hierarchy of journal pages.
///
/// Responsible for adding leaves, creating/finalizing pages, and coordinating with storage.
pub struct TimeHierarchyManager {
    config: Arc<Config>,
    storage: Arc<dyn StorageBackend>,
    /// Stores the currently active (open for writing) page for each level.
    /// Key: level (u8), Value: JournalPage
    /// This helps avoid repeatedly loading the same page from storage if many leaves
    /// are added to it sequentially.
    active_pages: Arc<DashMap<u8, JournalPage>>,
    last_finalized_page_ids: Arc<DashMap<u8, u64>>, // Tracks IDs for potential loading, debugging, or specific lookup needs
    last_finalized_page_hashes: Arc<DashMap<u8, [u8; 32]>>, // Tracks actual hashes for chaining
}

impl TimeHierarchyManager {
    /// Creates a new `TimeHierarchyManager`.
    ///
    /// # Arguments
    /// * `config` - The application configuration, wrapped in an `Arc`.
    /// * `storage` - The storage backend, wrapped in an `Arc`.
    pub fn new(config: Arc<Config>, storage: Arc<dyn StorageBackend>) -> Self {
        // TODO: Initialize active_pages by trying to load the latest open page for each level
        // from storage. This is important for resuming operations after a restart.
        TimeHierarchyManager {
            config,
            storage,
            active_pages: Arc::new(DashMap::new()),
            last_finalized_page_ids: Arc::new(DashMap::new()),
            last_finalized_page_hashes: Arc::new(DashMap::new()),
        }
    }

    /// Calculates the start and end times for a page window at a given level, containing the given timestamp.
    fn get_page_window(&self, level_idx: usize, timestamp: DateTime<Utc>) -> Result<(DateTime<Utc>, DateTime<Utc>), CJError> {
        let level_config = self.config.time_hierarchy.levels.get(level_idx)
            .ok_or_else(|| CJError::ConfigError(crate::config::ConfigError::ValidationError(format!("Time hierarchy level {} not configured.", level_idx))))?;

        let granularity_duration = level_config.duration();

        if granularity_duration.as_nanos() == 0 {
            return Err(CJError::ConfigError(crate::config::ConfigError::InvalidValue {
                field: format!("time_hierarchy.levels[{}].duration_seconds", level_idx),
                value: "0".to_string(),
                reason: "granularity cannot be zero".to_string(),
            }));
        }

        let ts_nanos = timestamp.timestamp_nanos_opt().unwrap_or_default();
        let gran_nanos = granularity_duration.as_nanos() as i64;
        let window_start_nanos = (ts_nanos / gran_nanos) * gran_nanos;
        
        let start_time = DateTime::from_timestamp_nanos(window_start_nanos);
        let end_time = start_time + granularity_duration;

        Ok((start_time, end_time))
    }

    /// Gets the current active page for level 0 for the given leaf, or creates a new one.
    /// This is a simplified version; a more robust impl would check storage for existing pages
    /// that fit the leaf's timestamp if no active page is found or if the active page is not suitable.
    async fn get_or_create_active_page_for_leaf(&self, leaf: &JournalLeaf) -> Result<JournalPage, CJError> {
    println!("[GOCAPL] Called for leaf ts: {}, level: 0", leaf.timestamp);
        let level_idx = 0; // Base level for leaves
        let (window_start, _window_end) = self.get_page_window(level_idx, leaf.timestamp)?;

        if let Some(active_page_entry_ref) = self.active_pages.get_mut(&(level_idx as u8)) {
        let active_page_entry = active_page_entry_ref.value(); // Get a reference to JournalPage
        println!("[GOCAPL] Found active page L{}P{} (created: {}, ends: {}, leaves: {}). Leaf ts: {}.", 
                 active_page_entry.level, active_page_entry.page_id, 
                 active_page_entry.creation_timestamp, active_page_entry.end_time, 
                 active_page_entry.content_hashes.len(), leaf.timestamp);
            // Check if the current active page is suitable for this leaf
            let is_in_window = leaf.timestamp >= active_page_entry.creation_timestamp && leaf.timestamp < active_page_entry.end_time;
        println!("[GOCAPL] Timestamp check (leaf >= page_start && leaf < page_end): {}", is_in_window);
        if is_in_window {
                // Check if the page is not full
            let has_space = active_page_entry.content_hashes.len() < self.config.rollup.max_leaves_per_page;
            println!("[GOCAPL] Space check (leaves < max_leaves): {}", has_space);
            if has_space {
                println!("[GOCAPL] Reusing (from active_pages) page L{}P{}.", active_page_entry.level, active_page_entry.page_id);
                return Ok(active_page_entry.clone());
            }
                // If full, do not return this page; proceed to create a new one.
                // The caller (add_leaf) will handle finalizing the full active page.
            }
        }

        // No suitable active page in memory. Try to load from storage or create a new one.
        let level_config = self.config.time_hierarchy.levels.get(level_idx)
            .ok_or_else(|| CJError::ConfigError(crate::config::ConfigError::ValidationError(format!("Time hierarchy level {} not configured.", level_idx))))?;

        let mut potential_prev_page_hash: Option<[u8; 32]> = None;

        if let Some(last_page_id_ref) = self.last_finalized_page_ids.get(&(level_idx as u8)) {
            let last_page_id = *last_page_id_ref.value();
            // Attempt to load the last known finalized page for this level
            if let Some(loaded_page) = self.storage.load_page(level_idx as u8, last_page_id).await? {
                // A page was loaded. Its hash is a candidate for the new page's prev_hash if we end up creating one.
                potential_prev_page_hash = Some(loaded_page.page_hash);

                // Check if this loaded page can be reused for the current leaf.
                // 1. Time window must match the calculated window for the current leaf.
                // 2. Page must not be full.
                if loaded_page.creation_timestamp == window_start &&
                   loaded_page.end_time == (window_start + level_config.duration()) && // Ensure window matches calculated one
                   (loaded_page.content_hashes.len()) < self.config.rollup.max_leaves_per_page {
                    
                    // Page is suitable for reuse.
                    // The caller (add_leaf) will handle inserting this into active_pages after modification.
                    println!("[GOCAPL] Reusing active page L{}P{}.", loaded_page.level, loaded_page.page_id);
            return Ok(loaded_page); 
                }
                // If not reusable, potential_prev_page_hash is already set from this loaded page's hash.
                // We'll proceed to create a new page, linking to this one.
            }
        }

        // If we reach here, either:
        // - No last finalized page ID was found for this level.
        // - Loading the last finalized page failed (e.g., Ok(None) from storage).
        // - The loaded page was not suitable for reuse (wrong window or full).
        // So, create a new page.
        let mut prev_hash_for_new_page_opt: Option<[u8; 32]> = potential_prev_page_hash;

        if prev_hash_for_new_page_opt.is_none() {
            if let Some(last_finalized_id_ref) = self.last_finalized_page_ids.get(&(level_idx as u8)) {
                let last_finalized_id = *last_finalized_id_ref.value();
                // Attempt to load this last finalized page to get its hash
                if let Some(last_finalized_page) = self.storage.load_page(level_idx as u8, last_finalized_id).await? {
                    prev_hash_for_new_page_opt = Some(last_finalized_page.page_hash);
                }
            }
        }

        let new_page = JournalPage::new(
            level_idx as u8,
            prev_hash_for_new_page_opt.map(PageContentHash::ThrallPageHash),
            window_start, // time_window_start for the new page should be the calculated window_start
            leaf.timestamp, // current_time for creation_timestamp, now passed as leaf_timestamp
            &self.config // Pass the main config
        );
        println!("[GOCAPL] Proceeding to create new page L{}P{}.", new_page.level, new_page.page_id);
        println!("[GOCAPL] New page creation timestamp: {}", new_page.creation_timestamp);
        Ok(new_page)
    }

    /// Gets or creates an active page for a parent level during rollup.
    async fn get_or_create_active_parent_page(&self, parent_level_idx: u8, triggering_timestamp: DateTime<Utc>) -> Result<JournalPage, CJError> {
        let (window_start, _window_end) = self.get_page_window(parent_level_idx as usize, triggering_timestamp)?;

        if let Some(active_page_entry) = self.active_pages.get_mut(&parent_level_idx) {
            if triggering_timestamp >= active_page_entry.creation_timestamp && triggering_timestamp < active_page_entry.end_time {
                // Found an active page for this level and time window.
                // Check if it's full.
                let _level_config = self.config.time_hierarchy.levels.get(parent_level_idx as usize)
                    .ok_or_else(|| CJError::ConfigError(crate::config::ConfigError::ValidationError(format!("Time hierarchy level {} not configured.", parent_level_idx))))?;
                
                // For parent pages, max_items is from rollup config, not per-level config (which might not exist or apply here)
                if active_page_entry.content_hashes.len() < self.config.rollup.max_leaves_per_page {
                    return Ok(active_page_entry.value().clone());
                }
                // If full, we'll fall through to create a new one, and the full one should have been finalized by a previous step.
            }
        }

        // No suitable active page, or it was full. Create a new one.
        let prev_hash_for_new_page_opt: Option<[u8; 32]> = self.last_finalized_page_hashes.get(&parent_level_idx).map(|hash_ref| *hash_ref.value());

        let new_page = JournalPage::new(
            parent_level_idx,
            prev_hash_for_new_page_opt.map(PageContentHash::ThrallPageHash),
            window_start, // Page starts at the beginning of its calculated window
            triggering_timestamp, // Creation timestamp is the trigger time (e.g., end_time of child)
            &self.config,
        );
        Ok(new_page)
    }

    /// Adds a journal leaf to the appropriate time page.
    ///
    /// This method handles page creation, leaf addition to active pages,
    /// and page finalization when a page is full or a new time window starts.
    ///
    /// # Arguments
    /// * `leaf` - A reference to the `JournalLeaf` to be added.
    pub async fn add_leaf(&self, leaf: &JournalLeaf) -> Result<(), CJError> {
    println!("[ADD_LEAF] Called for leaf ts: {}, level: 0", leaf.timestamp);
        let mut page = self.get_or_create_active_page_for_leaf(leaf).await?;

        // Add leaf hash to page
        page.add_content_hash(PageContentHash::LeafHash(leaf.leaf_hash), leaf.timestamp);
        page.recalculate_merkle_root_and_page_hash(); // Recalculate after adding content

        // level_config was previously fetched here but not used directly.
        // Max items logic now uses self.config.rollup.max_leaves_per_page
        // Page window/granularity logic is based on page.start_time and page.end_time or derived from get_page_window.
        // If specific level configuration (e.g. custom properties beyond duration) is needed in the future,
        // it would be re-introduced here.
        self.config.time_hierarchy.levels.get(page.level as usize)
            .ok_or_else(|| CJError::ConfigError(crate::config::ConfigError::ValidationError(format!("Time hierarchy level {} not configured.", page.level))))?;

        let mut should_finalize = false;
        let max_items = self.config.rollup.max_leaves_per_page;
        if page.content_hashes.len() >= max_items && !page.content_hashes.is_empty() {
            should_finalize = true;
        }
        // A page is also considered "finalized" for adding new leaves if the current leaf's timestamp
        // would fall outside the page's designated time window. This is implicitly handled by
        // get_or_create_active_page_for_leaf creating a *new* page if the leaf doesn't fit the current active one.
        // However, an explicit check against page.end_time might be useful if a page is kept active.

        if leaf.timestamp >= page.end_time { // This condition means leaf belongs to a *new* page window
            // This case should ideally be handled by get_or_create_active_page_for_leaf returning a new page.
            // If we reach here with the *same* page object, it implies an issue in page selection logic.
            // For safety, we can treat it as a finalization trigger for the *current* page if it has content.
            if !page.content_hashes.is_empty() { // Only finalize if it has some content
                 should_finalize = true;
            }
        }

        // Update the active page in the map
        println!("[ADD_LEAF] Page L{}P{} has {} leaves. Finalize: {}.", page.level, page.page_id, page.content_hashes.len(), should_finalize);
    self.active_pages.insert(page.level, page.clone());
    println!("[ADD_LEAF] Inserted/Updated L{}P{} in active_pages.", page.level, page.page_id);

        if should_finalize {
            let page_to_store = page.clone();
            println!("[GOCAPL] Found active page L{}P{} with {} leaves.", page_to_store.level, page_to_store.page_id, page_to_store.content_hashes.len());
            self.storage.store_page(&page_to_store).await?;
            self.last_finalized_page_ids.insert(page_to_store.level, page_to_store.page_id);
            self.last_finalized_page_hashes.insert(page_to_store.level, page_to_store.page_hash);
            self.active_pages.remove(&(page.level));
        println!("[ADD_LEAF] Removed L{}P{} from active_pages due to finalization.", page.level, page.page_id);
            self.perform_rollup(page_to_store.level, page_to_store.page_hash, page_to_store.end_time).await?;
        } else {
            // Page is not yet full/finalized, it remains in active_pages for further leaf additions.
            // No immediate storage write unless a specific strategy dictates otherwise (e.g. periodic flush).
        }

        println!("[ADD_LEAF] Finished for leaf ts: {}. Active pages for L0: {:?}", leaf.timestamp, self.active_pages.get(&0u8).map(|p| (p.value().page_id, p.value().content_hashes.len())));
        Ok(())
    }

    /// Retrieves a specific `JournalPage` from storage.
    ///
    /// # Arguments
    /// * `level` - The hierarchy level of the page.
    /// * `page_id` - The ID of the page to retrieve.
    pub async fn get_page(&self, level: u8, page_id: u64) -> Result<Option<JournalPage>, CJError> {
        // First, check active pages, as it might be there and not yet persisted (or is the latest version)
        if let Some(active_page_entry) = self.active_pages.get(&level) {
            if active_page_entry.page_id == page_id {
                return Ok(Some(active_page_entry.value().clone()));
            }
        }
        // If not in active_pages or different ID, try loading from persistent storage
        self.storage.load_page(level, page_id).await
    }

    async fn perform_rollup(&self, finalized_page_level: u8, finalized_page_hash: [u8; 32], finalized_page_end_time: DateTime<Utc>) -> Result<(), CJError> {
        let parent_level_idx = finalized_page_level + 1;

        // Check if the parent level exists in the configuration
        if self.config.time_hierarchy.levels.get(parent_level_idx as usize).is_none() {
            // Finalized page was at the highest configured level, so no further rollup.
            return Ok(());
        }

        let mut parent_page = self.get_or_create_active_parent_page(parent_level_idx, finalized_page_end_time).await?;
        parent_page.add_content_hash(PageContentHash::ThrallPageHash(finalized_page_hash), finalized_page_end_time);
        parent_page.recalculate_merkle_root_and_page_hash();
        // Determine if this parent page itself needs to be finalized.
        let mut should_finalize_parent = false;
        let max_items_parent = self.config.rollup.max_leaves_per_page;
        if parent_page.content_hashes.len() >= max_items_parent && !parent_page.content_hashes.is_empty() {
            should_finalize_parent = true;
        }

        // Check max_page_age_seconds for the parent page.
        // The 'age' is relative to its own creation_timestamp and the timestamp of the latest content added (finalized_page_end_time).
        let parent_page_age_seconds = (finalized_page_end_time - parent_page.creation_timestamp).num_seconds();
        if parent_page_age_seconds >= self.config.rollup.max_page_age_seconds as i64 && !parent_page.content_hashes.is_empty() {
            should_finalize_parent = true;
        }
        
        // Always update the active_pages map with the potentially modified parent page, unless it's being finalized immediately.
        if !should_finalize_parent {
            self.active_pages.insert(parent_level_idx, parent_page.clone());
        }

        if should_finalize_parent {
            let parent_page_to_store = parent_page.clone(); // Clone before moving to storage
            self.storage.store_page(&parent_page_to_store).await?;
            self.last_finalized_page_ids.insert(parent_level_idx, parent_page_to_store.page_id);
            self.last_finalized_page_hashes.insert(parent_level_idx, parent_page_to_store.page_hash);
            self.active_pages.remove(&parent_level_idx); // Remove from active as it's now finalized
            // Recursive call for the next level up
            Box::pin(self.perform_rollup(parent_level_idx, parent_page_to_store.page_hash, parent_page_to_store.end_time)).await?;
        }

        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use crate::types::time::{RollupConfig, TimeHierarchyConfig, TimeLevel};
    use crate::core::{SHARED_TEST_ID_MUTEX, reset_global_ids}; // Import shared test items
    use crate::storage::memory::MemoryStorage;
    use chrono::{Utc, Duration as ChronoDuration};
    use serde_json::json; // For creating delta_payload
    // Ensure std::sync::atomic::Ordering is imported if reset_global_ids needs it, or if it's used elsewhere.
    // It's used by reset_global_ids internally, so not needed here directly unless other code uses it.

    // Helper to create a default config for testing
    fn create_test_config() -> Config {
        Config {
            time_hierarchy: TimeHierarchyConfig {
                levels: vec![
                    TimeLevel { name: "seconds".to_string(), duration_seconds: 1 },
                    TimeLevel { name: "minutes".to_string(), duration_seconds: 60 },
                ],
            },
            rollup: RollupConfig {
                max_leaves_per_page: 2, // Small for easy testing of finalization
                max_page_age_seconds: 3600,
                force_rollup_on_shutdown: false,
            },
            storage: Default::default(),
            compression: Default::default(),
            logging: Default::default(),
            metrics: Default::default(),
            retention: Default::default(),
        }
    }

    // Helper to create a TimeHierarchyManager with MemoryStorage
    fn create_test_manager() -> (TimeHierarchyManager, Arc<MemoryStorage>) {
        let config = Arc::new(create_test_config());
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config, storage.clone());
        (manager, storage)
    }

    #[tokio::test]
    async fn test_add_leaf_to_new_page_and_check_active() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await; // Changed to .await for Tokio Mutex
        reset_global_ids();

        let (manager, storage) = create_test_manager();
        let now = Utc::now();

        // Add first leaf, page 0 (L0) should be created and then finalize immediately
        let leaf_for_375 = JournalLeaf::new(now, None, "container_for_375".to_string(), json!({"id_for_375":0}))
            .expect("Leaf creation for line 375 failed");
        manager.add_leaf(&leaf_for_375).await.expect("add_leaf for line 375 failed");
        // After first leaf, page 0 (L0) should be active but NOT finalized/stored.
        assert!(storage.load_page(0,0).await.expect("Storage query for L0P0 after 1st leaf failed").is_none(), "L0P0 should NOT be in storage after 1st leaf");
        let active_l0_page_after_first_leaf = manager.active_pages.get(&0u8).expect("Active L0 page should exist after 1st leaf").value().clone();
        assert_eq!(active_l0_page_after_first_leaf.page_id, 0, "Active L0 page ID should be 0 after 1st leaf");
        assert_eq!(active_l0_page_after_first_leaf.content_hashes.len(), 1, "Active L0 page should have 1 leaf after 1st leaf");
        assert_matches!(active_l0_page_after_first_leaf.content_hashes[0], PageContentHash::LeafHash(hash) if hash == leaf_for_375.leaf_hash);
        assert!(active_l0_page_after_first_leaf.prev_page_hash.is_none(), "L0P0 should not have a prev_page_hash");

        // Add a second leaf to L0, this should create page 1 (L0)
        let leaf2_time = now + chrono::Duration::milliseconds(100);
        let leaf_for_383 = JournalLeaf::new(leaf2_time, None, "container_for_383".to_string(), json!({"id_for_383":1}))
            .expect("Leaf creation for line 383 failed");
        manager.add_leaf(&leaf_for_383).await.expect("add_leaf for line 383 failed");
        
        // After second leaf, page 0 (L0) should be finalized and stored.
        let finalized_page_0 = storage.load_page(0,0).await.expect("Storage query for L0P0 after 2nd leaf failed").expect("L0P0 should be stored after 2nd leaf");
        assert_eq!(finalized_page_0.page_id, 0);
        assert_eq!(finalized_page_0.content_hashes.len(), 2, "Finalized L0P0 should have 2 leaves");

        // After L0P0 finalizes, there should be no active page for level 0.
        assert!(manager.active_pages.get(&0u8).is_none(), "No active L0 page should exist after L0P0 finalization");

        // A new L1 page (ID 1) should be active due to rollup.
        let active_l1_page_entry = manager.active_pages.get(&1u8).expect("Active L1 page should exist after L0P0 rollup");
        assert_eq!(active_l1_page_entry.value().page_id, 1, "Active L1 page ID should be 1");
        assert_eq!(active_l1_page_entry.value().content_hashes.len(), 1, "Active L1 page should have 1 thrall hash");
        assert_matches!(active_l1_page_entry.value().content_hashes[0], PageContentHash::ThrallPageHash(hash) if hash == finalized_page_0.page_hash);

        // Add a third leaf. This should create a new L0 page (ID 2).
        let leaf3_time = now + chrono::Duration::milliseconds(1000); // Ensure it's in a new 1s window if L0P0's window was tight
        let leaf3 = JournalLeaf::new(leaf3_time, None, "container_leaf3".to_string(), json!({"id":2}))
            .expect("Leaf3 creation failed");
        manager.add_leaf(&leaf3).await.expect("add_leaf for leaf3 failed");

        // A new L0 page (ID 2) should now be active.
        let active_l0_page_entry_after_leaf3 = manager.active_pages.get(&0u8)
            .expect("Active L0 page should exist after 3rd leaf");
        let active_l0_page_after_leaf3 = active_l0_page_entry_after_leaf3.value();
        assert_eq!(active_l0_page_after_leaf3.page_id, 2, "Newly active L0 page (after leaf3) should be ID 2");
        assert_eq!(active_l0_page_after_leaf3.content_hashes.len(), 1, "Active L0P2 should have 1 leaf");
        assert_matches!(active_l0_page_after_leaf3.content_hashes[0], PageContentHash::LeafHash(hash) if hash == leaf3.leaf_hash);
        assert_eq!(active_l0_page_after_leaf3.prev_page_hash, Some(PageContentHash::ThrallPageHash(finalized_page_0.page_hash)), "Prev hash of L0P2 should be hash of L0P0");
    }

    #[tokio::test]
    async fn test_single_rollup_max_items() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await; // Changed to .await for Tokio Mutex
        reset_global_ids();

        let (manager, storage) = create_test_manager();
        let timestamp1 = Utc::now();

        // Leaf 1
        let leaf1 = JournalLeaf::new(
            timestamp1,
            None,
            "rollup_container".to_string(),
            json!({ "data": "leaf1_for_rollup" })
        ).expect("Leaf1 creation failed in test_single_rollup_max_items");
        manager.add_leaf(&leaf1).await.expect("Failed to add leaf1");

        // At this point, page 0 (level 0) should be active but not full.
        let active_l0_page_after_leaf1 = manager.active_pages.get(&0u8).expect("L0 page not active after leaf1").clone();
        assert_eq!(active_l0_page_after_leaf1.page_id, 0);
        assert_eq!(active_l0_page_after_leaf1.content_hashes.len(), 1);
        assert!(storage.load_page(0,0).await.expect("Storage query failed for L0P0 (pre-finalization)").is_none(), "L0 page 0 should not be in storage yet (max_items test)");

        // Leaf 2 - this should trigger finalization of page 0 (level 0) and rollup
        let timestamp2 = timestamp1 + ChronoDuration::milliseconds(100); // Ensure slightly different timestamp if needed
        let leaf2 = JournalLeaf::new(
            timestamp2,
            None, // Assuming these are independent leaves for simplicity in this test
            "test_container".to_string(),
            json!({ "data": "leaf2_for_single_rollup" })
        ).expect("Leaf2 creation failed in test_single_rollup_max_items");
        manager.add_leaf(&leaf2).await.expect("Failed to add leaf2 for rollup in test_single_rollup_max_items");

        // L0 Page 0 (ID 0) should now be finalized and stored.
        let stored_l0_page = storage.load_page(0, 0).await
            .expect("Storage query failed for L0P0 (post-finalization) in max_items test")
            .expect("L0 Page 0 not found in storage after finalization (max_items test)");
        assert_eq!(stored_l0_page.page_id, 0, "Stored L0 page ID mismatch (max_items test)");
        assert_eq!(stored_l0_page.level, 0, "Stored L0 page level mismatch (max_items test)");
        assert_eq!(stored_l0_page.content_hashes.len(), 2, "Stored L0 page should have 2 leaves (max_items test)"); // leaf1 and leaf2
        assert_matches!(stored_l0_page.content_hashes[0], PageContentHash::LeafHash(hash) if hash == leaf1.leaf_hash);
        assert_matches!(stored_l0_page.content_hashes[1], PageContentHash::LeafHash(hash) if hash == leaf2.leaf_hash);

        // L0 Page 0 should no longer be active.
        assert!(manager.active_pages.get(&0u8).map_or(true, |p_entry| p_entry.value().page_id != 0), "L0 Page 0 should not be active after finalization (max_items test)");

        // L1 Page 1 (ID 1) should have been created by the rollup and be active.
        let active_l1_page_entry = manager.active_pages.get(&1u8)
            .expect("No active page found for level 1 after rollup (max_items test)");
        let active_l1_page = active_l1_page_entry.value();
        assert_eq!(active_l1_page.page_id, 1, "Active L1 page ID should be 1 (max_items test)");
        assert_eq!(active_l1_page.level, 1, "Active L1 page level should be 1 (max_items test)");
        assert_eq!(active_l1_page.content_hashes.len(), 1, "Active L1 page should have 1 thrall hash (max_items test)");
        assert_matches!(active_l1_page.content_hashes[0], PageContentHash::ThrallPageHash(hash) if hash == stored_l0_page.page_hash);
        assert_eq!(active_l1_page.prev_page_hash, None, "New L1 page should not have a prev_page_hash (max_items test)");

        // Check last finalized for L0
        let l0_finalized_id_entry = manager.last_finalized_page_ids.get(&0u8)
            .expect("No finalized ID for L0 after finalization (max_items test)");
        assert_eq!(*l0_finalized_id_entry.value(), 0, "Mismatch in last_finalized_page_id for L0 (max_items test)");
        
        let l0_finalized_hash_entry = manager.last_finalized_page_hashes.get(&0u8)
            .expect("No finalized hash for L0 after finalization (max_items test)");
        assert_eq!(*l0_finalized_hash_entry.value(), stored_l0_page.page_hash, "Mismatch in last_finalized_page_hash for L0 (max_items test)");

        // Add Leaf 3 - this should create a new L0 page (ID 2) that links to the finalized L0 Page 0
        let timestamp3 = timestamp2 + ChronoDuration::milliseconds(100);
        let leaf3 = JournalLeaf::new(
            timestamp3,
            None,
            "rollup_container".to_string(), // Can reuse container or use a new one
            json!({ "data": "leaf3_after_rollup" })
        ).expect("Leaf3 creation failed in test_single_rollup_max_items");
        manager.add_leaf(&leaf3).await.expect("Failed to add leaf3 after rollup");

        // L0 Page 2 (ID 2) should now be active.
        let active_l0_page_entry_after_leaf3 = manager.active_pages.get(&0u8)
            .expect("No active L0 page found after leaf3 (max_items test)");
        let active_l0_page_after_leaf3 = active_l0_page_entry_after_leaf3.value();
        assert_eq!(active_l0_page_after_leaf3.page_id, 2, "Active L0 page ID should be 2 after leaf3 (max_items test)");
        assert_eq!(active_l0_page_after_leaf3.content_hashes.len(), 1, "Active L0 page (ID 2) should have 1 leaf (max_items test)");
        assert_matches!(active_l0_page_after_leaf3.content_hashes[0], PageContentHash::LeafHash(hash) if hash == leaf3.leaf_hash);
        assert_eq!(active_l0_page_after_leaf3.prev_page_hash, Some(PageContentHash::ThrallPageHash(stored_l0_page.page_hash)), "prev_page_hash of L0 Page 2 does not match L0 Page 0 hash (max_items test)");
    }

    // Helper for cascading rollup tests
    fn create_cascading_test_config_and_manager() -> (TimeHierarchyManager, Arc<MemoryStorage>) {
        let mut config_val = create_test_config(); // Start with base config
        // Add a third level for cascading
        config_val.time_hierarchy.levels.push(TimeLevel { 
            name: "hours".to_string(), 
            duration_seconds: 3600 
        });
        // Set max_leaves_per_page to 1 to trigger finalization with a single item for cascade
        config_val.rollup.max_leaves_per_page = 1; 

        let config = Arc::new(config_val);
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config, storage.clone());
        (manager, storage)
    }

    #[tokio::test]
    async fn test_cascading_rollup_max_items() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await; // Changed to .await for Tokio Mutex
        reset_global_ids();

        let (manager, storage) = create_cascading_test_config_and_manager();
        let timestamp = Utc::now();

        // Leaf 1 - should trigger full cascade through L0, L1, L2
        let leaf1 = JournalLeaf::new(
            timestamp,
            None,
            "cascade_container".to_string(),
            json!({ "data": "leaf1_for_cascade" })
        ).expect("Leaf1 creation failed in test_cascading_rollup_max_items");
        manager.add_leaf(&leaf1).await.expect("Failed to add leaf1 for cascade");

        // L0P0 (ID 0) assertions
        let stored_l0_page = storage.load_page(0, 0).await.expect("Storage query failed for L0P0 in cascade").expect("L0P0 not found after cascade");
        assert_eq!(stored_l0_page.page_id, 0);
        assert_eq!(stored_l0_page.level, 0);
        assert_eq!(stored_l0_page.content_hashes.len(), 1);
        assert_matches!(stored_l0_page.content_hashes[0], PageContentHash::LeafHash(hash) if hash == leaf1.leaf_hash);

        // L1P1 (ID 1) assertions
        let stored_l1_page = storage.load_page(1, 1).await.expect("Storage query failed for L1P1 in cascade").expect("L1P1 not found after cascade");
        assert_eq!(stored_l1_page.page_id, 1);
        assert_eq!(stored_l1_page.level, 1);
        assert_eq!(stored_l1_page.content_hashes.len(), 1);
        assert_matches!(stored_l1_page.content_hashes[0], PageContentHash::ThrallPageHash(hash) if hash == stored_l0_page.page_hash);
        assert_eq!(stored_l1_page.prev_page_hash, None, "L1P1 should not have a prev_page_hash (first in its level)");

        // L2P2 (ID 2) assertions (highest level in this config)
        let stored_l2_page = storage.load_page(2, 2).await.expect("Storage query failed for L2P2 in cascade").expect("L2P2 not found after cascade");
        assert_eq!(stored_l2_page.page_id, 2);
        assert_eq!(stored_l2_page.level, 2);
        assert_eq!(stored_l2_page.content_hashes.len(), 1);
        assert_matches!(stored_l2_page.content_hashes[0], PageContentHash::ThrallPageHash(hash) if hash == stored_l1_page.page_hash);
        assert_eq!(stored_l2_page.prev_page_hash, None, "L2P2 should not have a prev_page_hash (first in its level)");
        
        // Check active pages are empty for L0, L1, L2 as all should have finalized
        assert!(manager.active_pages.get(&0u8).is_none(), "L0 should have no active page after cascade");
        assert!(manager.active_pages.get(&1u8).is_none(), "L1 should have no active page after cascade");
        assert!(manager.active_pages.get(&2u8).is_none(), "L2 should have no active page after cascade (top level finalized)");

        // Check last_finalized_page_ids and last_finalized_page_hashes
        assert_eq!(*manager.last_finalized_page_ids.get(&0u8).expect("L0 ID not in last_finalized_page_ids after cascade").value(), 0, "L0 last finalized ID mismatch");
        assert_eq!(*manager.last_finalized_page_hashes.get(&0u8).expect("L0 hash not in last_finalized_page_hashes after cascade").value(), stored_l0_page.page_hash, "L0 last finalized hash mismatch");
        
        assert_eq!(*manager.last_finalized_page_ids.get(&1u8).expect("L1 ID not in last_finalized_page_ids after cascade").value(), 1, "L1 last finalized ID mismatch");
        assert_eq!(*manager.last_finalized_page_hashes.get(&1u8).expect("L1 hash not in last_finalized_page_hashes after cascade").value(), stored_l1_page.page_hash, "L1 last finalized hash mismatch");

        assert_eq!(*manager.last_finalized_page_ids.get(&2u8).expect("L2 ID not in last_finalized_page_ids after cascade").value(), 2, "L2 last finalized ID mismatch");
        assert_eq!(*manager.last_finalized_page_hashes.get(&2u8).expect("L2 hash not in last_finalized_page_hashes after cascade").value(), stored_l2_page.page_hash, "L2 last finalized hash mismatch");
    }

}
