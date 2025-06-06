//! Query engine implementation for the CivicJournal time-series database.
//!
//! Provides functionality for executing complex queries against the journal,
//! including cryptographic proof generation and verification.

use crate::query::types::{
    QueryError, LeafInclusionProof, ReconstructedState, QueryPoint, DeltaReport,
    PageIntegrityReport,
};
use crate::config::Config;
use crate::storage::StorageBackend;
use crate::core::time_manager::TimeHierarchyManager;
use std::sync::Arc;
use chrono::{DateTime, Utc};
use serde_json::Value;

/// The main query engine for executing complex queries against the journal.
///
/// This struct provides methods for querying journal data and generating
/// cryptographic proofs of inclusion and consistency. It works with any
/// storage backend that implements the `StorageBackend` trait.
///
/// # Type Parameters
#[derive(Clone, Debug)]
pub struct QueryEngine {
    /// The storage backend used to retrieve journal pages
    storage: Arc<dyn StorageBackend>,
    
    /// The time hierarchy manager for locating pages in the time-based hierarchy
    time_manager: Arc<TimeHierarchyManager>,
    
    /// Application configuration
    _config: Arc<Config>,
}

impl QueryEngine {
    /// Creates a new `QueryEngine` with the specified storage, time manager, and configuration.
    ///
    /// # Arguments
    /// * `storage` - The storage backend for retrieving journal pages
    /// * `time_manager` - The time hierarchy manager for locating pages
    /// * `config` - Application configuration
    pub fn new(
        storage: Arc<dyn StorageBackend>,
        time_manager: Arc<TimeHierarchyManager>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            storage,
            time_manager,
            _config: config,
        }
    }

    /// Generates an inclusion proof for a specific leaf in the journal.
    ///
    /// This method searches for a leaf with the given hash and constructs
    /// a cryptographic proof of its inclusion in the journal.
    ///
    /// # Arguments
    /// * `leaf_hash` - The hash of the leaf to generate a proof for
    ///
    /// # Returns
    /// A `LeafInclusionProof` containing the proof if the leaf is found,
    /// or a `QueryError` if the leaf is not found or an error occurs.
    ///
    /// # Note
    /// The current implementation searches through level 0 pages. Future
    /// versions may support more efficient lookups using hints or indexing.
    pub async fn get_leaf_inclusion_proof(
        &self,
        leaf_hash: &[u8; 32],
        // TODO: Implement page_id_hint for more efficient lookups
        // page_id_hint: Option<(u8, u64)>,
    ) -> Result<LeafInclusionProof, QueryError> {
        // TODO: Implement more efficient page searching, possibly with a timestamp hint
        // or by querying TimeHierarchyManager for pages within a relevant time range if known.
        // For now, we iterate through known level 0 pages.

        let mut level0_page_ids_to_check: Vec<u64> = Vec::new();

        // Collect active level 0 page ID (if any)
        if let Some(active_page_id) = self.time_manager.get_current_active_page_id(0).await {
            level0_page_ids_to_check.push(active_page_id);
        }

        // Collect finalized level 0 pages from storage
        let finalized_pages = self.storage.list_finalized_pages_summary(0).await?;
        level0_page_ids_to_check.extend(finalized_pages.into_iter().map(|p| p.page_id));

        // Deduplicate in case a page was active and then immediately archived (edge case)
        level0_page_ids_to_check.sort_unstable();
        level0_page_ids_to_check.dedup();

        for page_id in level0_page_ids_to_check {
            match self.storage.load_page(0, page_id).await {
                Ok(Some(page)) => { 
                    let mut leaf_hashes_in_page = Vec::new();
                    let mut found_leaf_in_page_contents = false;

                    // Iterate over content to find the leaf and collect all leaf hashes for Merkle proof
                    // The 'page' variable is from the Ok(Some(page)) match arm and is guaranteed to be JournalPage.
                    match &page.content {
                        crate::core::page::PageContent::Leaves(leaves) => {
                            for leaf_in_page in leaves {
                                leaf_hashes_in_page.push(leaf_in_page.leaf_hash);
                                if leaf_in_page.leaf_hash == *leaf_hash { // Dereference leaf_hash
                                    found_leaf_in_page_contents = true;
                                }
                            }
                        }
                        crate::core::page::PageContent::ThrallHashes(hashes) => {
                            // This page contains thrall hashes, so it cannot directly contain the target JournalLeaf.
                            // For the Merkle proof of *this* page, we would use these thrall hashes.
                            // Since we're trying to prove leaf_to_prove, and it's not here,
                            // found_leaf_in_page_contents will remain false.
                            leaf_hashes_in_page.extend(hashes.iter().cloned());
                        }
                        crate::core::page::PageContent::NetPatches(_net_patches_content) => {
                            // L0 pages should ideally only contain Leaves. NetPatches here is unexpected.
                            eprintln!(
                                "[QueryEngine] Warning: Encountered PageContent::NetPatches in L0 page (ID: {}) during find_leaf_proof. This is unexpected for L0.",
                                page.page_id
                            );
                            // NetPatches content (Vec<u8>) doesn't directly provide leaf hashes for Merkle proof of a *JournalLeaf*.
                            // found_leaf_in_page_contents will remain false, so this page won't be considered to contain the target leaf.
                        }
                    }

                    if found_leaf_in_page_contents {
                        let leaf_idx = leaf_hashes_in_page.iter().position(|x| x == leaf_hash).unwrap();
                        
                        let merkle_tree = crate::core::merkle::MerkleTree::new(leaf_hashes_in_page)
                            .map_err(|e| QueryError::CoreError(e))?;
                        
                        if let Some(merkle_proof_for_leaf) = merkle_tree.get_proof(leaf_idx) {
                            // Attempt to load the full JournalLeaf from storage
                            let actual_leaf = self.storage.load_leaf_by_hash(leaf_hash).await?;

                            // If the leaf is not found in storage, this is an inconsistency or error,
                            // as we found its hash in a page. For now, we'll return an error.
                            // A more robust solution might involve how leaves are guaranteed to be stored
                            // if their hash is in a page, or a different error type.
                            let journal_leaf = actual_leaf.ok_or_else(|| QueryError::LeafDataNotFound(*leaf_hash))?;

                            return Ok(LeafInclusionProof {
                                leaf: journal_leaf, // Use the actual leaf from storage
                                page_id: page.page_id,
                                level: page.level,
                                proof: merkle_proof_for_leaf,
                                page_merkle_root: page.merkle_root,
                            });
                        } else {
                            eprintln!("Failed to generate Merkle proof for a found leaf. Page ID: {}, Leaf Index: {}", page.page_id, leaf_idx);
                        }
                    }
                }
                Ok(None) => {
                     eprintln!("Page not found in storage: ID {}, Level 0", page_id);
                }
                Err(e) => {
                    eprintln!("Failed to load page {}: {:?}", page_id, e);
                }
            }
        }

        Err(QueryError::LeafNotFound(*leaf_hash))
    }

    fn apply_delta(state: &mut Value, delta: &Value) {
        match (state, delta) {
            (Value::Object(b), Value::Object(p)) => {
                for (k, v) in p {
                    match b.get_mut(k) {
                        Some(existing) => Self::apply_delta(existing, v),
                        None => { b.insert(k.clone(), v.clone()); }
                    }
                }
            }
            (s, d) => *s = d.clone(),
        }
    }

    pub async fn reconstruct_container_state(
        &self,
        container_id: &str,
        at_timestamp: DateTime<Utc>,
    ) -> Result<ReconstructedState, QueryError> {
        let mut pages = self.storage.list_finalized_pages_summary(0).await?;
        if let Some(active) = self.time_manager.get_current_active_page_id(0).await {
            if let Ok(Some(p)) = self.storage.load_page(0, active).await {
                pages.push(crate::core::page::JournalPageSummary {
                    page_id: p.page_id,
                    level: p.level,
                    creation_timestamp: p.creation_timestamp,
                    end_time: p.end_time,
                    page_hash: p.page_hash,
                });
            }
        }
        pages.sort_by_key(|p| p.creation_timestamp);
        let mut state = Value::Null;
        let mut found = false;
        for summary in pages {
            if summary.creation_timestamp > at_timestamp { break; }
            if let Ok(Some(page)) = self.storage.load_page(summary.level, summary.page_id).await {
                if let crate::core::page::PageContent::Leaves(leaves) = page.content {
                    for leaf in leaves {
                        if leaf.timestamp > at_timestamp { break; }
                        if leaf.container_id == container_id {
                            Self::apply_delta(&mut state, &leaf.delta_payload);
                            found = true;
                        }
                    }
                }
            }
        }
        if !found {
            return Err(QueryError::ContainerNotFound(container_id.to_string()));
        }
        Ok(ReconstructedState { container_id: container_id.to_string(), at_point: QueryPoint::Timestamp(at_timestamp), state_data: state })
    }

    pub async fn get_delta_report(
        &self,
        container_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<DeltaReport, QueryError> {
        if from > to { return Err(QueryError::InvalidParameters("from is after to".into())); }
        let mut pages = self.storage.list_finalized_pages_summary(0).await?;
        if let Some(active) = self.time_manager.get_current_active_page_id(0).await {
            if let Ok(Some(p)) = self.storage.load_page(0, active).await {
                pages.push(crate::core::page::JournalPageSummary {
                    page_id: p.page_id,
                    level: p.level,
                    creation_timestamp: p.creation_timestamp,
                    end_time: p.end_time,
                    page_hash: p.page_hash,
                });
            }
        }
        pages.sort_by_key(|p| p.creation_timestamp);
        let mut deltas = Vec::new();
        for summary in pages {
            if summary.end_time < from || summary.creation_timestamp > to { continue; }
            if let Ok(Some(page)) = self.storage.load_page(summary.level, summary.page_id).await {
                if let crate::core::page::PageContent::Leaves(leaves) = page.content {
                    for leaf in leaves {
                        if leaf.timestamp < from { continue; }
                        if leaf.timestamp > to { break; }
                        if leaf.container_id == container_id { deltas.push(leaf); }
                    }
                }
            }
        }
        if deltas.is_empty() {
            return Err(QueryError::ContainerNotFound(container_id.to_string()));
        }
        deltas.sort_by_key(|l| l.timestamp);
        Ok(DeltaReport { container_id: container_id.to_string(), from_point: QueryPoint::Timestamp(from), to_point: QueryPoint::Timestamp(to), deltas })
    }

    pub async fn get_page_chain_integrity(
        &self,
        level: u8,
        from: Option<u64>,
        to: Option<u64>,
    ) -> Result<Vec<PageIntegrityReport>, QueryError> {
        if let (Some(f), Some(t)) = (from, to) { if f > t { return Err(QueryError::InvalidParameters("from > to".into())); } }
        let mut pages = self.storage.list_finalized_pages_summary(level).await?;
        if let Some(active) = self.time_manager.get_current_active_page_id(level).await {
            if let Ok(Some(p)) = self.storage.load_page(level, active).await {
                pages.push(crate::core::page::JournalPageSummary {
                    page_id: p.page_id,
                    level: p.level,
                    creation_timestamp: p.creation_timestamp,
                    end_time: p.end_time,
                    page_hash: p.page_hash,
                });
            }
        }
        pages.sort_by_key(|p| p.page_id);
        let mut reports = Vec::new();
        let mut prev_hash: Option<[u8; 32]> = None;
        for summary in pages {
            if let Some(f) = from { if summary.page_id < f { continue; } }
            if let Some(t) = to { if summary.page_id > t { continue; } }
            match self.storage.load_page(level, summary.page_id).await {
                Ok(Some(mut page)) => {
                    let mut issues = Vec::new();
                    let orig_hash = page.page_hash;
                    let orig_root = page.merkle_root;
                    page.recalculate_merkle_root_and_page_hash();
                    if page.merkle_root != orig_root { issues.push("merkle_root mismatch".into()); }
                    if page.page_hash != orig_hash { issues.push("page_hash mismatch".into()); }
                    if let Some(prev) = prev_hash { if page.prev_page_hash != Some(prev) { issues.push("prev_page_hash mismatch".into()); } }
                    prev_hash = Some(orig_hash);
                    reports.push(PageIntegrityReport { page_id: summary.page_id, level, is_valid: issues.is_empty(), issues });
                }
                Ok(None) => {
                    reports.push(PageIntegrityReport { page_id: summary.page_id, level, is_valid: false, issues: vec!["page missing".into()] });
                    prev_hash = None;
                }
                Err(e) => return Err(QueryError::CoreError(e)),
            }
        }
        Ok(reports)
    }

    // Helper async function (example, would need more thought)
    // async fn find_page_for_leaf(&self, leaf_hash: &Hash) -> Result<(u8, u64), QueryError> {
    //     // This is a complex problem without a dedicated index (leaf_hash -> page_id)
    //     // Option 1: Query TimeManager for all active pages across levels, load and check.
    //     // Option 2: If leaves have a somewhat predictable sequence or timestamp, narrow down search.
    //     // Option 3: Full scan (very bad for performance).
    //     // This function would likely involve iterating through pages known to TimeManager
    //     // or directly querying storage if it supports some form of scan.
    //     Err(QueryError::NotImplemented("find_page_for_leaf".to_string()))
    // }
}
