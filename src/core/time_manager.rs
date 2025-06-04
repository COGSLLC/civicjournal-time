// src/core/time_manager.rs

use std::sync::Arc;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::storage::StorageBackend;
use crate::core::leaf::JournalLeaf;
use crate::core::page::{JournalPage, PageContent, PageIdGenerator};
use crate::error::CJError;
use crate::types::time::{RollupContentType, RollupRetentionPolicy, LevelRollupConfig};

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
    page_id_gen: PageIdGenerator,
    /// Stores the currently active (open for writing) page for each level.
    /// Key: level (u8), Value: JournalPage
    /// This helps avoid repeatedly loading the same page from storage if many leaves
    /// are added to it sequentially.
    pub active_pages: Arc<Mutex<HashMap<u8, JournalPage>>>,
    pub last_finalized_page_ids: Arc<tokio::sync::Mutex<HashMap<u8, u64>>>, // Tracks IDs for potential loading, debugging, or specific lookup needs
    pub last_finalized_page_hashes: Arc<tokio::sync::Mutex<HashMap<u8, [u8; 32]>>>, // Tracks actual hashes for chaining
}

impl TimeHierarchyManager {
    pub async fn load_page_from_storage(&self, level: u8, page_id: u64) -> Result<Option<JournalPage>, CJError> {
        self.storage.load_page(level, page_id).await
    }

    /// Creates a new `TimeHierarchyManager`.
    ///
    /// # Arguments
    /// * `config` - The application configuration, wrapped in an `Arc`.
    /// * `storage` - The storage backend, wrapped in an `Arc`.
    pub fn new(config: Arc<Config>, storage: Arc<dyn StorageBackend>) -> Self {
        println!("[THM_NEW_CONFIG_CHECK] Initializing with Config. L0 duration_seconds: {}, L0 max_items_per_page: {}",
            config.time_hierarchy.levels.get(0).map_or(0, |l| l.duration_seconds), // Default to 0 if not found, to make it obvious if L0 config is missing
            config.time_hierarchy.levels.get(0).map_or(0, |l| l.rollup_config.max_items_per_page) // Default to 0 if not found
        );
        // TODO: Initialize active_pages by trying to load the latest open page for each level
        // from storage. This is important for resuming operations after a restart.
        TimeHierarchyManager {
            config,
            storage,
            page_id_gen: PageIdGenerator::new(),
            active_pages: Arc::new(Mutex::new(HashMap::new())),
            last_finalized_page_ids: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            last_finalized_page_hashes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
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
    pub async fn get_current_active_page_id(&self, level: u8) -> Option<u64> {
        let active_pages_guard = self.active_pages.lock().await;
        active_pages_guard.get(&level).map(|page_entry| page_entry.page_id)
    }

    /// Gets the current active page for level 0 for the given leaf, or creates a new one.
    /// This is a simplified version; a more robust impl would check storage for existing pages
    /// that fit the leaf's timestamp if no active page is found or if the active page is not suitable.
    async fn get_or_create_active_page_for_leaf(&self, leaf: &JournalLeaf) -> Result<JournalPage, CJError> {
        println!("[GOCAPL] Called for leaf ts: {}, level: 0", leaf.timestamp);
        let level_idx = 0; // Base level for leaves
        let mut active_pages_guard = self.active_pages.lock().await;
        let (window_start, _window_end) = self.get_page_window(level_idx, leaf.timestamp)?;

        println!("[GOCAPL_DBG] About to call contains_key for level {}", level_idx);
        let key_exists = active_pages_guard.contains_key(&(level_idx as u8));
        println!("[GOCAPL_DBG] Called contains_key for level {}, exists: {}", level_idx, key_exists);

        println!("[GOCAPL_DBG] About to call get_mut for level {}", level_idx);
        // Access the HashMap through the guard
        if let Some(active_page_entry) = active_pages_guard.get_mut(&(level_idx as u8)) {
            println!("[GOCAPL_DBG] Called get_mut for level {}. Found entry.", level_idx);
            println!("[GOCAPL] Found active page L{}P{} (created: {}, ends: {}, leaves: {}). Leaf ts: {}.", 
                     active_page_entry.level, active_page_entry.page_id, 
                     active_page_entry.creation_timestamp, active_page_entry.end_time, 
                     active_page_entry.content_len(), leaf.timestamp);
            // Check if the current active page is suitable for this leaf
            let is_in_window = leaf.timestamp >= active_page_entry.creation_timestamp && leaf.timestamp < active_page_entry.end_time;
            println!("[GOCAPL] Timestamp check (leaf >= page_start && leaf < page_end): {}", is_in_window);
            if is_in_window {
                // Check if the page is not full
                let has_space = active_page_entry.content_len() < self.config.time_hierarchy.levels[0].rollup_config.max_items_per_page;
                println!("[GOCAPL] Space check (leaves < max_leaves): {}", has_space);
                if has_space {
                    println!("[GOCAPL] Reusing (from active_pages) page L{}P{}.", active_page_entry.level, active_page_entry.page_id);
                    let page_to_return = active_page_entry.clone();
                    // Drop the guard before returning to release the lock.
                    // Though it would drop automatically, being explicit can sometimes be clearer.
                    // drop(active_pages_guard);
                    return Ok(page_to_return);
                }
                // If full, do not return this page; proceed to create a new one.
                // The caller (add_leaf) will handle finalizing the full active page.
            }
        } else {
            println!("[GOCAPL_DBG] Called get_mut for level {}. No active page found for this level.", level_idx);
        }

        // No suitable active page in memory (either not present, or not suitable). 
        drop(active_pages_guard);
        // Try to load from storage or create a new one.
        let level_config = self.config.time_hierarchy.levels.get(level_idx)
            .ok_or_else(|| CJError::ConfigError(crate::config::ConfigError::ValidationError(format!("Time hierarchy level {} not configured.", level_idx))))?;

        let mut potential_prev_page_hash: Option<[u8; 32]> = None;

        if let Some(last_page_id) = self.last_finalized_page_ids.lock().await.get(&(level_idx as u8)).copied() {
            // Attempt to load the last known finalized page for this level
            if let Some(loaded_page) = self.storage.load_page(level_idx as u8, last_page_id).await? {
                // A page was loaded. Its hash is a candidate for the new page's prev_hash if we end up creating one.
                potential_prev_page_hash = Some(loaded_page.page_hash);

                // Check if this loaded page can be reused for the current leaf.
                // 1. Time window must match the calculated window for the current leaf.
                // 2. Page must not be full.
                if loaded_page.creation_timestamp == window_start &&
                   loaded_page.end_time == (window_start + level_config.duration()) && // Ensure window matches calculated one
                   (loaded_page.content_len()) < self.config.time_hierarchy.levels[0].rollup_config.max_items_per_page {
                    
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
            if let Some(last_finalized_id) = self.last_finalized_page_ids.lock().await.get(&(level_idx as u8)).copied() {
                // Attempt to load this last finalized page to get its hash
                if let Some(last_finalized_page) = self.storage.load_page(level_idx as u8, last_finalized_id).await? {
                    prev_hash_for_new_page_opt = Some(last_finalized_page.page_hash);
                }
            }
        }

        let new_page = JournalPage::new_with_id(
            self.page_id_gen.next(),
            level_idx as u8,
            prev_hash_for_new_page_opt,
            window_start,
            &self.config,
        );
        println!("[GOCAPL] Proceeding to create new page L{}P{}.", new_page.level, new_page.page_id);
        println!("[GOCAPL] New page creation timestamp: {}", new_page.creation_timestamp);
        Ok(new_page)
    }

    /// Gets or creates an active page for a parent level during rollup.
    async fn get_or_create_active_parent_page(&self, parent_level_idx: u8, triggering_timestamp: DateTime<Utc>) -> Result<JournalPage, CJError> {
        let (window_start, _window_end) = self.get_page_window(parent_level_idx as usize, triggering_timestamp)?;
        let mut active_pages_guard = self.active_pages.lock().await;

        println!("[GOCAPP_DBG] Attempting active_pages_guard.get_mut(&{}) for parent.", parent_level_idx);
        if let Some(active_page_entry) = active_pages_guard.get_mut(&parent_level_idx) {
            println!("[GOCAPP_DBG] Success self.active_pages.get_mut(&{}) for parent. Entry found.", parent_level_idx);
            // Check if the current active page is suitable
            let is_in_window = triggering_timestamp >= active_page_entry.creation_timestamp && triggering_timestamp < active_page_entry.end_time;
            if is_in_window {
                let level_config_for_space_check = self.config.time_hierarchy.levels.get(parent_level_idx as usize)
                    .ok_or_else(|| {
                        CJError::ConfigError(crate::config::ConfigError::ValidationError(format!("Time hierarchy level {} not configured for space check.", parent_level_idx)))
                    })?;
                let has_space = active_page_entry.content_len() < level_config_for_space_check.rollup_config.max_items_per_page;
                if has_space {
                    println!("[GOCAPP] Reusing active page L{}P{} for parent.", active_page_entry.level, active_page_entry.page_id);
                    return Ok(active_page_entry.clone()); // Lock guard drops automatically here
                }
            }
        } else {
            println!("[GOCAPP_DBG] No active page found for parent level {} via get_mut.", parent_level_idx);
        }
        
        drop(active_pages_guard); // Release lock before potentially long storage operations or new page creation

        // No suitable active page, try to load from storage or create new
        let level_config = self.config.time_hierarchy.levels.get(parent_level_idx as usize)
            .ok_or_else(|| CJError::ConfigError(crate::config::ConfigError::ValidationError(format!("Time hierarchy level {} not configured.", parent_level_idx))))?;

        let mut potential_prev_page_hash: Option<[u8; 32]> = None;
        
        if let Some(hash_val) = self.last_finalized_page_hashes.lock().await.get(&parent_level_idx).copied() {
            potential_prev_page_hash = Some(hash_val);
        }

        if let Some(last_page_id) = self.last_finalized_page_ids.lock().await.get(&parent_level_idx).copied() {
            if let Some(loaded_page) = self.storage.load_page(parent_level_idx, last_page_id).await? {
                potential_prev_page_hash = Some(loaded_page.page_hash); 
                if loaded_page.creation_timestamp == window_start &&
                   loaded_page.end_time == (window_start + level_config.duration()) &&
                   (loaded_page.content_len()) < self.config.time_hierarchy.levels[parent_level_idx as usize].rollup_config.max_items_per_page {
                    println!("[GOCAPP] Loaded and reusing page L{}P{} for parent from storage.", loaded_page.level, loaded_page.page_id);
                    return Ok(loaded_page);
                }
            }
        }
        
        let new_page = JournalPage::new_with_id(
            self.page_id_gen.next(),
            parent_level_idx,
            potential_prev_page_hash,
            window_start,
            &self.config,
        );
        println!("[GOCAPP] Created new page L{}P{} for parent.", new_page.level, new_page.page_id);
        Ok(new_page)
    }

    pub async fn add_leaf(&self, leaf: &JournalLeaf) -> Result<u64, CJError> {
        println!("[ADD_LEAF] Called for leaf ts: {}", leaf.timestamp);
        let mut page = self.get_or_create_active_page_for_leaf(leaf).await?;
        let original_page_id = page.page_id; // For logging
        println!("[ADD_LEAF] Obtained page L{}P{} for leaf. Current content_len: {}", page.level, page.page_id, page.content_len());

        let was_empty = page.is_content_empty();
        page.add_leaf(leaf.clone()); // This can fail if leaf is out of order for page (panics)
        println!("[ADD_LEAF] Leaf added to L{}P{}. New content_len: {}", page.level, page.page_id, page.content_len());

        let level_config = &self.config.time_hierarchy.levels[page.level as usize];
        let max_items = level_config.rollup_config.max_items_per_page;
        let max_age = if level_config.rollup_config.max_page_age_seconds > 0 {
            Some(chrono::Duration::seconds(level_config.rollup_config.max_page_age_seconds as i64))
        } else {
            None
        };

        println!("[ADD_LEAF_DBG_FULL] Checking is_full: page.content_len() = {}, max_items = {}", page.content_len(), max_items);
    let is_full = page.content_len() >= max_items;
        // Determine the timestamp to base the age calculation on.
        // If the page was empty before adding this leaf, the "oldest content" effectively becomes this leaf's timestamp.
        // Otherwise, use the page's existing first_child_ts.
        let oldest_ts_in_page_for_age_check = if was_empty { 
            leaf.timestamp 
        } else { 
            page.first_child_ts.unwrap_or(page.creation_timestamp) // Fallback to creation_timestamp if first_child_ts is somehow None
        };
        
        println!("[ADD_LEAF_DBG_AGE] Checking is_over_age: leaf.timestamp = {}, oldest_ts_in_page_for_age_check = {}, max_age = {:?}", leaf.timestamp, oldest_ts_in_page_for_age_check, max_age);
    let is_over_age = max_age.map_or(false, |age_limit| {
            // A page is over_age if the current leaf's timestamp makes the oldest content in the page exceed the max_age.
            // Or, if the page itself (based on its creation) is older than max_age from the current leaf's perspective.
            // The critical check is: `current_time - oldest_content_timestamp >= max_age`
            // Here, `leaf.timestamp` acts as the `current_time` for the decision.
            if leaf.timestamp >= oldest_ts_in_page_for_age_check { // Ensure no negative duration
                let calculated_age = leaf.timestamp - oldest_ts_in_page_for_age_check;
                println!("[ADD_LEAF_DBG_AGE_CALC] calculated_age = {:?}, age_limit = {:?}, comparison_result = {}", calculated_age, age_limit, calculated_age >= age_limit);
                calculated_age >= age_limit
            } else {
                false // Leaf is before oldest content, page cannot be "over age" due to this leaf.
            }
        });
        println!("[ADD_LEAF_DBG_AGE] is_over_age = {}", is_over_age);
        
        println!("[ADD_LEAF] Page L{}P{} - Full: {}, OverAge: {} (oldest_ts_for_age_check: {}, leaf_ts: {}, max_age: {:?})", 
            page.level, page.page_id, is_full, is_over_age, oldest_ts_in_page_for_age_check, leaf.timestamp, max_age);

        if is_full || is_over_age {
            page.recalculate_merkle_root_and_page_hash(); // Finalize hash before storing
            println!("[ADD_LEAF] Finalizing page L{}P{} due to: Full ({}), OverAge ({}). Hash: {:.8}", 
                page.level, page.page_id, is_full, is_over_age, 
                page.page_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>());

            self.storage.store_page(&page).await?;
            self.last_finalized_page_ids.lock().await.insert(page.level, page.page_id);
            self.last_finalized_page_hashes.lock().await.insert(page.level, page.page_hash);
            
            let mut active_pages_guard = self.active_pages.lock().await;
            active_pages_guard.remove(&page.level);
            drop(active_pages_guard); // Release lock before calling perform_rollup

            println!("[ADD_LEAF] Removed L{}P{} from active_pages. Triggering rollup.", page.level, page.page_id);
            // Use original_page_id's timestamp (leaf.timestamp) for the rollup trigger
            self.perform_rollup(page.level, page.page_hash, leaf.timestamp).await?;
        } else {
            let mut active_pages_guard = self.active_pages.lock().await;
            active_pages_guard.insert(page.level, page.clone());
            drop(active_pages_guard);
            println!("[ADD_LEAF] Updated L{}P{} in active_pages.", page.level, page.page_id);
        }
        Ok(original_page_id)
    }



    pub async fn trigger_special_rollup(&self, trigger_timestamp: DateTime<Utc>) -> Result<(), CJError> {
        println!("[SPECIAL_ROLLUP] Called with trigger_timestamp: {}", trigger_timestamp);

        let mut active_pages_guard = self.active_pages.lock().await;
        if let Some(mut page_to_finalize) = active_pages_guard.remove(&0) { // Try to take L0 page
            drop(active_pages_guard); // Release lock early if page was found and removed

            if page_to_finalize.is_content_empty() {
                println!("[SPECIAL_ROLLUP] Active L0 page P{} is empty. No rollup triggered.", page_to_finalize.page_id);
                // If this L0 page was empty and removed, it's effectively discarded.
                // If it needs to be re-inserted if empty, that logic would go here,
                // but typically special rollups aim to flush non-empty pages.
                return Ok(());
            }

            println!("[SPECIAL_ROLLUP] Finalizing active L0 page P{} (content_len: {}).", page_to_finalize.page_id, page_to_finalize.content_len());
            page_to_finalize.recalculate_merkle_root_and_page_hash(); // Finalize hash before storing
            
            self.storage.store_page(&page_to_finalize).await?;
            println!("[SPECIAL_ROLLUP] Stored L0P{}. Hash: {:.8}", page_to_finalize.page_id, page_to_finalize.page_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>());

            // Update tracking for the finalized L0 page
            self.last_finalized_page_ids.lock().await.insert(0, page_to_finalize.page_id);
            self.last_finalized_page_hashes.lock().await.insert(0, page_to_finalize.page_hash);
            
            // The page was already removed from active_pages by active_pages_guard.remove(&0)

            println!("[SPECIAL_ROLLUP] Triggering perform_rollup for L0P{} with original_trigger_timestamp: {}", page_to_finalize.page_id, trigger_timestamp);
            // Call perform_rollup with the finalized L0 page's details
            self.perform_rollup(0, page_to_finalize.page_hash, trigger_timestamp).await?;
            println!("[SPECIAL_ROLLUP] Completed for L0P{}.", page_to_finalize.page_id);

        } else {
            drop(active_pages_guard); // Release lock if no L0 page was found
            println!("[SPECIAL_ROLLUP] No active L0 page found to finalize.");
        }
        Ok(())
    }

/// Applies retention policies to pages based on configuration.
pub async fn apply_retention_policies(&self) -> Result<(), CJError> {
    let now = Utc::now();
    println!("[Retention] Applying retention policies with current time: {:?}", now);

    for (level_idx, level_config) in self.config.time_hierarchy.levels.iter().enumerate() {
        let level = level_idx as u8;
        println!("[Retention] Processing L{} ({})", level, level_config.name);

        let effective_policy = match &level_config.retention_policy {
            Some(specific_policy) => {
                println!("[Retention] L{}: Using specific policy: {:?}", level, specific_policy);
                specific_policy.clone()
            }
            None => {
                if self.config.retention.enabled && self.config.retention.period_seconds > 0 {
                    println!("[Retention] L{}: No specific policy, using global: DeleteAfterSecs({}).", level, self.config.retention.period_seconds);
                    RollupRetentionPolicy::DeleteAfterSecs(self.config.retention.period_seconds)
                } else {
                    println!("[Retention] L{}: No specific policy, global retention disabled or period is 0. Defaulting to KeepIndefinitely.", level);
                    RollupRetentionPolicy::KeepIndefinitely
                }
            }
        };

        match effective_policy {
            RollupRetentionPolicy::KeepIndefinitely => {
                println!("[Retention] L{}: Policy is KeepIndefinitely. No pages will be deleted.", level);
            }
            RollupRetentionPolicy::DeleteAfterSecs(secs) => {
                if secs == 0 {
                    println!("[Retention] L{}: Policy is DeleteAfterSecs(0). Skipping as this implies immediate deletion or misconfiguration.", level);
                    continue;
                }
                let retention_duration = chrono::Duration::seconds(secs as i64);
                println!("[Retention] L{}: Policy is DeleteAfterSecs({}s). Max age: {}.", level, secs, retention_duration);

                let page_summaries = match self.storage.list_finalized_pages_summary(level).await {
                    Ok(summaries) => summaries,
                    Err(e) => {
                        eprintln!("[Retention] L{}: Error listing finalized pages: {}. Skipping level.", level, e);
                        continue;
                    }
                };

                if page_summaries.is_empty() {
                    println!("[Retention] L{}: No finalized pages found.", level);
                    continue;
                }
                println!("[Retention] L{}: Found {} finalized pages.", level, page_summaries.len());

                for summary in page_summaries {
                    if summary.end_time > now {
                        println!("[Retention] L{}/{}: Skipping, end_time {} is in the future.", level, summary.page_id, summary.end_time);
                        continue;
                    }
                    let page_age = now.signed_duration_since(summary.end_time);
                    if page_age > retention_duration {
                        println!("[Retention] L{}/{}: Deleting (end_time: {}, age: {}s)", level, summary.page_id, summary.end_time, page_age.num_seconds());
                        if let Err(e) = self.storage.delete_page(level, summary.page_id).await {
                            eprintln!("[Retention] L{}/{}: Error deleting page: {}. Continuing.", level, summary.page_id, e);
                        }
                    }
                }
            }
            RollupRetentionPolicy::KeepNPages(n) => {
                if n == 0 {
                    println!("[Retention] L{}: Policy is KeepNPages(0). Deleting all pages.", level);
                    let page_summaries = match self.storage.list_finalized_pages_summary(level).await {
                        Ok(summaries) => summaries,
                        Err(e) => {
                            eprintln!("[Retention] L{}: Error listing for KeepNPages(0): {}. Skipping.", level, e);
                            continue; // Skip to next level if error listing pages
                        }
                    };
                    for summary in page_summaries {
                        println!("[Retention] L{}/{}: Deleting (KeepNPages(0))", level, summary.page_id);
                        if let Err(e) = self.storage.delete_page(level, summary.page_id).await {
                            eprintln!("[Retention] L{}/{}: Error deleting page: {}. Continuing.", level, summary.page_id, e);
                        }
                    }
                    continue; // Skip to next level after processing KeepNPages(0)
                }

                // Logic for n > 0
                println!("[Retention] L{}: Policy is KeepNPages({}).", level, n);
                let mut page_summaries = match self.storage.list_finalized_pages_summary(level).await {
                    Ok(summaries) => summaries,
                    Err(e) => {
                        eprintln!("[Retention] L{}: Error listing finalized pages for KeepNPages({}): {}. Skipping level.", level, n, e);
                        continue; // Skip to next level if error listing pages
                    }
                };

                if page_summaries.len() <= n {
                    println!("[Retention] L{}: Found {} pages, which is <= {}. No pages will be deleted.", level, page_summaries.len(), n);
                    // continue; // No, don't continue here, just fall through to end of match arm
                } else {
                    // Sort pages by end_time (oldest first) to identify which ones to delete
                    page_summaries.sort_unstable_by_key(|s| s.end_time);
                    
                    let num_to_delete = page_summaries.len() - n;
                    println!("[Retention] L{}: Found {} pages. Deleting {} oldest pages to keep {}.", level, page_summaries.len(), num_to_delete, n);

                    for summary_to_delete in page_summaries.iter().take(num_to_delete) {
                        println!("[Retention] L{}/{}: Deleting (KeepNPages, end_time: {})", level, summary_to_delete.page_id, summary_to_delete.end_time);
                        if let Err(e) = self.storage.delete_page(level, summary_to_delete.page_id).await {
                            eprintln!("[Retention] L{}/{}: Error deleting page: {}. Continuing.", level, summary_to_delete.page_id, e);
                        }
                    }
                }
            } // Closes RollupRetentionPolicy::KeepNPages(n) arm
        } // Closes match effective_policy
    } // Closes for (level_idx, level_config)

    println!("[Retention] Per-level policies application complete.");
    Ok(())
} // Closes async fn apply_retention_policies

    async fn perform_rollup(&self, finalized_page_level: u8, finalized_page_hash: [u8; 32], original_trigger_timestamp: DateTime<Utc>) -> Result<(), CJError> {
        println!("[ROLLUP-DEBUG] ENTER perform_rollup: level={}, hash={:.8}, trigger_ts={}", 
            finalized_page_level, 
            finalized_page_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>(), 
            original_trigger_timestamp);
        let parent_level_idx = finalized_page_level + 1;
        println!("[ROLLUP-DEBUG] Attempting rollup from L{} to L{}", finalized_page_level, parent_level_idx);

        // Check if the parent level exists in the configuration
        // Load the finalized child page from storage to access its content/details
    let loaded_child_page = match self.storage.load_page_by_hash(finalized_page_hash).await? {
        Some(page) => page,
        None => {
            // This case should ideally not happen if page_hash is valid and from a stored page.
            // However, if it does, we might log an error and stop the rollup for this path.
            eprintln!("perform_rollup: Failed to load child page with hash {:?}. Stopping rollup for this branch.", finalized_page_hash);
            // Consider returning a specific error or just Ok(()) if this is a recoverable scenario for the system.
            return Err(CJError::StorageError(format!("Failed to load child page with hash {:?} for rollup", finalized_page_hash)));
        }
    };

    let parent_level_config = match self.config.time_hierarchy.levels.get(parent_level_idx as usize) {
        Some(config) => config,
        None => {
            // Finalized page was at the highest configured level, so no further rollup.
            println!("[ROLLUP-DEBUG] No parent level L{} configured, stopping rollup cascade", parent_level_idx);
            return Ok(());
        }
    };
    println!("[ROLLUP-DEBUG] Found parent level L{} config with max_items={}, max_age={}s",
        parent_level_idx,
        parent_level_config.rollup_config.max_items_per_page,
        parent_level_config.rollup_config.max_page_age_seconds);

    // If there's an existing active parent page and the triggering timestamp is beyond its window, finalize it first
    {
        let mut active_pages_guard = self.active_pages.lock().await;
        if let Some(existing_parent) = active_pages_guard.get(&parent_level_idx).cloned() {
            if original_trigger_timestamp >= existing_parent.end_time {
                println!("[ROLLUP] Finalizing stale parent page L{}P{} before creating new one", existing_parent.level, existing_parent.page_id);
                active_pages_guard.remove(&parent_level_idx);
                drop(active_pages_guard);
                let mut page_to_store = existing_parent.clone();
                page_to_store.recalculate_merkle_root_and_page_hash();
                self.storage.store_page(&page_to_store).await?;
                self.last_finalized_page_ids.lock().await.insert(parent_level_idx, page_to_store.page_id);
                self.last_finalized_page_hashes.lock().await.insert(parent_level_idx, page_to_store.page_hash);
                // Cascade further up
                Box::pin(self.perform_rollup(parent_level_idx, page_to_store.page_hash, original_trigger_timestamp)).await?;
            } else {
                drop(active_pages_guard);
            }
        }
    }


        println!("[ROLLUP-DEBUG] PRE_CALL get_or_create_active_parent_page for L{}", parent_level_idx);
    let mut parent_page = self.get_or_create_active_parent_page(parent_level_idx, original_trigger_timestamp).await?;
    println!("[ROLLUP-DEBUG] POST_CALL get_or_create_active_parent_page for L{}. Got page L{}P{}", parent_level_idx, parent_page.level, parent_page.page_id);
        println!("[ROLLUP-DEBUG] Got parent page L{}P{} with {} existing items", parent_level_idx, parent_page.page_id, parent_page.content_len());

    // The timestamp to associate with the child's content in the parent page.
    // This should be the end_time of the child page, as per ROLLUP.md.
    let child_level_config = &self.config.time_hierarchy.levels[loaded_child_page.level as usize];
    println!("[ROLLUP_CHILD_END_TIME_DBG] Child L{}P{} (creation_ts: {}), child_level_config.duration_seconds = {}, page.end_time_field = {}",
        loaded_child_page.level,
        loaded_child_page.page_id,
        loaded_child_page.creation_timestamp,
        child_level_config.duration_seconds,
        loaded_child_page.end_time // Access field directly
    );
    let child_content_timestamp_for_parent = loaded_child_page.end_time; // Use the field

    match parent_level_config.rollup_config.content_type {
        RollupContentType::ChildHashes => {
            parent_page.add_thrall_hash(loaded_child_page.page_hash, child_content_timestamp_for_parent);
        }
        RollupContentType::NetPatches => {
            match loaded_child_page.content {
                PageContent::NetPatches(child_patches) => {
                    if !child_patches.is_empty() { // Only merge if there's something to merge
                        parent_page.merge_net_patches(child_patches, child_content_timestamp_for_parent);
                    }
                }
                PageContent::Leaves(leaves) => {
                // This is the case for L0 -> L1 (NetPatches).
                // We convert L0 leaves into a NetPatch representation.
                // Assumption: leaf.container_id is ObjectID.
                // Assumption: leaf.delta_payload is a serde_json::Value::Object where keys are FieldNames.
                let mut patches_from_leaves: std::collections::HashMap<String, std::collections::HashMap<String, serde_json::Value>> = std::collections::HashMap::new();

                for leaf in leaves {
                    if let serde_json::Value::Object(fields_map) = &leaf.delta_payload {
                        let object_id_patches = patches_from_leaves.entry(leaf.container_id.clone()).or_default();
                        for (field_name, field_value) in fields_map {
                            // Last write wins for the same object_id/field_name within this child page's leaves.
                            object_id_patches.insert(field_name.clone(), field_value.clone());
                        }
                    } else {
                        eprintln!(
                            "[Rollup] Warning: Leaf {} in L0 page {} (Hash: {:.8}) has non-object delta_payload ({:?}) when rolling up to NetPatches. Skipping this leaf for NetPatch generation.",
                            leaf.leaf_id, loaded_child_page.page_id, loaded_child_page.page_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>(), leaf.delta_payload
                        );
                    }
                }

                if !patches_from_leaves.is_empty() {
                    parent_page.merge_net_patches(patches_from_leaves, child_content_timestamp_for_parent);
                } else {
                    // It's okay if no patches are generated, e.g., if all leaves had non-object payloads
                    // or the child page was empty of leaves to begin with (though an empty L0 page might be discarded earlier).
                    println!(
                        "[Rollup] Info: No NetPatches generated from L0 page {} (Hash: {:.8}) for parent L1 page {} (Hash: {:.8}). Child page might have had no suitable leaves or was empty.",
                        loaded_child_page.page_id, loaded_child_page.page_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>(), 
                        parent_page.page_id, parent_page.page_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>()
                    );
                }
            }
                PageContent::ThrallHashes(_) => {
                    // Rolling up a ChildHashes page into a NetPatches page is problematic.
                    // This implies a misconfiguration or an unsupported rollup path.
                    eprintln!("Error: Cannot roll up PageContent::ThrallHashes into PageContent::NetPatches. Child page ID: {}. Parent Page ID: {}", loaded_child_page.page_id, parent_page.page_id);
                    // Potentially return an error or skip adding content.

                }
            }
        }
    }
    parent_page.recalculate_merkle_root_and_page_hash();
    let mut active_pages_guard = self.active_pages.lock().await;
        // Determine if this parent page itself needs to be finalized.
    let max_items_parent = parent_level_config.rollup_config.max_items_per_page;
    println!("[ROLLUP_DBG_AGE_SETUP_PRE_OLDEST] L{}P{}: parent_page.first_child_ts = {:?}, parent_page.creation_timestamp = {}", parent_page.level, parent_page.page_id, parent_page.first_child_ts, parent_page.creation_timestamp);
    let oldest_content_ts_in_parent = parent_page.first_child_ts.unwrap_or(parent_page.creation_timestamp);
    // Age is based on page creation time to ensure consistent finalization
    let parent_page_age_seconds = (original_trigger_timestamp - parent_page.creation_timestamp).num_seconds();
    println!("[ROLLUP_DBG_AGE_SETUP] L{}P{}: original_trigger_timestamp = {}, oldest_content_ts_in_parent = {}, calculated parent_page_age_seconds = {}", parent_page.level, parent_page.page_id, original_trigger_timestamp, oldest_content_ts_in_parent, parent_page_age_seconds);
    let max_age_parent_seconds = parent_level_config.rollup_config.max_page_age_seconds;

    println!("[ROLLUP_DBG_FULL] Checking parent_is_full for L{}P{}: parent_page.content_len() = {}, max_items_parent = {}", parent_page.level, parent_page.page_id, parent_page.content_len(), max_items_parent);
    let is_full_by_items = parent_page.content_len() >= max_items_parent;
    println!("[ROLLUP_DBG_AGE_CALC] Checking parent_is_over_age for L{}P{}: parent_page_age_seconds = {}, max_age_parent_seconds = {}, comparison_result = {}", parent_page.level, parent_page.page_id, parent_page_age_seconds, max_age_parent_seconds, parent_page_age_seconds >= max_age_parent_seconds as i64);
    let is_over_age = parent_page_age_seconds >= max_age_parent_seconds as i64;

    // A page should be considered for finalization if it's full or over age.
    // However, it's only *actually* finalized if it also has content.
    // An empty page that is over age is handled by discard logic.

    if parent_page.is_content_empty() {
        if is_over_age {
            // EMPTY PAGE DISCARD LOGIC: Page is empty AND over age.
            println!("[Rollup] Discarding empty page L{}/{} due to age. Original trigger: {:?}, Page creation: {:?}, First child_ts: {:?}", 
                parent_level_idx, parent_page.page_id, original_trigger_timestamp, parent_page.creation_timestamp, parent_page.first_child_ts);
            active_pages_guard.remove(&parent_level_idx);
            // Do not store, do not update last_finalized_*, do not recurse.
            drop(active_pages_guard);
            return Ok(());
        } else {
            // Page is empty but NOT over age. Keep it active.
            // This can happen if a child page caused a parent to be created, but the child's content
            // didn't actually populate the parent (e.g. L0 leaves -> NetPatches, but all leaves were unsuitable).
            // The parent is empty but not old enough to discard.
            println!("[Rollup] Keeping empty page L{}/{} active (not over age). Original trigger: {:?}", 
                parent_level_idx, parent_page.page_id, original_trigger_timestamp);
            active_pages_guard.insert(parent_level_idx, parent_page.clone());
            drop(active_pages_guard); // Release lock as this branch is done with active_pages
        }
    } else { // Page is NOT empty
        if is_full_by_items || is_over_age {
            // NORMAL FINALIZATION (not empty, AND (full or over age)):
            println!(
                "[Rollup] Finalizing non-empty page L{}/{} (Full: {}, OverAge: {}). Original trigger: {:?}",
                parent_level_idx,
                parent_page.page_id,
                is_full_by_items,
                is_over_age,
                original_trigger_timestamp
            );
            let parent_page_to_store = parent_page.clone();

            println!(
                "[ROLLUP-DEBUG] Removing L{}P{} from active_pages before cascade",
                parent_level_idx,
                parent_page_to_store.page_id
            );
            active_pages_guard.remove(&parent_level_idx); // Remove from active as it's now finalized
            println!(
                "[ROLLUP-DEBUG] L{}P{} removed from active_pages successfully (under lock).",
                parent_level_idx,
                parent_page_to_store.page_id
            );
            drop(active_pages_guard); // Release lock before awaiting storage

            println!(
                "[ROLLUP-DEBUG] STORING PAGE L{}P{} to storage",
                parent_level_idx,
                parent_page_to_store.page_id
            );
            self.storage.store_page(&parent_page_to_store).await?;
            println!(
                "[ROLLUP-DEBUG] Page L{}P{} stored successfully",
                parent_level_idx,
                parent_page_to_store.page_id
            );

            self
                .last_finalized_page_ids
                .lock()
                .await
                .insert(parent_level_idx, parent_page_to_store.page_id);
            self
                .last_finalized_page_hashes
                .lock()
                .await
                .insert(parent_level_idx, parent_page_to_store.page_hash);

            println!(
                "[ROLLUP-DEBUG] Starting recursive rollup call to L{} with parent hash {:.8}",
                parent_level_idx + 1,
                parent_page_to_store
                    .page_hash
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>()
            );
            Box::pin(
                self.perform_rollup(
                    parent_level_idx,
                    parent_page_to_store.page_hash,
                    original_trigger_timestamp,
                )
            )
            .await?;
            println!("[ROLLUP-DEBUG] Returned from recursive rollup call to L{}", parent_level_idx + 1);
        } else {
            // Page is NOT empty, NOT full, and NOT over age. Keep it active.
            println!("[Rollup] Keeping non-empty page L{}/{} active (not full, not over age). Original trigger: {:?}", 
                parent_level_idx, parent_page.page_id, original_trigger_timestamp);
            active_pages_guard.insert(parent_level_idx, parent_page.clone());
            println!("[ROLLUP-DEBUG] Inserted/Updated L{}P{} in active_pages (under lock).", parent_level_idx, parent_page.page_id);
            drop(active_pages_guard); // Release lock as this branch is done with active_pages
        }
    }

    println!("[ROLLUP-DEBUG] EXIT perform_rollup: level={}, hash={:.8}", 
        finalized_page_level, 
        finalized_page_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>());
    Ok(())
}

} // Closes impl TimeHierarchyManager

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::page::{JournalPage, PageContent};
    use crate::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids}; // Import shared test items
    use crate::storage::memory::MemoryStorage; // For creating an instance
    use std::sync::atomic::Ordering; // If used directly, otherwise covered by reset_global_ids
    use chrono::Duration;
    use chrono::TimeZone; // Cascade: Added for with_ymd_and_hms
    use serde_json::json; // ADDED for json! macro
    use crate::config::{Config, RetentionConfig, StorageConfig as TestStorageConfig, CompressionConfig, LoggingConfig, MetricsConfig};
    use crate::StorageType as TestStorageType; // Re-exported at crate root
    use crate::CompressionAlgorithm; // Re-exported at crate root
    use crate::TimeLevel;
    use crate::TimeHierarchyConfig;
    // Note: This directly creates a JournalPage. For manager tests, you'd typically add leaves.
    // However, for retention, we need to control page end_times precisely.
    fn create_retention_test_page(level: u8, page_id_offset: u64, time_window_start: DateTime<Utc>, config: &Config) -> JournalPage {
        let gen = PageIdGenerator::new();
        for _ in 0..page_id_offset {
            gen.next();
        }
        JournalPage::new_with_id(
            gen.next(),
            level,
            None,
            time_window_start,
            config,
        )
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
                        rollup_config: LevelRollupConfig {
                            max_items_per_page: 1,
                            max_page_age_seconds: 1000, // Match global test default
                            content_type: RollupContentType::ChildHashes,
                        },
                        retention_policy: l0_policy,
                        name: "seconds".to_string(),
                        duration_seconds: 1
                    },
                    TimeLevel {
                        rollup_config: LevelRollupConfig {
                            max_items_per_page: 1,
                            max_page_age_seconds: 1000, // Match global test default
                            content_type: RollupContentType::ChildHashes,
                        },
                        retention_policy: l1_policy,
                        name: "minutes".to_string(),
                        duration_seconds: 60
                    },
                ],
            },
            force_rollup_on_shutdown: false,
            storage: TestStorageConfig {
                storage_type: TestStorageType::Memory,
                base_path: "".to_string(),
                max_open_files: 100, // Default from StorageConfig
            },
            retention: RetentionConfig {
                enabled: retention_enabled,
                period_seconds: retention_period_seconds,
                cleanup_interval_seconds: 300, // Default, not typically used in these specific tests
            },
            compression: CompressionConfig::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
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
            println!("[TEST-DEBUG] Starting test_add_leaf_to_new_page_and_check_active");
            let _guard = crate::test_utils::acquire_test_mutex("test_add_leaf_to_new_page_and_check_active").await;
            reset_global_ids();
    
            println!("[TEST-DEBUG] Creating test manager and storage");
            let (manager, storage) = create_test_manager(); // Config from create_test_manager implies mlpp=1 for L0 & L1
            let now = Utc::now();
            println!("[TEST-DEBUG] Test time: {}", now);
    
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
            assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active after L0P0 finalization");
    
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
            assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "No L1 page should be active after L1P1 finalization (L1 is highest configured level)");
            assert!(manager.active_pages.lock().await.get(&2u8).is_none(), "No L2 page should exist as L2 is not configured by create_test_manager");
    
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
            assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active after L0P0 finalization");
    
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
            assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active");
    
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
            assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "No L1 page should be active");
            assert!(manager.active_pages.lock().await.get(&2u8).is_none(), "No L2 page should exist");
    
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
            assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active");
            
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
            assert_eq!(stored_l1_p5.prev_page_hash, Some(stored_l1_p3.page_hash), "L1P5 prev_page_hash should be L1P3's hash");
            assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "No L1 page should be active");
        
            // Check last finalized for L0 and L1 after leaf 3
            assert_eq!(manager.last_finalized_page_ids.lock().await.get(&0u8).copied().unwrap(), stored_l0_p4.page_id, "L0 last_finalized_page_id mismatch after leaf3");
            assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&0u8).copied().unwrap(), stored_l0_p4.page_hash, "L0 last_finalized_page_hash mismatch after leaf3");
            assert_eq!(manager.last_finalized_page_ids.lock().await.get(&1u8).copied().unwrap(), stored_l1_p5.page_id, "L1 last_finalized_page_id mismatch after leaf3");
            assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&1u8).copied().unwrap(), stored_l1_p5.page_hash, "L1 last_finalized_page_hash mismatch after leaf3");
    }

fn create_cascading_test_config_and_manager() -> (TimeHierarchyManager, Arc<MemoryStorage>) {
    let config_val = Config {
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                TimeLevel {
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 1,
                        max_page_age_seconds: 1000,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None, name: "L0".to_string(), duration_seconds: 1 },
                TimeLevel {
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 1,
                        max_page_age_seconds: 1000,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None, name: "L1".to_string(), duration_seconds: 1 },
                TimeLevel {
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 1,
                        max_page_age_seconds: 1000,
                        content_type: RollupContentType::ChildHashes,
                    },
                    retention_policy: None, name: "L2".to_string(), duration_seconds: 1 },
            ],
        },
        force_rollup_on_shutdown: false,
        storage: TestStorageConfig {
            storage_type: TestStorageType::Memory,
            base_path: "".to_string(),
            max_open_files: 100, // Default from StorageConfig
        },
        retention: RetentionConfig { // Default retention disabled for these rollup tests
            enabled: false,
            period_seconds: 0,
            // REMOVED: default_max_age_seconds: 0,
            cleanup_interval_seconds: 300, // Assuming a default, consistent with create_test_config
        },
        compression: CompressionConfig::default(),
        logging: LoggingConfig::default(),
        metrics: MetricsConfig::default(),
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
        assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "L0 should have no active page after cascade");
        assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "L1 should have no active page after cascade");
        assert!(manager.active_pages.lock().await.get(&2u8).is_none(), "L2 should have no active page after cascade (top level finalized)");

        // Check last_finalized_page_ids and last_finalized_page_hashes
        assert_eq!(manager.last_finalized_page_ids.lock().await.get(&0u8).copied().expect("L0 ID not in last_finalized_page_ids after cascade"), 0, "L0 last finalized ID mismatch");
        assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&0u8).copied().expect("L0 hash not in last_finalized_page_hashes after cascade"), stored_l0_page.page_hash, "L0 last finalized hash mismatch");
        
        assert_eq!(manager.last_finalized_page_ids.lock().await.get(&1u8).copied().expect("L1 ID not in last_finalized_page_ids after cascade"), 1, "L1 last finalized ID mismatch");
        assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&1u8).copied().expect("L1 hash not in last_finalized_page_hashes after cascade"), stored_l1_page.page_hash, "L1 last finalized hash mismatch");

        assert_eq!(manager.last_finalized_page_ids.lock().await.get(&2u8).copied().expect("L2 ID not in last_finalized_page_ids after cascade"), 2, "L2 last finalized ID mismatch");
        assert_eq!(manager.last_finalized_page_hashes.lock().await.get(&2u8).copied().expect("L2 hash not in last_finalized_page_hashes after cascade"), stored_l2_page.page_hash, "L2 last finalized hash mismatch");
    }

    #[tokio::test]
    async fn test_retention_disabled() {
        println!("[TEST-DEBUG] Starting test_retention_disabled");
        let _guard = crate::test_utils::acquire_test_mutex("test_retention_disabled").await;
        reset_global_ids();

        println!("[TEST-DEBUG] Creating config with retention disabled");
        let config = Arc::new(create_test_config(false, 60, None, None)); // Retention disabled, 60s period (irrelevant)
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());
        println!("[TEST-DEBUG] Manager created successfully");

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
        let config = Arc::new(create_test_config(
            true, // global retention enabled
            0,   // global retention period_seconds (should not affect L0)
            None, // L0 policy
            None, // L1 policy
        )); 
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
        println!("[TEST-DEBUG] Starting test_retention_all_pages_deleted");
        let _guard = crate::test_utils::acquire_test_mutex("test_retention_all_pages_deleted").await;
        reset_global_ids();

        println!("[TEST-DEBUG] Creating config with retention period {}s", 60);
        let retention_period_seconds = 60;
        let config = Arc::new(create_test_config(true, retention_period_seconds, None, None));
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());
        println!("[TEST-DEBUG] Manager and storage created successfully");

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
        println!("[TEST-DEBUG] Starting test_retention_l0_keep_n_pages");
        let _guard = crate::test_utils::acquire_test_mutex("test_retention_l0_keep_n_pages").await;
        reset_global_ids();

        println!("[TEST-DEBUG] Creating config with L0 KeepNPages({})", 2);
        let keep_n = 2;
        // Global retention is enabled but L0 will use its specific KeepNPages policy.
        // L1 will use global (DeleteAfterSecs(60)) or KeepIndefinitely if global is off.
        let config = Arc::new(create_test_config(
            true, // global retention enabled
            60,   // global retention period_seconds (should not affect L0)
            Some(RollupRetentionPolicy::KeepNPages(keep_n)), // L0 policy
            None, // L1 policy (will use global or default)
        ));
        println!("[TEST-DEBUG] Creating storage and manager");
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());
        println!("[TEST-DEBUG] Manager created successfully");

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
    } // End of test_retention_mixed_policies_l0_keep_n_l1_delete_after_secs

    // Helper function to create a configuration suitable for testing age-based rollups
    fn create_age_based_rollup_config_and_manager(
        l0_duration_seconds: u64,
        l0_max_leaves: u32,
        l0_max_age_secs: u64,
        l1_duration_seconds: u64,
        l1_max_leaves: u32,
        l1_max_age_secs: u64,
    ) -> (TimeHierarchyManager, Arc<MemoryStorage>) {
        let config_val = Config {
            time_hierarchy: TimeHierarchyConfig {
                levels: vec![
                    TimeLevel { // Level 0
                        name: "L0_age_test".to_string(),
                        duration_seconds: l0_duration_seconds,
                        rollup_config: LevelRollupConfig {
                            max_items_per_page: l0_max_leaves as usize,
                            max_page_age_seconds: l0_max_age_secs,
                            content_type: RollupContentType::ChildHashes,
                        },
                        retention_policy: None,
                    },
                    TimeLevel { // Level 1
                        name: "L1_age_test".to_string(),
                        duration_seconds: l1_duration_seconds,
                        rollup_config: LevelRollupConfig {
                            max_items_per_page: l1_max_leaves as usize,
                            max_page_age_seconds: l1_max_age_secs,
                            content_type: RollupContentType::ChildHashes,
                        },
                        retention_policy: None,
                    },
                ],
            },
            force_rollup_on_shutdown: false,
            storage: TestStorageConfig {
                storage_type: TestStorageType::Memory,
                base_path: "".to_string(),
                max_open_files: 100, // Default from StorageConfig
            },
            retention: RetentionConfig {
                enabled: false, // Keep retention disabled for these rollup tests
                period_seconds: 0,
                cleanup_interval_seconds: 300,
            },
            compression: CompressionConfig::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
        };
        let config = Arc::new(config_val);
        let storage = Arc::new(MemoryStorage::new());
        let manager = TimeHierarchyManager::new(config.clone(), storage.clone());
        (manager, storage)
    }

    #[tokio::test]
    async fn test_age_based_rollup_cascade() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        let l0_max_age = 2; // L0 pages finalize after 2 seconds
        let l1_max_age = 4; // L1 pages finalize after 4 seconds (relative to their creation)
        // Set max_leaves high so finalization is age-based
        let (manager, storage) = create_age_based_rollup_config_and_manager(
            60,     // l0_duration_seconds (60s so leaf2 falls in same window)
            100,    // l0_max_leaves
            l0_max_age, // l0_max_age_secs (2s)
            60,     // l1_duration_seconds (default 60s)
            100,    // l1_max_leaves
            l1_max_age  // l1_max_age_secs (4s)
        );

        let initial_time = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(); // Cascade: Use fixed time for deterministic test

        // 1. Add leaf1 to L0
        let leaf1_ts = initial_time;
        let leaf1 = JournalLeaf::new(leaf1_ts, None, "age_test_container".to_string(), json!({"id": 1})).unwrap();
        manager.add_leaf(&leaf1).await.unwrap();

        // L0P0 (ID 0) should be active, not yet finalized
        assert!(manager.active_pages.lock().await.get(&0u8).is_some(), "L0P0 should be active initially");
        assert_eq!(manager.active_pages.lock().await.get(&0u8).unwrap().page_id, 0, "Active L0 page should be ID 0");
        assert!(storage.load_page(0, 0).await.unwrap().is_none(), "L0P0 should not be in storage yet");

        // 2. Wait for L0P0 to age past its max_page_age_seconds
        tokio::time::sleep(std::time::Duration::from_secs(l0_max_age + 1)).await;

        // 3. Add leaf2. This should trigger finalization of L0P0 due to age.
        // L0P0's rollup will create/update L1P1 (ID 1).
        let leaf2_ts = initial_time + Duration::seconds((l0_max_age + 1).try_into().unwrap()); // Cascade: Ensure leaf2 is timed to finalize L0P0
        let leaf2 = JournalLeaf::new(leaf2_ts, None, "age_test_container".to_string(), json!({"id": 2})).unwrap();
        manager.add_leaf(&leaf2).await.unwrap();

        // L0P0 (ID 0) should now be finalized and stored
        let stored_l0p0 = storage.load_page(0, 0).await.unwrap().expect("L0P0 (ID 0) should be stored after aging");
        assert_eq!(stored_l0p0.page_id, 0);
        // Check content of L0P0 - it should contain both leaf1 and leaf2, as leaf2 was added, then L0P0 finalized by age.
        match stored_l0p0.content {
            PageContent::Leaves(leaves) => {
                assert_eq!(leaves.len(), 2, "L0P0 should contain leaf1 and leaf2");
                assert_eq!(leaves[0].leaf_hash, leaf1.leaf_hash, "L0P0 leaf1 content mismatch");
                assert_eq!(leaves[1].leaf_hash, leaf2.leaf_hash, "L0P0 leaf2 content mismatch");
            }
            _ => panic!("L0P0 content should be Leaves"),
        }
        
        // After L0P0 finalizes (containing leaf1 and leaf2), no L0 page should be active until a new leaf is added.
        assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "No L0 page should be active after L0P0 finalized");
        // The next leaf added would create L0P1 (as L0P0 was ID 0, and L1P1 from rollup would be ID 1)
        
        // L1P1 (ID 1) should have been created by L0P0's rollup and should be active
        let l1p1_active_page = manager.active_pages.lock().await.get(&1u8).expect("L1P1 (ID 1) should be active after L0P0 rollup").clone();
        assert_eq!(l1p1_active_page.page_id, 1, "Active L1 page should be L1P1 (ID 1)");
        let l1p1_creation_time = l1p1_active_page.creation_timestamp;
        assert!(storage.load_page(1, 1).await.unwrap().is_none(), "L1P1 should not be in storage yet");

        // Add leaf3. This will create L0P2 (ID 2, as L0P0 was 0, L1P1 is 1).
        // Timestamp it to be young within the next minute from initial_time to ensure L0P2 doesn't auto-finalize.
        let leaf3_ts = initial_time + Duration::minutes(1) + Duration::seconds(1);
        let leaf3 = JournalLeaf::new(leaf3_ts, None, "age_test_container".to_string(), json!({"id": 3})).unwrap();
        manager.add_leaf(&leaf3).await.unwrap();

        // L0P2 should now be active
        let l0p2_active_page = manager.active_pages.lock().await.get(&0u8).expect("L0P2 (for leaf3) should be active").clone();
        assert_eq!(l0p2_active_page.page_id, 2, "Active L0 page should be L0P2 (ID 2)");
        let l0p2_creation_time = l0p2_active_page.creation_timestamp;

        // 4. Wait for L1P1 to age past its max_page_age_seconds
        // Calculate how long L1P1 has been "alive" in test-logical time up to leaf3_ts (current logical time point)
        let l1p1_age_at_leaf3_ts_s = leaf3_ts.signed_duration_since(l1p1_creation_time).num_seconds();
        let target_l1p1_total_age_s = l1_max_age as i64 + 1; // Target age for L1P1 to ensure it's old enough
        let mut additional_wait_for_l1_s: i64 = 0;

        if l1p1_age_at_leaf3_ts_s < target_l1p1_total_age_s {
            additional_wait_for_l1_s = target_l1p1_total_age_s - l1p1_age_at_leaf3_ts_s;
        }
        // If l1p1_age_at_leaf3_ts_s is already >= target_l1p1_total_age_s, additional_wait_for_l1_s remains 0.
        
        if additional_wait_for_l1_s > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(additional_wait_for_l1_s as u64)).await;
        }
        
        // 5. Now, L0P2 (containing leaf3) is active. We need it to finalize by age to trigger a rollup to L1P1.
        // L1P1 has already been made to wait past its age limit (step 4).
        // So, the rollup from L0P2 should trigger L1P1's finalization.

        // First, wait for L0P2 to age sufficiently.
        // l0p2_creation_time was captured when L0P2 was created (with leaf3).
        let mut time_to_wait_for_l0p2 = l0_max_age + 1;
        // Calculate elapsed time since l0p2_creation_time (which is initial_time + 1 minute)
        // Utc::now() is tricky here due to test execution speed. 
        // Instead, base the sleep on the *known* l0p2_creation_time and the *current* leaf3_ts.
        // leaf3_ts is initial_time + 1min + 1sec. l0p2_creation_time is initial_time + 1min.
        // So, L0P2 is currently 1 second old. We need it to be l0_max_age + 1 seconds old.
        // If l0_max_age is 2s, we need it to be 3s old. It's 1s old. So wait 2s.
        let current_l0p2_age = leaf3_ts.signed_duration_since(l0p2_creation_time).num_seconds() as u64;
        if time_to_wait_for_l0p2 > current_l0p2_age {
            time_to_wait_for_l0p2 -= current_l0p2_age;
        } else {
            time_to_wait_for_l0p2 = 0; // Already aged enough or past, no wait needed before adding leaf4
        }
        if time_to_wait_for_l0p2 > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(time_to_wait_for_l0p2)).await;
        }

        // Now add leaf4 to L0P2. This will make L0P2 finalize by age.
        // leaf4_ts should be l0p2_creation_time + l0_max_age + 1 second.
        let leaf4_ts = l0p2_creation_time + Duration::seconds((l0_max_age + 1).try_into().unwrap());
        let leaf4 = JournalLeaf::new(leaf4_ts, None, "age_test_container".to_string(), json!({"id": 4})).unwrap();
        manager.add_leaf(&leaf4).await.unwrap();

        // L0P2 should be finalized. No L0 page active.
        assert!(manager.active_pages.lock().await.get(&0u8).is_none(), "L0P2 should be finalized after leaf4");
        
        // The rollup from L0P2 should have triggered L1P1's age-based finalization.
        // So, L1P1 should also not be active.
        let active_l1_page_id = manager.active_pages.lock().await.get(&1u8).map(|p| p.page_id);
        assert!(active_l1_page_id != Some(1), "L1P1 should be finalized by age after L0P2's rollup");

        // 6. L1P1 should now be stored.
        let stored_l1p1 = storage.load_page(1, 1).await.unwrap().expect("L1P1 (ID 1) should be stored after aging and rollup");
        assert_eq!(stored_l1p1.page_id, 1, "Stored L1P1 page ID should be 1");
        assert_eq!(stored_l1p1.level, 1, "Stored L1P1 level should be 1");
        match stored_l1p1.content {
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 1, "L1P1 should contain one thrall hash from L0P0");
                // Load L0P0 (which was stored much earlier) to get its page hash for comparison.
                // Thrall hashes store the finalized child page's page hash, not the Merkle root.
                let l0p0_finalized = storage
                    .load_page(0, 0)
                    .await
                    .unwrap()
                    .expect("L0P0 (ID 0) should still be stored");
                assert_eq!(
                    hashes[0],
                    l0p0_finalized.page_hash,
                    "L1P1 thrall hash should match L0P0's page hash"
                );
            }
            _ => panic!("L1P1 content should be ThrallHashes"),
        }
    }

    #[tokio::test]
    async fn test_parent_page_accumulates_multiple_thralls() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await;
        reset_global_ids();

        // L0: finalize after 1 leaf.
        // L1: finalize after 3 thralls (from L0 pages). Age limits are high.
        let l0_max_leaves = 1;
        let l1_max_leaves = 3;
        let high_age_limit = 3600; // 1 hour, effectively infinite for this test

        let (manager, storage) = create_age_based_rollup_config_and_manager(
            60,             // l0_duration_seconds (default 60s)
            l0_max_leaves,  // l0_max_leaves (1 for this test)
            high_age_limit, // l0_max_age_secs
            3600,           // l1_duration_seconds (default 3600s)
            l1_max_leaves,  // l1_max_leaves (3 for this test)
            high_age_limit  // l1_max_age_secs
        );

        let time_base = Utc::now();

        // Leaf 1 -> L0P0 (ID 0) finalizes -> L1P1 (ID 1) gets 1st thrall.
        let leaf1 = JournalLeaf::new(time_base, None, "parent_accum_container".to_string(), json!({"id": "L0P0_leaf1"})).unwrap();
        manager.add_leaf(&leaf1).await.unwrap();

        let stored_l0p0 = storage.load_page(0, 0).await.unwrap().expect("L0P0 (ID 0) should be stored");
        assert_eq!(stored_l0p0.page_id, 0);
        let active_pages_guard = manager.active_pages.lock().await;
        let active_l1_page = active_pages_guard.get(&1u8).expect("L1P1 (ID 1) should be active");
        assert_eq!(active_l1_page.page_id, 1, "Active L1 page should be L1P1 (ID 1)");
        match &active_l1_page.content {
            PageContent::ThrallHashes(hashes) => assert_eq!(hashes.len(), 1, "L1P1 should have 1 thrall hash"),
            _ => panic!("L1P1 content should be ThrallHashes"),
        }
        assert!(storage.load_page(1, 1).await.unwrap().is_none(), "L1P1 should not be stored yet");

        // Leaf 2 -> L0P2 (ID 2) finalizes -> L1P1 (ID 1) gets 2nd thrall.
        let leaf2_ts = time_base + chrono::Duration::seconds(1); // Ensure different page window if L0 duration is short
        let leaf2 = JournalLeaf::new(leaf2_ts, None, "parent_accum_container".to_string(), json!({"id": "L0P2_leaf1"})).unwrap();
        manager.add_leaf(&leaf2).await.unwrap();

        let stored_l0p2 = storage.load_page(0, 2).await.unwrap().expect("L0P2 (ID 2) should be stored");
        assert_eq!(stored_l0p2.page_id, 2);
        let active_pages_guard_updated = manager.active_pages.lock().await;
        let active_l1_page_updated = active_pages_guard_updated.get(&1u8).expect("L1P1 (ID 1) should still be active");
        assert_eq!(active_l1_page_updated.page_id, 1, "Active L1 page should still be L1P1 (ID 1)");
        match &active_l1_page_updated.content {
            PageContent::ThrallHashes(hashes) => assert_eq!(hashes.len(), 2, "L1P1 should have 2 thrall hashes"),
            _ => panic!("L1P1 updated content should be ThrallHashes"),
        }
        assert!(storage.load_page(1, 1).await.unwrap().is_none(), "L1P1 should still not be stored yet");

        // Leaf 3 -> L0P4 (ID 4) finalizes -> L1P1 (ID 1) gets 3rd thrall. L1P1 should finalize.
        let leaf3_ts = time_base + chrono::Duration::seconds(2);
        let leaf3 = JournalLeaf::new(leaf3_ts, None, "parent_accum_container".to_string(), json!({"id": "L0P4_leaf1"})).unwrap();
        manager.add_leaf(&leaf3).await.unwrap();

        let stored_l0p4 = storage.load_page(0, 4).await.unwrap().expect("L0P4 (ID 4) should be stored");
        assert_eq!(stored_l0p4.page_id, 4);

        // L1P1 (ID 1) should now be finalized and stored.
        let stored_l1p1 = storage.load_page(1, 1).await.unwrap().expect("L1P1 (ID 1) should be stored after 3rd thrall");
        assert_eq!(stored_l1p1.page_id, 1);
        match &stored_l1p1.content {
            PageContent::ThrallHashes(hashes) => {
                assert_eq!(hashes.len(), 3, "L1P1 should contain 3 thrall hashes");
                assert_eq!(hashes[0], stored_l0p0.page_hash, "L1P1 thrall 1 mismatch");
                assert_eq!(hashes[1], stored_l0p2.page_hash, "L1P1 thrall 2 mismatch");
                assert_eq!(hashes[2], stored_l0p4.page_hash, "L1P1 thrall 3 mismatch");
            }
            _ => panic!("L1P1 content mismatch, expected ThrallHashes"),
        }

        // After L1P1 finalizes, the active_pages entry for level 1 should be removed.
        // A new L1 page will only be created when the *next* L0 page finalizes and rolls up.
        assert!(manager.active_pages.lock().await.get(&1u8).is_none(), "Active L1 page should be None after L1P1 finalized and before next L0 rollup");

        // Leaf 4 -> L0P5 (ID 5) finalizes -> new L1P3 (ID 3) should be created.
        let leaf4_ts = time_base + chrono::Duration::seconds(3);
        let leaf4 = JournalLeaf::new(leaf4_ts, None, "parent_accum_container".to_string(), json!({"id": "L0P5_leaf1"})).unwrap();
        manager.add_leaf(&leaf4).await.unwrap();

        let _stored_l0p5 = storage.load_page(0, 5).await.unwrap().expect("L0P5 (ID 5) should be stored");

        let new_active_pages_guard = manager.active_pages.lock().await;
        let new_active_l1_page = new_active_pages_guard.get(&1u8).expect("A new L1 page (L1P3) should be active");
        assert_eq!(new_active_l1_page.page_id, 3, "New active L1 page should be L1P3 (ID 3)");
        match &new_active_l1_page.content {
            PageContent::ThrallHashes(hashes) => assert_eq!(hashes.len(), 1, "L1P3 should have 1 thrall hash from L0P5"),
            _ => panic!("L1P3 content should be ThrallHashes"),
        }
    }

}
