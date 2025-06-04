// src/query/engine.rs
use crate::query::types::{
    QueryError, LeafInclusionProof,
};
use crate::config::Config;

use crate::storage::StorageBackend;
use crate::core::time_manager::TimeHierarchyManager;

use std::sync::Arc;

#[derive(Clone)]
pub struct QueryEngine<S: StorageBackend + Send + Sync + 'static> {
    storage: Arc<S>,
    time_manager: Arc<TimeHierarchyManager>,
    _config: Arc<Config>,
}

impl<S: StorageBackend + Send + Sync + 'static> QueryEngine<S> {
    pub fn new(
        storage: Arc<S>,
        time_manager: Arc<TimeHierarchyManager>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            storage,
            time_manager,
            _config: config,
        }
    }

    pub async fn get_leaf_inclusion_proof(
        &self,
        leaf_hash: &[u8; 32],
        // Optional: page_id_hint: Option<(u8, u64)>,
    ) -> Result<LeafInclusionProof, QueryError> {
        // TODO: Implement more efficient page searching, possibly with a timestamp hint
        // or by querying TimeHierarchyManager for pages within a relevant time range if known.
        // For now, we iterate through known level 0 pages.

        let mut level0_page_ids_to_check: Vec<u64> = Vec::new();

        // Collect active level 0 page ID
        if let Some(active_page_id) = self.time_manager.get_current_active_page_id(0) {
            level0_page_ids_to_check.push(active_page_id);
        }

        // Collect archived level 0 page IDs
        // Assuming TimeHierarchyManager has a method like get_archived_page_ids_for_level
        // This method might not exist yet and needs to be confirmed or implemented in TimeHierarchyManager.
        // For now, let's assume it exists for the logic flow.
        // if let Some(archived_ids) = self.time_manager.get_archived_page_ids_for_level(0) {
        //     level0_page_ids_to_check.extend(archived_ids);
        // }
        // TODO: Add a proper way to get all relevant page IDs from TimeHierarchyManager.
        // For now, we'll just use the active page if available.
        
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
                        crate::core::page::PageContent::NetPatches(net_patches_content) => {
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
