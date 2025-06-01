// src/core/time_manager.rs

use std::sync::Arc;
use chrono::{DateTime, Utc};
use dashmap::DashMap;

use crate::config::Config;
use crate::storage::StorageBackend;
use crate::core::leaf::JournalLeaf;
use crate::core::page::JournalPage;
use crate::error::CJError;
use crate::types::time::RollupRetentionPolicy;

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
    /// Optionally returns the page ID of the currently active page for a given level.
    /// Returns None if no page is currently active for that level.
    pub fn get_current_active_page_id(&self, level: u8) -> Option<u64> {
        self.active_pages.get(&level).map(|page_entry| page_entry.value().page_id)
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
                 active_page_entry.content_len(), leaf.timestamp);
            // Check if the current active page is suitable for this leaf
            let is_in_window = leaf.timestamp >= active_page_entry.creation_timestamp && leaf.timestamp < active_page_entry.end_time;
        println!("[GOCAPL] Timestamp check (leaf >= page_start && leaf < page_end): {}", is_in_window);
        if is_in_window {
                // Check if the page is not full
            let has_space = active_page_entry.content_len() < self.config.rollup.max_leaves_per_page;
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
                   (loaded_page.content_len()) < self.config.rollup.max_leaves_per_page {
                    
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
            level_idx as u8, // The level of the new page
            prev_hash_for_new_page_opt, // This is already Option<[u8; 32]>
            window_start, // time_window_start for the new page should be the calculated window_start
            &self.config, // Pass the main config
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
                if active_page_entry.content_len() < self.config.rollup.max_leaves_per_page {
                    return Ok(active_page_entry.value().clone());
                }
                // If full, we'll fall through to create a new one, and the full one should have been finalized by a previous step.
            }
        }

        // No suitable active page, or it was full. Create a new one.
        let prev_hash_for_new_page_opt: Option<[u8; 32]> = self.last_finalized_page_hashes.get(&parent_level_idx).map(|hash_ref| *hash_ref.value());

        let new_page = JournalPage::new(
            parent_level_idx, // The level for the new parent page
            prev_hash_for_new_page_opt, // This is already Option<[u8; 32]>
            window_start, // Page starts at the beginning of its calculated window
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
        page.add_leaf(leaf.clone());
        page.recalculate_merkle_root_and_page_hash(); // Recalculate after adding content

        let level_config = self.config.time_hierarchy.levels.get(page.level as usize)
            .ok_or_else(|| CJError::ConfigError(crate::config::ConfigError::ValidationError(format!("Time hierarchy level {} not configured.", page.level))))?;

        let mut should_finalize = false;
        let max_items = level_config.rollup_config.max_leaves_per_page;
        let max_age_seconds = level_config.rollup_config.max_page_age_seconds;

        // Check for item limit
        if !page.is_content_empty() && page.content_len() >= max_items {
            println!("[ADD_LEAF_FINALIZE_CHECK] Page L{}P{} met item limit ({} >= {})", page.level, page.page_id, page.content_len(), max_items);
            should_finalize = true;
        }

        // Check for age limit if not already finalized by item count
        if !should_finalize && !page.is_content_empty() && max_age_seconds > 0 { // Only check age if max_age is configured
            if page.creation_timestamp <= leaf.timestamp { // Ensure leaf.timestamp is not before page creation
                let page_age = (leaf.timestamp - page.creation_timestamp).num_seconds() as u64;
                if page_age >= max_age_seconds {
                    println!("[ADD_LEAF_FINALIZE_CHECK] Page L{}P{} met age limit ({}s >= {}s)", page.level, page.page_id, page_age, max_age_seconds);
                    should_finalize = true;
                }
            } else {
                // This case (leaf timestamp before page creation) should ideally not happen if page selection is correct.
                // Log it if it does, but don't finalize based on this anomalous condition.
                println!("[ADD_LEAF_FINALIZE_CHECK] Warning: Leaf timestamp {} is before page L{}P{} creation_timestamp {}. Age check skipped.", 
                         leaf.timestamp, page.level, page.page_id, page.creation_timestamp);
            }
        }

        // A page is also considered "finalized" for adding new leaves if the current leaf's timestamp
        // would fall outside the page's designated time window. This is implicitly handled by
        // get_or_create_active_page_for_leaf creating a *new* page if the leaf doesn't fit the current active one.
        // However, if the leaf *just* meets the end_time, it should finalize the current page.
        if !should_finalize && !page.is_content_empty() && leaf.timestamp >= page.end_time {
            println!("[ADD_LEAF_FINALIZE_CHECK] Page L{}P{} met end_time due to leaf ts {} >= page end_time {}", page.level, page.page_id, leaf.timestamp, page.end_time);
            should_finalize = true;
        }

        // Update the active page in the map
        println!("[ADD_LEAF] Page L{}P{} has {} leaves. Finalize: {}.", page.level, page.page_id, page.content_len(), should_finalize);
    self.active_pages.insert(page.level, page.clone());
    println!("[ADD_LEAF] Inserted/Updated L{}P{} in active_pages.", page.level, page.page_id);

        if should_finalize {
            let page_to_store = page.clone();
            println!("[GOCAPL] Found active page L{}P{} with {} leaves.", page_to_store.level, page_to_store.page_id, page_to_store.content_len());
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

        println!("[ADD_LEAF] Finished for leaf ts: {}. Active pages for L0: {:?}", leaf.timestamp, self.active_pages.get(&0u8).map(|p| (p.value().page_id, p.value().content_len())));
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

    /// Retrieves a specific `JournalPage` directly from the storage backend.
    ///
    /// This method bypasses any caching or active page management within the `TimeHierarchyManager`
    /// and directly queries the underlying storage.
    ///
    /// # Arguments
    /// * `level` - The hierarchy level of the page.
    /// * `page_id` - The ID of the page to retrieve.
    ///
    /// # Returns
    /// Returns a `Result` containing an `Option<JournalPage>`.
    /// `Ok(Some(page))` if the page is found.
    /// `Ok(None)` if the page is not found in storage.
    /// `Err(CJError::StorageError)` if there was an error accessing the storage.
    pub async fn get_page_from_storage(&self, level: u8, page_id: u64) -> Result<Option<JournalPage>, CJError> {
        self.storage.load_page(level, page_id).await.map_err(|e| {
            CJError::StorageError(format!(
                "Storage backend failed to load page L{}P{}: {}",
                level, page_id, e
            ))
        })
    }


    async fn perform_rollup(&self, finalized_page_level: u8, finalized_page_hash: [u8; 32], finalized_page_end_time: DateTime<Utc>) -> Result<(), CJError> {
        let parent_level_idx = finalized_page_level + 1;

        // Check if the parent level exists in the configuration
        if self.config.time_hierarchy.levels.get(parent_level_idx as usize).is_none() {
            // Finalized page was at the highest configured level, so no further rollup.
            return Ok(());
        }

        let mut parent_page = self.get_or_create_active_parent_page(parent_level_idx, finalized_page_end_time).await?;
        parent_page.add_thrall_hash(finalized_page_hash, finalized_page_end_time);
        parent_page.recalculate_merkle_root_and_page_hash();
        // Determine if this parent page itself needs to be finalized.
        let mut should_finalize_parent = false;
        let max_items_parent = self.config.rollup.max_leaves_per_page;
        if parent_page.content_len() >= max_items_parent && !parent_page.is_content_empty() {
            should_finalize_parent = true;
        }

        // Check max_page_age_seconds for the parent page.
        // The 'age' is relative to its own creation_timestamp and the timestamp of the latest content added (finalized_page_end_time).
        let parent_page_age_seconds = (finalized_page_end_time - parent_page.creation_timestamp).num_seconds();
        if parent_page_age_seconds >= self.config.rollup.max_page_age_seconds as i64 && !parent_page.is_content_empty() {
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

    /// Applies retention policies to delete old pages based on configuration.
    pub async fn apply_retention_policies(&self) -> Result<(), CJError> {
        let now = chrono::Utc::now();
        println!("[Retention] Applying per-level policies at {}.
", now);

        for (level_idx, level_config) in self.config.time_hierarchy.levels.iter().enumerate() {
            let level = level_idx as u8;
            println!("[Retention] Processing L{} ({})
", level, level_config.name);

            let effective_policy = match &level_config.retention_policy {
                Some(specific_policy) => {
                    println!("[Retention] L{}: Using specific policy: {:?}
", level, specific_policy);
                    specific_policy.clone()
                }
                None => {
                    if self.config.retention.enabled && self.config.retention.period_seconds > 0 {
                        println!("[Retention] L{}: No specific policy, using global: DeleteAfterSecs({}).
", level, self.config.retention.period_seconds);
                        RollupRetentionPolicy::DeleteAfterSecs(self.config.retention.period_seconds)
                    } else {
                        println!("[Retention] L{}: No specific policy, global retention disabled or period is 0. Defaulting to KeepIndefinitely.
", level);
                        RollupRetentionPolicy::KeepIndefinitely
                    }
                }
            };

            match effective_policy {
                RollupRetentionPolicy::KeepIndefinitely => {
                    println!("[Retention] L{}: Policy is KeepIndefinitely. No pages will be deleted.
", level);
                }
                RollupRetentionPolicy::DeleteAfterSecs(secs) => {
                    if secs == 0 {
                        println!("[Retention] L{}: Policy is DeleteAfterSecs(0). Skipping as this implies immediate deletion or misconfiguration.
", level);
                        continue;
                    }
                    let retention_duration = chrono::Duration::seconds(secs as i64);
                    println!("[Retention] L{}: Policy is DeleteAfterSecs({}s). Max age: {}.
", level, secs, retention_duration);

                    let page_summaries = match self.storage.list_finalized_pages_summary(level).await {
                        Ok(summaries) => summaries,
                        Err(e) => {
                            eprintln!("[Retention] L{}: Error listing finalized pages: {}. Skipping level.
", level, e);
                            continue;
                        }
                    };

                    if page_summaries.is_empty() {
                        println!("[Retention] L{}: No finalized pages found.
", level);
                        continue;
                    }
                    println!("[Retention] L{}: Found {} finalized pages.
", level, page_summaries.len());

                    for summary in page_summaries {
                        if summary.end_time > now {
                            println!("[Retention] L{}/{}: Skipping, end_time {} is in the future.
", level, summary.page_id, summary.end_time);
                            continue;
                        }
                        let page_age = now.signed_duration_since(summary.end_time);
                        if page_age > retention_duration {
                            println!("[Retention] L{}/{}: Deleting (end_time: {}, age: {}s)
", level, summary.page_id, summary.end_time, page_age.num_seconds());
                            if let Err(e) = self.storage.delete_page(level, summary.page_id).await {
                                eprintln!("[Retention] L{}/{}: Error deleting page: {}. Continuing.
", level, summary.page_id, e);
                            }
                        }
                    }
                }
                RollupRetentionPolicy::KeepNPages(n) => {
                    if n == 0 {
                        println!("[Retention] L{}: Policy is KeepNPages(0). Deleting all pages.
", level);
                        let page_summaries = match self.storage.list_finalized_pages_summary(level).await {
                            Ok(summaries) => summaries,
                            Err(e) => {
                                eprintln!("[Retention] L{}: Error listing for KeepNPages(0): {}. Skipping.
", level, e);
                                continue;
                            }
                        };
                        for summary in page_summaries {
                            println!("[Retention] L{}/{}: Deleting (KeepNPages(0))
", level, summary.page_id);
                            if let Err(e) = self.storage.delete_page(level, summary.page_id).await {
                                eprintln!("[Retention] L{}/{}: Error deleting page: {}. Continuing.
", level, summary.page_id, e);
                            }
                        }
                        continue;
                    }

                    println!("[Retention] L{}: Policy is KeepNPages({}).
", level, n);
                    let mut page_summaries = match self.storage.list_finalized_pages_summary(level).await {
                        Ok(summaries) => summaries,
                        Err(e) => {
                            eprintln!("[Retention] L{}: Error listing finalized pages: {}. Skipping level.
", level, e);
                            continue;
                        }
                    };

                    if page_summaries.len() <= n {
                        println!("[Retention] L{}: Found {} pages, which is <= {}. No pages will be deleted.
", level, page_summaries.len(), n);
                        continue;
                    }

                    page_summaries.sort_unstable_by_key(|s| s.end_time);

                    let num_to_delete = page_summaries.len() - n;
                    println!("[Retention] L{}: Found {} pages. Deleting {} oldest pages to keep {}.
", level, page_summaries.len(), num_to_delete, n);

                    for summary_to_delete in page_summaries.iter().take(num_to_delete) {
                        println!("[Retention] L{}/{}: Deleting (KeepNPages, end_time: {})
", level, summary_to_delete.page_id, summary_to_delete.end_time);
                        if let Err(e) = self.storage.delete_page(level, summary_to_delete.page_id).await {
                            eprintln!("[Retention] L{}/{}: Error deleting page: {}. Continuing.
", level, summary_to_delete.page_id, e);
                        }
                    }
                }
            }
        }
        println!("[Retention] Per-level policies application complete.
");
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::page::PageContent;
    use crate::types::time::{RollupConfig, TimeHierarchyConfig, TimeLevel};
    use crate::core::{SHARED_TEST_ID_MUTEX, reset_global_ids}; // Import shared test items
    use crate::storage::memory::MemoryStorage; // For creating an instance
    use std::sync::atomic::Ordering; // If used directly, otherwise covered by reset_global_ids
    use chrono::Duration;
    use serde_json::json; // ADDED for json! macro
    use crate::config::RetentionConfig; // ADDED for RetentionConfig struct

    // Helper to create a JournalPage for retention tests with a specific creation/end time logic
    // Note: This directly creates a JournalPage. For manager tests, you'd typically add leaves.
    // However, for retention, we need to control page end_times precisely.
    fn create_retention_test_page(level: u8, page_id_offset: u64, time_window_start: DateTime<Utc>, config: &Config) -> JournalPage {
        let original_next_id = crate::core::page::NEXT_PAGE_ID.load(Ordering::SeqCst);
        crate::core::page::NEXT_PAGE_ID.store(original_next_id + page_id_offset, Ordering::SeqCst);
        
        let page = JournalPage::new(
            level,
            None, // prev_page_hash
            time_window_start, // this determines end_time based on level duration
            config
        );
        // Restore NEXT_PAGE_ID if other tests depend on its sequential flow without manual setting like this
        // For isolated retention tests, this direct setting is fine if we reset_global_ids before each test block.
        // Better: ensure reset_global_ids() is called before any page creation in a test.
        page
    }

    // Helper to create a config for testing, allowing retention customization
    fn create_test_config(
        retention_enabled: bool, 
        retention_period_seconds: u64,
        l0_policy: Option<RollupRetentionPolicy>,
        l1_policy: Option<RollupRetentionPolicy>
    ) -> Config {
        Config {
            time_hierarchy: TimeHierarchyConfig {
                levels: vec![
                    TimeLevel {
                        rollup_config: RollupConfig {
                            max_leaves_per_page: 1,
                            max_page_age_seconds: 1000, // Match global test default
                            force_rollup_on_shutdown: false,
                        },
                        retention_policy: l0_policy,
                        name: "seconds".to_string(),
                        duration_seconds: 1
                    },
                    TimeLevel {
                        rollup_config: RollupConfig {
                            max_leaves_per_page: 1,
                            max_page_age_seconds: 1000, // Match global test default
                            force_rollup_on_shutdown: false,
                        },
                        retention_policy: l1_policy,
                        name: "minutes".to_string(),
                        duration_seconds: 60
                    },
                ],
            },
            rollup: RollupConfig { // Sensible defaults for most tests
                max_leaves_per_page: 1, // Corrected back to 1 for test consistency
                max_page_age_seconds: 1000,
                force_rollup_on_shutdown: false,
            },
            storage: Default::default(),
            retention: RetentionConfig {
                enabled: retention_enabled,
                period_seconds: retention_period_seconds,
                cleanup_interval_seconds: 300, // Default, not typically used in these specific tests
            },
            compression: Default::default(),
            logging: Default::default(),
            metrics: Default::default(),
        }
    }

    // Helper to create a TimeHierarchyManager with MemoryStorage
    fn create_test_manager() -> (TimeHierarchyManager, Arc<MemoryStorage>) {
        // Default to retention disabled for general tests not focused on retention
        let config = Arc::new(create_test_config(false, 0, None, None)); 
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config, storage.clone());
        (manager, storage)
    }

        #[tokio::test]
        async fn test_add_leaf_to_new_page_and_check_active() {
            let _guard = SHARED_TEST_ID_MUTEX.lock().await;
            reset_global_ids();
    
            let (manager, storage) = create_test_manager(); // Config from create_test_manager implies mlpp=1 for L0 & L1
            let now = Utc::now();
    
            // Leaf 1: L0P0 (ID 0) finalizes, L1P1 (ID 1) finalizes. No L2 configured by create_test_manager.
            let leaf1_ts = now;
            let leaf1 = JournalLeaf::new(leaf1_ts, None, "container1".to_string(), json!({"id":1})).unwrap();
            manager.add_leaf(&leaf1).await.unwrap();
    
            let stored_l0_p0_leaf1 = storage.load_page(0, 0).await.unwrap().expect("L0P0 (ID 0) should be in storage after 1st leaf");
            assert_eq!(stored_l0_p0_leaf1.page_id, 0);
            assert_eq!(stored_l0_p0_leaf1.content_len(), 1);
            match &stored_l0_p0_leaf1.content {
                PageContent::Leaves(leaves) => {
                    assert_eq!(leaves.len(), 1, "L0P0 (ID 0) content length mismatch");
                    assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0P0 (ID 0) leaf hash mismatch");
                }
                _ => panic!("Expected Leaves for stored_l0_p0_leaf1, found {:?}", stored_l0_p0_leaf1.content),
            }
            assert!(manager.active_pages.get(&0u8).is_none(), "No L0 page should be active after L0P0 finalization");
    
            let stored_l1_p1_leaf1 = storage.load_page(1, 1).await.unwrap().expect("L1P1 (ID 1) should be stored after L0P0 rollup and L1 finalization");
            assert_eq!(stored_l1_p1_leaf1.page_id, 1); // L1 page ID is 1
            assert_eq!(stored_l1_p1_leaf1.content_len(), 1, "Stored L1P1 should have 1 thrall hash from L0P0");
            match &stored_l1_p1_leaf1.content {
                PageContent::ThrallHashes(hashes) => {
                    assert_eq!(hashes.len(), 1, "L1P1 (ID 1) content length mismatch");
                    assert_eq!(hashes[0], stored_l0_p0_leaf1.page_hash, "L1P1 (ID 1) thrall hash mismatch");
                }
                _ => panic!("Expected ThrallHashes for stored_l1_p1_leaf1, found {:?}", stored_l1_p1_leaf1.content),
            }
            assert!(manager.active_pages.get(&1u8).is_none(), "No L1 page should be active after L1P1 finalization (L1 is highest configured level)");
            assert!(manager.active_pages.get(&2u8).is_none(), "No L2 page should exist as L2 is not configured by create_test_manager");
    
            // Leaf 2: Creates L0P2 (ID 2) because L0P0 (ID 0) already finalized. L0P2 finalizes. L1P3 (ID 3) created from L0P2 rollup, finalizes.
            let leaf2_ts = now + Duration::milliseconds(100); // now is from first leaf's setup
            let leaf2 = JournalLeaf::new(leaf2_ts, None, "container1".to_string(), json!({"id":2})).unwrap();
            manager.add_leaf(&leaf2).await.unwrap();
    
            // Check L0P0 (ID 0) - should still have only leaf1. stored_l0_p0_leaf1 is from the first leaf's assertions.
            assert_eq!(stored_l0_p0_leaf1.page_id, 0);
            assert_eq!(stored_l0_p0_leaf1.content_len(), 1, "L0P0 (ID 0) should still only have 1 leaf (leaf1)");
            match &stored_l0_p0_leaf1.content { // Re-assertion on L0P0 (ID 0)
                PageContent::Leaves(leaves) => {
                    assert_eq!(leaves.len(), 1, "L0P0 (ID 0) re-assertion content length mismatch");
                    assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0P0 (ID 0) re-assertion leaf hash mismatch");
                }
                _ => panic!("Expected Leaves for stored_l0_p0_leaf1 (re-assertion), found {:?}", stored_l0_p0_leaf1.content),
            }
    
            // Check L0P2 (ID 2) - created for leaf2
            let stored_l0_p2 = storage.load_page(0, 2).await.unwrap().expect("L0P2 (ID 2) should be in storage after leaf2");
            assert_eq!(stored_l0_p2.page_id, 2);
            assert_eq!(stored_l0_p2.content_len(), 1, "L0P2 (ID 2) should have 1 leaf (leaf2)");
            match &stored_l0_p2.content {
                PageContent::Leaves(leaves) => {
                    assert_eq!(leaves.len(), 1, "L0P2 (ID 2) content length mismatch");
                    assert_eq!(leaves[0].leaf_hash, leaf2.leaf_hash, "L0P2 (ID 2) leaf hash mismatch");
                }
                _ => panic!("Expected Leaves for stored_l0_p2, found {:?}", stored_l0_p2.content),
            }
            assert_eq!(stored_l0_p2.prev_page_hash, Some(stored_l0_p0_leaf1.page_hash), "L0P2 prev_page_hash should be L0P0's hash");
            assert!(manager.active_pages.get(&0u8).is_none(), "No L0 page should be active");
    
            // Check L1P1 (ID 1) - from L0P0 rollup. stored_l1_p1_leaf1 is from the first leaf's assertions (Page ID 1).
            assert_eq!(stored_l1_p1_leaf1.page_id, 1);
            assert_eq!(stored_l1_p1_leaf1.content_len(), 1, "Stored L1P1 (ID 1) should have 1 thrall hash from L0P0");
            match &stored_l1_p1_leaf1.content { // Re-assertion on L1P1 (ID 1)
                PageContent::ThrallHashes(hashes) => {
                    assert_eq!(hashes.len(), 1, "L1P1 (ID 1) re-assertion content length mismatch");
                    assert_eq!(hashes[0], stored_l0_p0_leaf1.page_hash, "L1P1 (ID 1) re-assertion thrall hash mismatch");
                }
                _ => panic!("Expected ThrallHashes for stored_l1_p1_leaf1 (re-assertion), found {:?}", stored_l1_p1_leaf1.content),
            }
    
            // Check L1P3 (ID 3) - from L0P2 rollup, should be stored
            let stored_l1_p3 = storage.load_page(1, 3).await.unwrap().expect("L1P3 (ID 3) from L0P2 rollup should be stored");
            assert_eq!(stored_l1_p3.page_id, 3);
            assert_eq!(stored_l1_p3.content_len(), 1, "Stored L1P3 should have 1 thrall hash from L0P2");
            match &stored_l1_p3.content {
                PageContent::ThrallHashes(hashes) => {
                    assert_eq!(hashes.len(), 1, "L1P3 (ID 3) content length mismatch");
                    assert_eq!(hashes[0], stored_l0_p2.page_hash, "L1P3 (ID 3) thrall hash mismatch");
                }
                _ => panic!("Expected ThrallHashes for stored_l1_p3, found {:?}", stored_l1_p3.content),
            }
            assert_eq!(stored_l1_p3.prev_page_hash, Some(stored_l1_p1_leaf1.page_hash), "L1P3 prev_page_hash should be L1P1's hash");
            assert!(manager.active_pages.get(&1u8).is_none(), "No L1 page should be active");
            assert!(manager.active_pages.get(&2u8).is_none(), "No L2 page should exist");
    
            // Leaf 3: Creates L0P4 (ID 4). L0P4 finalizes. L1P5 (ID 5) created from L0P4 rollup, finalizes.
            // Original L0P0 (ID 0) and L0P2 (ID 2) are finalized and stored.
            // Original L1P1 (ID 1) and L1P3 (ID 3) are finalized and stored.
            let leaf3_ts = now + Duration::milliseconds(200);
            let leaf3 = JournalLeaf::new(leaf3_ts, None, "container3".to_string(), json!({"id":3})).unwrap();
            manager.add_leaf(&leaf3).await.unwrap();
    
            // Check L0P4 (ID 4) - created for leaf3
            let stored_l0_p4 = storage.load_page(0, 4).await.unwrap().expect("L0P4 (ID 4) should be in storage after leaf3");
            assert_eq!(stored_l0_p4.page_id, 4);
            assert_eq!(stored_l0_p4.content_len(), 1, "L0P4 (ID 4) should have 1 leaf (leaf3)");
            match &stored_l0_p4.content {
                PageContent::Leaves(leaves) => {
                    assert_eq!(leaves.len(), 1, "L0P4 (ID 4) content length mismatch");
                    assert_eq!(leaves[0].leaf_hash, leaf3.leaf_hash, "L0P4 (ID 4) leaf hash mismatch");
                }
                _ => panic!("Expected Leaves for stored_l0_p4, found {:?}", stored_l0_p4.content),
            }
            assert_eq!(stored_l0_p4.prev_page_hash, Some(stored_l0_p2.page_hash), "L0P4 prev_page_hash should be L0P2's hash");
            assert!(manager.active_pages.get(&0u8).is_none(), "No L0 page should be active");
            
            // Check L1P5 (ID 5) - from L0P4 rollup, should be stored
            let stored_l1_p5 = storage.load_page(1, 5).await.unwrap().expect("L1P5 (ID 5) from L0P4 rollup should be stored");
            assert_eq!(stored_l1_p5.page_id, 5);
            assert_eq!(stored_l1_p5.content_len(), 1, "Stored L1P5 should have 1 thrall hash from L0P4");
            match &stored_l1_p5.content {
                PageContent::ThrallHashes(hashes) => {
                    assert_eq!(hashes.len(), 1, "L1P5 (ID 5) content length mismatch");
                    assert_eq!(hashes[0], stored_l0_p4.page_hash, "L1P5 (ID 5) thrall hash mismatch");
                }
                _ => panic!("Expected ThrallHashes for stored_l1_p5, found {:?}", stored_l1_p5.content),
            }
            assert_eq!(stored_l1_p5.prev_page_hash, Some(stored_l1_p3.page_hash), "L1P5 prev_page_hash should be L1P3's hash");
            assert!(manager.active_pages.get(&1u8).is_none(), "No L1 page should be active");
        }

    #[tokio::test]
    async fn test_single_rollup_max_items() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        let (manager, storage) = create_test_manager(); // Config from create_test_manager: L0 (1s, mlpp=1), L1 (60s, mlpp=1), no L2.
        let timestamp1 = Utc::now();

        // Leaf 1: L0P0 (ID 0) finalizes, L1P0 (ID 1) finalizes. No L2 configured by create_test_manager.
        let leaf1 = JournalLeaf::new(timestamp1, None, "rollup_container1".to_string(), json!({ "data": "leaf1" })).unwrap();
        manager.add_leaf(&leaf1).await.unwrap();

        let stored_l0_p0_leaf1 = storage.load_page(0, 0).await.unwrap().expect("L0P0 (ID 0) should be in storage after 1st leaf");
        assert_eq!(stored_l0_p0_leaf1.page_id, 0);
        assert_eq!(stored_l0_p0_leaf1.content_len(), 1);
        match &stored_l0_p0_leaf1.content {
            PageContent::Leaves(leaves) => {
                assert_eq!(leaves.len(), 1, "L0P0 (ID 0) content length mismatch");
                assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0P0 (ID 0) leaf hash mismatch");
            }
            _ => panic!("Expected Leaves for stored_l0_p0_leaf1, found {:?}", stored_l0_p0_leaf1.content),
        }
        assert!(manager.active_pages.get(&0u8).is_none(), "No L0 page should be active after L0P0 finalization");

        let stored_l1_p0_leaf1 = storage.load_page(1, 1).await.unwrap().expect("L1P0 (ID 1) should be stored after L0P0 rollup and L1 finalization");
        assert_eq!(stored_l1_p0_leaf1.page_id, 1);
        assert_eq!(stored_l1_p0_leaf1.content_len(), 1, "Stored L1P0 should have 1 thrall hash from L0P0");
        match &stored_l1_p0_leaf1.content {
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 1, "L1P0 (ID 1) content length mismatch in test_single_rollup_max_items");
                assert_eq!(hashes[0], stored_l0_p0_leaf1.page_hash, "L1P0 (ID 1) thrall hash mismatch in test_single_rollup_max_items");
            }
            _ => panic!("Expected ThrallHashes for stored_l1_p0_leaf1 in test_single_rollup_max_items, found {:?}", stored_l1_p0_leaf1.content),
        }
        assert!(manager.active_pages.get(&1u8).is_none(), "No L1 page should be active after L1P0 finalization (L1 is highest configured level)");
        assert!(manager.active_pages.get(&2u8).is_none(), "No L2 page should exist as L2 is not configured by create_test_manager");

        // Leaf 2: Creates L0P2 (ID 2) because L0P0 (ID 0) already finalized. L0P2 finalizes. L1P3 (ID 3) created from L0P2 rollup, finalizes.
        let timestamp2 = timestamp1 + Duration::milliseconds(100); // timestamp1 is from first leaf's setup
        let leaf2 = JournalLeaf::new(timestamp2, None, "rollup_container2".to_string(), json!({ "data": "leaf2" })).unwrap();
        manager.add_leaf(&leaf2).await.unwrap();

        // Check L0P0 (ID 0) - should still have only leaf1. stored_l0_p0_leaf1 is from the first leaf's assertions.
        assert_eq!(stored_l0_p0_leaf1.page_id, 0);
        assert_eq!(stored_l0_p0_leaf1.content_len(), 1, "L0P0 (ID 0) should still only have 1 leaf (leaf1)");
        match &stored_l0_p0_leaf1.content { // Re-assertion for L0P0 after leaf2 added
            PageContent::Leaves(leaves) => {
                assert_eq!(leaves.len(), 1, "L0P0 (ID 0) content length mismatch (re-assertion)");
                assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0P0 (ID 0) leaf hash mismatch (re-assertion)");
            }
            _ => panic!("Expected Leaves for stored_l0_p0_leaf1 (re-assertion), found {:?}", stored_l0_p0_leaf1.content),
        } // leaf1 is from first leaf's setup

        // Check L0P2 (ID 2) - created for leaf2
        let stored_l0_p2 = storage.load_page(0, 2).await.unwrap().expect("L0P2 (ID 2) should be in storage after leaf2");
        assert_eq!(stored_l0_p2.page_id, 2);
        assert_eq!(stored_l0_p2.content_len(), 1, "L0P2 (ID 2) should have 1 leaf (leaf2)");
        match &stored_l0_p2.content {
            PageContent::Leaves(leaves) => {
                assert_eq!(leaves.len(), 1, "L0P2 (ID 2) content length mismatch");
                assert_eq!(leaves[0].leaf_hash, leaf2.leaf_hash, "L0P2 (ID 2) leaf hash mismatch");
            }
            _ => panic!("Expected Leaves for stored_l0_p2, found {:?}", stored_l0_p2.content),
        }
        assert_eq!(stored_l0_p2.prev_page_hash, Some(stored_l0_p0_leaf1.page_hash), "L0P2 prev_page_hash should be L0P0's hash");
        assert!(manager.active_pages.get(&0u8).is_none(), "No L0 page should be active");

        // Check L1P1 (ID 1) - from L0P0 rollup. stored_l1_p0_leaf1 is from the first leaf's assertions (Page ID 1).
        assert_eq!(stored_l1_p0_leaf1.page_id, 1);
        assert_eq!(stored_l1_p0_leaf1.content_len(), 1, "Stored L1P1 (ID 1) should have 1 thrall hash from L0P0");
        match &stored_l1_p0_leaf1.content { // This is page ID 1 (stored_l1_p0_leaf1)
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 1, "Stored L1P1 (ID 1) content length mismatch (re-assertion)");
                assert_eq!(hashes[0], stored_l0_p0_leaf1.page_hash, "Stored L1P1 (ID 1) thrall hash mismatch (re-assertion)");
            }
            _ => panic!("Expected ThrallHashes for stored_l1_p0_leaf1 (re-assertion), found {:?}", stored_l1_p0_leaf1.content),
        }

        // Check L1P3 (ID 3) - from L0P2 rollup, should be stored
        let stored_l1_p3 = storage.load_page(1, 3).await.unwrap().expect("L1P3 (ID 3) from L0P2 rollup should be stored");
        assert_eq!(stored_l1_p3.page_id, 3);
        assert_eq!(stored_l1_p3.content_len(), 1, "Stored L1P3 should have 1 thrall hash from L0P2");
        match &stored_l1_p3.content {
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 1, "Stored L1P3 (ID 3) content length mismatch");
                assert_eq!(hashes[0], stored_l0_p2.page_hash, "Stored L1P3 (ID 3) thrall hash mismatch");
            }
            _ => panic!("Expected ThrallHashes for stored_l1_p3, found {:?}", stored_l1_p3.content),
        }
        assert_eq!(stored_l1_p3.prev_page_hash, Some(stored_l1_p0_leaf1.page_hash), "L1P3 prev_page_hash should be L1P1's hash"); // Corrected variable
        assert!(manager.active_pages.get(&1u8).is_none(), "No L1 page should be active");
        assert!(manager.active_pages.get(&2u8).is_none(), "No L2 page should exist");

        // Check last finalized for L0
        assert_eq!(*manager.last_finalized_page_ids.get(&0u8).unwrap().value(), stored_l0_p2.page_id, "L0 last_finalized_page_id mismatch");
        assert_eq!(*manager.last_finalized_page_hashes.get(&0u8).unwrap().value(), stored_l0_p2.page_hash, "L0 last_finalized_page_hash mismatch");
        // Leaf 3: Creates L0P4 (ID 4) because L0P2 (ID 2) already finalized. L0P4 finalizes. L1P5 (ID 5) created from L0P4 rollup, finalizes.
        let timestamp3 = timestamp2 + Duration::milliseconds(100);
        let leaf3 = JournalLeaf::new(timestamp3, None, "rollup_container3".to_string(), json!({ "data": "leaf3" })).unwrap();
        manager.add_leaf(&leaf3).await.unwrap();

        // Check L0P4 (ID 4) - created for leaf3
        let stored_l0_p4 = storage.load_page(0, 4).await.unwrap().expect("L0P4 (ID 4) should be in storage after leaf3");
        assert_eq!(stored_l0_p4.page_id, 4);
        assert_eq!(stored_l0_p4.content_len(), 1, "L0P4 (ID 4) should have 1 leaf (leaf3)");
        match &stored_l0_p4.content {
            PageContent::Leaves(leaves) => {
                assert_eq!(leaves.len(), 1, "L0P4 (ID 4) content length mismatch");
                assert_eq!(leaves[0].leaf_hash, leaf3.leaf_hash, "L0P4 (ID 4) leaf hash mismatch");
            }
            _ => panic!("Expected Leaves for stored_l0_p4, found {:?}", stored_l0_p4.content),
        }
        assert_eq!(stored_l0_p4.prev_page_hash, Some(stored_l0_p2.page_hash), "L0P4 prev_page_hash should be L0P2's hash"); // Links to L0P2
        assert!(manager.active_pages.get(&0u8).is_none(), "No L0 page should be active");

        // Check L1P5 (ID 5) - from L0P4 rollup, should be stored
        let stored_l1_p5 = storage.load_page(1, 5).await.unwrap().expect("L1P5 (ID 5) from L0P4 rollup should be stored");
        assert_eq!(stored_l1_p5.page_id, 5);
        assert_eq!(stored_l1_p5.content_len(), 1, "Stored L1P5 should have 1 thrall hash from L0P4");
        match &stored_l1_p5.content {
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 1, "Stored L1P5 (ID 5) content length mismatch");
                assert_eq!(hashes[0], stored_l0_p4.page_hash, "Stored L1P5 (ID 5) thrall hash mismatch");
            }
            _ => panic!("Expected ThrallHashes for stored_l1_p5, found {:?}", stored_l1_p5.content),
        }
        assert_eq!(stored_l1_p5.prev_page_hash, Some(stored_l1_p3.page_hash), "L1P5 prev_page_hash should be L1P3's hash"); // Links to L1P3
        assert!(manager.active_pages.get(&1u8).is_none(), "No L1 page should be active");
        
        // Check last finalized for L0 and L1 after leaf 3
        assert_eq!(*manager.last_finalized_page_ids.get(&0u8).unwrap().value(), stored_l0_p4.page_id, "L0 last_finalized_page_id mismatch after leaf3");
        assert_eq!(*manager.last_finalized_page_hashes.get(&0u8).unwrap().value(), stored_l0_p4.page_hash, "L0 last_finalized_page_hash mismatch after leaf3");
        assert_eq!(*manager.last_finalized_page_ids.get(&1u8).unwrap().value(), stored_l1_p5.page_id, "L1 last_finalized_page_id mismatch after leaf3");
        assert_eq!(*manager.last_finalized_page_hashes.get(&1u8).unwrap().value(), stored_l1_p5.page_hash, "L1 last_finalized_page_hash mismatch after leaf3");
    }

fn create_cascading_test_config_and_manager() -> (TimeHierarchyManager, Arc<MemoryStorage>) {
    let config_val = Config {
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                TimeLevel {
                    rollup_config: RollupConfig {
                        max_leaves_per_page: 1,
                        max_page_age_seconds: 1000,
                        force_rollup_on_shutdown: false,
                    },
                    retention_policy: None, name: "L0".to_string(), duration_seconds: 1 },
                TimeLevel {
                    rollup_config: RollupConfig {
                        max_leaves_per_page: 1,
                        max_page_age_seconds: 1000,
                        force_rollup_on_shutdown: false,
                    },
                    retention_policy: None, name: "L1".to_string(), duration_seconds: 1 },
                TimeLevel {
                    rollup_config: RollupConfig {
                        max_leaves_per_page: 1,
                        max_page_age_seconds: 1000,
                        force_rollup_on_shutdown: false,
                    },
                    retention_policy: None, name: "L2".to_string(), duration_seconds: 1 },
            ],
        },
        rollup: RollupConfig {
            max_leaves_per_page: 1, // This is key for cascading by item count
            max_page_age_seconds: 1000, // Large age so it doesn't interfere with item count based finalization
            force_rollup_on_shutdown: false,
        },
        storage: Default::default(),
        retention: RetentionConfig { // Default retention disabled for these rollup tests
            enabled: false,
            period_seconds: 0,
            // REMOVED: default_max_age_seconds: 0,
            cleanup_interval_seconds: 300, // Assuming a default, consistent with create_test_config
        },
        compression: Default::default(),
        logging: Default::default(),
        metrics: Default::default(),
    };
    // max_leaves_per_page is set to 1 above for cascading tests.

    let config = Arc::new(config_val);
    let storage = Arc::new(MemoryStorage::new());
    let manager = TimeHierarchyManager::new(config.clone(), storage.clone());
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
        assert_eq!(stored_l0_page.content_len(), 1);
        match &stored_l0_page.content {
            PageContent::Leaves(leaves) => {
                assert_eq!(leaves.len(), 1, "L0 page should have 1 leaf");
                assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0 leaf hash mismatch");
            }
            _ => panic!("L0 page content should be Leaves, found {:?}", stored_l0_page.content),
        }

        // L1P1 (ID 1) assertions
        let stored_l1_page = storage.load_page(1, 1).await.expect("Storage query failed for L1P1 in cascade").expect("L1P1 not found after cascade");
        assert_eq!(stored_l1_page.page_id, 1);
        assert_eq!(stored_l1_page.level, 1);
        assert_eq!(stored_l1_page.content_len(), 1);
        match &stored_l1_page.content {
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 1, "L1 page should have 1 thrall hash");
                assert_eq!(hashes[0], stored_l0_page.page_hash, "L1 thrall hash mismatch");
            }
            _ => panic!("L1 page content should be ThrallHashes, found {:?}", stored_l1_page.content),
        }
        assert_eq!(stored_l1_page.prev_page_hash, None, "L1P1 should not have a prev_page_hash (first in its level)");

        // L2P2 (ID 2) assertions (highest level in this config)
        let stored_l2_page = storage.load_page(2, 2).await.expect("Storage query failed for L2P2 in cascade").expect("L2P2 not found after cascade");
        assert_eq!(stored_l2_page.page_id, 2);
        assert_eq!(stored_l2_page.level, 2);
        assert_eq!(stored_l2_page.content_len(), 1);
        match &stored_l2_page.content {
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 1, "L2 page should have 1 thrall hash");
                assert_eq!(hashes[0], stored_l1_page.page_hash, "L2 thrall hash mismatch");
            }
            _ => panic!("L2 page content should be ThrallHashes, found {:?}", stored_l2_page.content),
        }
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

    #[tokio::test]
    async fn test_retention_disabled() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        let config = Arc::new(create_test_config(false, 60, None, None)); // Retention disabled, 60s period (irrelevant)
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

        let now = Utc::now();
        let old_page_time = now - Duration::seconds(120); // 2 minutes ago, older than 60s

        // Create and store an old page
        let old_page = create_retention_test_page(0, 0, old_page_time, &config);
        storage.store_page(&old_page).await.unwrap();
        assert!(storage.page_exists(0, old_page.page_id).await.unwrap(), "Old page should exist before retention");

        manager.apply_retention_policies().await.unwrap();

        assert!(storage.page_exists(0, old_page.page_id).await.unwrap(), "Old page should still exist as retention is disabled");
    }

    #[tokio::test]
    async fn test_retention_period_zero() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        // Retention enabled, but period is 0 seconds
        let config = Arc::new(create_test_config(true, 0, None, None)); 
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

        let now = Utc::now();
        let old_page_time = now - Duration::seconds(120); // 2 minutes ago

        let old_page = create_retention_test_page(0, 0, old_page_time, &config);
        storage.store_page(&old_page).await.unwrap();
        assert!(storage.page_exists(0, old_page.page_id).await.unwrap(), "Old page should exist before retention");

        manager.apply_retention_policies().await.unwrap();

        // Page should still exist because a 0-second retention period means nothing is ever old enough to delete
        assert!(storage.page_exists(0, old_page.page_id).await.unwrap(), "Old page should still exist with 0s retention period");
    }

    #[tokio::test]
    async fn test_retention_no_pages_old_enough() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        let retention_period_seconds = 60;
        let config = Arc::new(create_test_config(true, retention_period_seconds, None, None)); 
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

        let now = Utc::now();
        // Page is newer than retention period (30s old vs 60s period)
        let newish_page_time = now - Duration::seconds(30); 

        let newish_page = create_retention_test_page(0, 0, newish_page_time, &config);
        storage.store_page(&newish_page).await.unwrap();
        assert!(storage.page_exists(0, newish_page.page_id).await.unwrap(), "Newish page should exist before retention");

        manager.apply_retention_policies().await.unwrap();

        assert!(storage.page_exists(0, newish_page.page_id).await.unwrap(), "Newish page should still exist as it's not old enough");
    }

    #[tokio::test]
    async fn test_retention_some_pages_deleted() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        let retention_period_seconds = 60;
        let config = Arc::new(create_test_config(true, retention_period_seconds, None, None));
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

        let now = Utc::now();
        let old_page_time = now - Duration::seconds(120); // 2 minutes old, older than 60s period
        let new_page_time = now - Duration::seconds(30);  // 30 seconds old, newer than 60s period

        // Create and store an old page (page_id will be offset by 0 from current NEXT_PAGE_ID)
        let old_page = create_retention_test_page(0, 0, old_page_time, &config);
        storage.store_page(&old_page).await.unwrap();
        let old_page_id = old_page.page_id;
        assert!(storage.page_exists(0, old_page_id).await.unwrap(), "Old page should exist before retention");

        // Create and store a new page (page_id will be offset by 1 from current NEXT_PAGE_ID after old_page)
        // Ensure reset_global_ids() was called so page IDs are predictable if create_retention_test_page manipulates it.
        // Or, ensure create_retention_test_page uses unique offsets if called multiple times without reset in between.
        // Current create_retention_test_page adds offset to current NEXT_PAGE_ID, so subsequent calls will get higher IDs.
        let new_page = create_retention_test_page(0, 1, new_page_time, &config); 
        storage.store_page(&new_page).await.unwrap();
        let new_page_id = new_page.page_id;
        assert!(storage.page_exists(0, new_page_id).await.unwrap(), "New page should exist before retention");
        assert_ne!(old_page_id, new_page_id, "Page IDs must be different");

        manager.apply_retention_policies().await.unwrap();

        assert!(!storage.page_exists(0, old_page_id).await.unwrap(), "Old page should have been deleted");
        assert!(storage.page_exists(0, new_page_id).await.unwrap(), "New page should still exist");
    }

    #[tokio::test]
    async fn test_retention_all_pages_deleted() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        let retention_period_seconds = 60;
        let config = Arc::new(create_test_config(true, retention_period_seconds, None, None));
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

        let now = Utc::now();
        let very_old_page_time = now - Duration::seconds(120); // 2 minutes old
        let also_old_page_time = now - Duration::seconds(90);  // 1.5 minutes old

        let page1 = create_retention_test_page(0, 0, very_old_page_time, &config);
        storage.store_page(&page1).await.unwrap();
        let page1_id = page1.page_id;
        assert!(storage.page_exists(0, page1_id).await.unwrap(), "Page 1 should exist before retention");

        let page2 = create_retention_test_page(0, 1, also_old_page_time, &config);
        storage.store_page(&page2).await.unwrap();
        let page2_id = page2.page_id;
        assert!(storage.page_exists(0, page2_id).await.unwrap(), "Page 2 should exist before retention");

        manager.apply_retention_policies().await.unwrap();

        assert!(!storage.page_exists(0, page1_id).await.unwrap(), "Page 1 should have been deleted");
        assert!(!storage.page_exists(0, page2_id).await.unwrap(), "Page 2 should have been deleted");
    }

    #[tokio::test]
    async fn test_retention_multi_level() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        let retention_period_seconds = 60;
        // Config has 2 levels by default from create_test_config: L0 (1s), L1 (60s)
        let config = Arc::new(create_test_config(true, retention_period_seconds, None, None));
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

        let now = Utc::now();

        // Level 0 pages
        let l0_old_time = now - Duration::seconds(120); // 2 mins old, > 60s retention
        let l0_new_time = now - Duration::seconds(30);  // 30s old, < 60s retention
        let l0_old_page = create_retention_test_page(0, 0, l0_old_time, &config);
        let l0_new_page = create_retention_test_page(0, 1, l0_new_time, &config);
        storage.store_page(&l0_old_page).await.unwrap();
        storage.store_page(&l0_new_page).await.unwrap();

        // Level 1 pages
        // Note: Page end_time depends on level's duration. 
        // L1 duration is 60s. So a page starting 120s ago ended 60s ago.
        // A page starting 70s ago ended 10s ago.
        let _l1_old_time = now - Duration::seconds(120); // Page ended ~60s ago (120s start + 60s duration for L1). Age is ~60s. This might be on the edge or kept depending on exact now.
                                                      // Let's make it clearly old: start time 180s ago, so end_time is 120s ago.
        let l1_very_old_time = now - Duration::seconds(180); // Page ended 120s ago. Age is 120s. Should be deleted.
        let l1_new_time = now - Duration::seconds(70);  // Page ended 10s ago. Age is 10s. Should be kept.

        let l1_old_page = create_retention_test_page(1, 2, l1_very_old_time, &config);
        let l1_new_page = create_retention_test_page(1, 3, l1_new_time, &config);
        storage.store_page(&l1_old_page).await.unwrap();
        storage.store_page(&l1_new_page).await.unwrap();

        assert!(storage.page_exists(0, l0_old_page.page_id).await.unwrap(), "L0 old page pre-retention");
        assert!(storage.page_exists(0, l0_new_page.page_id).await.unwrap(), "L0 new page pre-retention");
        assert!(storage.page_exists(1, l1_old_page.page_id).await.unwrap(), "L1 old page pre-retention");
        assert!(storage.page_exists(1, l1_new_page.page_id).await.unwrap(), "L1 new page pre-retention");

        manager.apply_retention_policies().await.unwrap();

        assert!(!storage.page_exists(0, l0_old_page.page_id).await.unwrap(), "L0 old page should be deleted");
        assert!(storage.page_exists(0, l0_new_page.page_id).await.unwrap(), "L0 new page should be kept");
        assert!(!storage.page_exists(1, l1_old_page.page_id).await.unwrap(), "L1 old page should be deleted");
        assert!(storage.page_exists(1, l1_new_page.page_id).await.unwrap(), "L1 new page should be kept");
    }

    #[tokio::test]
    async fn test_retention_l0_keep_n_pages() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        let keep_n = 2;
        // Global retention is enabled but L0 will use its specific KeepNPages policy.
        // L1 will use global (DeleteAfterSecs(60)) or KeepIndefinitely if global is off.
        let config = Arc::new(create_test_config(
            true, // global retention enabled
            60,   // global retention period_seconds (should not affect L0)
            Some(RollupRetentionPolicy::KeepNPages(keep_n)), // L0 policy
            None, // L1 policy (will use global or default)
        ));
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

        let now = Utc::now();
        // Create 4 pages for L0 with varying end times
        let page1_time = now - Duration::seconds(400);
        let page2_time = now - Duration::seconds(300);
        let page3_time = now - Duration::seconds(200);
        let page4_time = now - Duration::seconds(100);

        let page1 = create_retention_test_page(0, 0, page1_time, &config);
        let page2 = create_retention_test_page(0, 1, page2_time, &config);
        let page3 = create_retention_test_page(0, 2, page3_time, &config);
        let page4 = create_retention_test_page(0, 3, page4_time, &config);

        storage.store_page(&page1).await.unwrap();
        storage.store_page(&page2).await.unwrap();
        storage.store_page(&page3).await.unwrap();
        storage.store_page(&page4).await.unwrap();

        assert!(storage.page_exists(0, page1.page_id).await.unwrap(), "L0 Page 1 pre-retention");
        assert!(storage.page_exists(0, page2.page_id).await.unwrap(), "L0 Page 2 pre-retention");
        assert!(storage.page_exists(0, page3.page_id).await.unwrap(), "L0 Page 3 pre-retention");
        assert!(storage.page_exists(0, page4.page_id).await.unwrap(), "L0 Page 4 pre-retention");

        manager.apply_retention_policies().await.unwrap();

        // Page1 and Page2 are the oldest, should be deleted.
        // Page3 and Page4 are the newest, should be kept.
        assert!(!storage.page_exists(0, page1.page_id).await.unwrap(), "L0 Page 1 (oldest) should be deleted by KeepNPages(2)");
        assert!(!storage.page_exists(0, page2.page_id).await.unwrap(), "L0 Page 2 (second oldest) should be deleted by KeepNPages(2)");
        assert!(storage.page_exists(0, page3.page_id).await.unwrap(), "L0 Page 3 (second newest) should be kept by KeepNPages(2)");
        assert!(storage.page_exists(0, page4.page_id).await.unwrap(), "L0 Page 4 (newest) should be kept by KeepNPages(2)");
    }

    #[tokio::test]
    async fn test_retention_l0_keep_n_pages_zero() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        // L0 policy is KeepNPages(0), which should delete all L0 pages.
        let config = Arc::new(create_test_config(
            true, // global retention enabled
            60,   // global retention period_seconds (should not affect L0)
            Some(RollupRetentionPolicy::KeepNPages(0)), // L0 policy
            None, // L1 policy
        ));
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

        let now = Utc::now();
        let page1_time = now - Duration::seconds(200);
        let page2_time = now - Duration::seconds(100);

        let page1 = create_retention_test_page(0, 0, page1_time, &config);
        let page2 = create_retention_test_page(0, 1, page2_time, &config);

        storage.store_page(&page1).await.unwrap();
        storage.store_page(&page2).await.unwrap();

        assert!(storage.page_exists(0, page1.page_id).await.unwrap(), "L0 Page 1 pre-retention");
        assert!(storage.page_exists(0, page2.page_id).await.unwrap(), "L0 Page 2 pre-retention");

        manager.apply_retention_policies().await.unwrap();

        assert!(!storage.page_exists(0, page1.page_id).await.unwrap(), "L0 Page 1 should be deleted by KeepNPages(0)");
        assert!(!storage.page_exists(0, page2.page_id).await.unwrap(), "L0 Page 2 should be deleted by KeepNPages(0)");
    }

    #[tokio::test]
    async fn test_retention_mixed_policies_l0_keep_n_l1_delete_after_secs() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        let l0_keep_n = 1;
        let l1_delete_after_secs = 30u64;

        let config = Arc::new(create_test_config(
            false, // global retention disabled, policies are per-level only
            0,     // global retention period_seconds (irrelevant)
            Some(RollupRetentionPolicy::KeepNPages(l0_keep_n)), // L0 policy
            Some(RollupRetentionPolicy::DeleteAfterSecs(l1_delete_after_secs)), // L1 policy
        ));
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());

        let now = Utc::now();

        // L0 pages: KeepNPages(1)
        let l0_page_old_time = now - Duration::seconds(200);
        let l0_page_new_time = now - Duration::seconds(100);
        let l0_page_old = create_retention_test_page(0, 0, l0_page_old_time, &config);
        let l0_page_new = create_retention_test_page(0, 1, l0_page_new_time, &config);
        storage.store_page(&l0_page_old).await.unwrap();
        storage.store_page(&l0_page_new).await.unwrap();

        // L1 pages: DeleteAfterSecs(30)
        // L1 page duration is 60s from create_test_config.
        // Page end_time = start_time + level_duration
        // Page age = now - end_time
        let l1_page_old_start_time = now - Duration::seconds(100); // end_time = now - 40s. Age = 40s. Should be deleted.
        let l1_page_new_start_time = now - Duration::seconds(70);  // end_time = now - 10s. Age = 10s. Should be kept.
        
        let l1_page_old = create_retention_test_page(1, 2, l1_page_old_start_time, &config);
        let l1_page_new = create_retention_test_page(1, 3, l1_page_new_start_time, &config);
        storage.store_page(&l1_page_old).await.unwrap();
        storage.store_page(&l1_page_new).await.unwrap();

        assert!(storage.page_exists(0, l0_page_old.page_id).await.unwrap(), "L0 old pre-retention");
        assert!(storage.page_exists(0, l0_page_new.page_id).await.unwrap(), "L0 new pre-retention");
        assert!(storage.page_exists(1, l1_page_old.page_id).await.unwrap(), "L1 old pre-retention");
        assert!(storage.page_exists(1, l1_page_new.page_id).await.unwrap(), "L1 new pre-retention");

        manager.apply_retention_policies().await.unwrap();

        // L0 assertions: KeepNPages(1)
        assert!(!storage.page_exists(0, l0_page_old.page_id).await.unwrap(), "L0 old should be deleted by KeepNPages(1)");
        assert!(storage.page_exists(0, l0_page_new.page_id).await.unwrap(), "L0 new should be kept by KeepNPages(1)");

        // L1 assertions: DeleteAfterSecs(30)
        assert!(!storage.page_exists(1, l1_page_old.page_id).await.unwrap(), "L1 old (age 40s) should be deleted by DeleteAfterSecs(30)");
        assert!(storage.page_exists(1, l1_page_new.page_id).await.unwrap(), "L1 new (age 10s) should be kept by DeleteAfterSecs(30)");
    }

}
