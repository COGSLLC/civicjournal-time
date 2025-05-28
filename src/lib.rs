//! A journal system using merkle-chains and pages for efficient historical state management.
//! 
//! This crate provides a way to store and query state changes over time with cryptographic
//! integrity guarantees using a merkle-chain of journal pages.

#![warn(missing_docs)]
#![allow(clippy::new_without_default)]

pub mod merkle_tree;
pub mod rate_limiter;
pub mod schema;
pub mod recovery;
pub mod journal_api;
pub mod health_monitor;
pub mod fork_detection;
pub mod time_hierarchy;
#[cfg(test)]
pub mod integration_tests;

use chrono::{DateTime, Utc};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::sync::Mutex;
use std::{
    num::NonZeroUsize,
    sync::Arc,
};

use thiserror::Error as ThisError;

// Re-export MerkleTree for public use with a different name to avoid conflict
pub use merkle_tree::MerkleTree as MerkleTreeImpl;

/// A simple wrapper for serializing Arc<T>
#[derive(Debug, Clone)]
pub struct ArcWrapper<T>(pub Arc<T>);

impl<T: Serialize> Serialize for ArcWrapper<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for ArcWrapper<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        T::deserialize(deserializer).map(|x| ArcWrapper(Arc::new(x)))
    }
}

/// A single change record ("leaf") in the journal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalLeaf {
    /// Unique, auto-incrementing identifier
    pub leaf_id: u64,
    /// Timestamp of the change (ISO 8601)
    pub timestamp: DateTime<Utc>,
    /// Hash of the previous JournalLeaf record
    pub prev_hash: [u8; 32],
    /// Identifier of the container being modified (e.g., "proposal:abc123")
    pub container_id: String,
    /// The actual change data (JSON/YAML patch or full record data)
    pub delta_payload: Vec<u8>,
    /// Hash of this leaf: SHA-256(leaf_id || timestamp || prev_hash || container_id || delta_payload)
    pub leaf_hash: [u8; 32],
}

/// A "page" that groups N consecutive leaves for Merkle-tree batching
#[derive(Debug, Clone)]
pub struct JournalPage {
    /// Unique identifier for this page
    pub page_id: u64,
    /// Array of leaf hashes in this page
    pub leaf_hashes: Vec<[u8; 32]>,
    /// Root hash of the Merkle tree over leaf_hashes
    pub merkle_root: [u8; 32],
    /// Hash of this page: SHA-256(page_id || merkle_root || previous_page_hash)
    pub page_hash: [u8; 32],
    /// Hash of the previous page in the chain
    pub previous_page_hash: [u8; 32],
    /// External timestamp authority proof for merkle_root (RFC 3161/OpenTimestamps)
    pub ts_proof: Vec<u8>,
}

/// Serializable version of JournalPage for persistence
#[derive(Serialize, Deserialize)]
struct SerializableJournalPage {
    page_id: u64,
    leaf_hashes: Vec<[u8; 32]>,
    merkle_root: [u8; 32],
    page_hash: [u8; 32],
    previous_page_hash: [u8; 32],
    ts_proof: Vec<u8>,
}

impl JournalPage {
    /// Converts the JournalPage to a serializable format
    pub fn to_serializable(&self) -> SerializableJournalPage {
        SerializableJournalPage {
            page_id: self.page_id,
            leaf_hashes: self.leaf_hashes.clone(),
            merkle_root: self.merkle_root,
            page_hash: self.page_hash,
            previous_page_hash: self.previous_page_hash,
            ts_proof: self.ts_proof.clone(),
        }
    }

    /// Creates a JournalPage from a serializable format
    pub fn from_serializable(serializable: SerializableJournalPage) -> Self {
        Self {
            page_id: serializable.page_id,
            leaf_hashes: serializable.leaf_hashes,
            merkle_root: serializable.merkle_root,
            page_hash: serializable.page_hash,
            previous_page_hash: serializable.previous_page_hash,
            ts_proof: serializable.ts_proof,
        }
    }
    
    /// Gets the page ID
    pub fn page_id(&self) -> u64 {
        self.page_id
    }
    
    /// Gets the Merkle root hash
    pub fn merkle_root(&self) -> &[u8; 32] {
        &self.merkle_root
    }
    
    /// Gets the page hash
    pub fn page_hash(&self) -> &[u8; 32] {
        &self.page_hash
    }
    
    /// Gets the previous page hash
    pub fn previous_page_hash(&self) -> &[u8; 32] {
        &self.previous_page_hash
    }
    
    /// Gets the timestamp proof
    pub fn timestamp_proof(&self) -> &[u8] {
        &self.ts_proof
    }
}

/// Manages the journal of pages and leaves
/// Errors that can occur during journal operations
#[derive(Debug, ThisError)]
pub enum JournalError {
    /// The specified page was not found
    #[error("Page not found: {0}")]
    PageNotFound(u64),
    /// The specified leaf was not found
    #[error("Leaf not found: {0}")]
    LeafNotFound(u64),
    /// Invalid hash in the journal
    #[error("Invalid hash at position {0}")]
    InvalidHash(String),
    
    /// Other error
    #[error("Error: {0}")]
    Other(String),
    /// Merkle proof verification failed
    #[error("Merkle proof verification failed: {0}")]
    MerkleProofError(String),
    /// Timestamp verification failed
    #[error("Timestamp verification failed: {0}")]
    TimestampError(String),
    /// An I/O error occurred
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A serialization error occurred
    #[error("Serialization error: {0}")]
    Serialization(String),
    /// A deserialization error occurred
    #[error("Deserialization error: {0}")]
    Deserialization(String),
    /// Verification failed
    #[error("Verification error: {0}")]
    VerificationError(String),
    /// Rate limiting error
    #[error("Rate limited: {0}")]
    RateLimited(String),
    /// Temporarily banned
    #[error("Temporarily banned due to excessive requests")]
    TemporarilyBanned,
}

/// Statistics about the journal
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct JournalStats {
    /// Number of leaves in the journal
    pub leaf_count: u64,
    /// Number of pages in the journal
    pub page_count: u64,
    /// Earliest timestamp in the journal
    pub earliest_timestamp: Option<DateTime<Utc>>,
    /// Latest timestamp in the journal
    pub latest_timestamp: Option<DateTime<Utc>>,
    /// Size of the journal in bytes
    pub total_size_bytes: u64,
}

/// Configuration for the journal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalConfig {
    /// Maximum number of leaves per page
    pub max_leaves_per_page: usize,
    /// Maximum number of pages to keep in memory
    pub max_in_memory_pages: usize,
    /// Maximum number of leaves to keep in memory
    pub max_in_memory_leaves: usize,
    /// Path prefix for storing journal data
    pub storage_path: String,
    /// Whether to enable external timestamping
    pub enable_timestamping: bool,
    /// URL of the timestamp authority (if enabled)
    pub timestamp_authority_url: Option<String>,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            max_leaves_per_page: 1000,
            max_in_memory_pages: 100,
            max_in_memory_leaves: 10_000,
            storage_path: "./journal_data".to_string(),
            enable_timestamping: true,
            timestamp_authority_url: Some("https://freetsa.org/tsr".to_string()),
        }
    }
}

/// Merkle tree implementation for page verification
#[derive(Debug, Clone)]
struct MerkleTree {
    /// The root hash of the Merkle tree
    root_hash: [u8; 32],
    /// The leaves of the tree
    leaves: Vec<[u8; 32]>,
    /// Internal nodes of the tree
    nodes: Vec<[u8; 32]>,
}

/// Main journal structure that manages pages and leaves
#[derive(Debug)]
pub struct Journal {
    /// Configuration for the journal
    config: JournalConfig,
    /// The most recent page in the journal
    head_page: Option<Arc<JournalPage>>,
    /// In-memory cache of recent pages
    page_cache: LruCache<u64, Arc<JournalPage>>,
    /// In-memory cache of recent leaves
    leaf_cache: LruCache<u64, Arc<JournalLeaf>>,
    /// Statistics about the journal
    stats: JournalStats,
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}

impl Journal {
    /// Creates a new Journal with default configuration
    pub fn new() -> Self {
        Self::with_config(JournalConfig::default())
    }

    /// Creates a Journal with a custom configuration
    pub fn with_config(config: JournalConfig) -> Self {
        Journal {
            config: config.clone(),
            head_page: None,
            page_cache: LruCache::new(
                NonZeroUsize::new(config.max_in_memory_pages).unwrap_or_else(|| NonZeroUsize::new(1).unwrap())
            ),
            leaf_cache: LruCache::new(
                NonZeroUsize::new(config.max_in_memory_leaves).unwrap_or_else(|| NonZeroUsize::new(100).unwrap())
            ),
            stats: JournalStats::default(),
        }
    }
    
    /// Gets the current head page
    pub fn head_page(&self) -> Option<&Arc<JournalPage>> {
        self.head_page.as_ref()
    }
    
    /// Gets the current statistics
    pub fn stats(&self) -> &JournalStats {
        &self.stats
    }
    
    /// Gets a page by its ID
    pub fn get_page(&self, page_id: u64) -> Option<Arc<JournalPage>> {
        // Check if page is in memory
        if let Some(page) = &self.head_page {
            if page.page_id == page_id {
                return Some(page.clone());
            }
        }
        
        // Try to load from disk
        self.load_page_from_disk(page_id).ok()
    }
    
    /// Gets a leaf by its ID
    pub fn get_leaf(&self, leaf_id: u64) -> Option<Arc<JournalLeaf>> {
        // Try to load from disk first since we're not keeping a mutable cache in this method
        self.load_leaf_from_disk(leaf_id).ok()
    }
    
    /// Adds a new leaf to the journal
    pub fn add_leaf(&mut self, container_id: String, delta_payload: Vec<u8>) -> Result<u64, JournalError> {
        // Generate a new leaf ID
        let leaf_id = self.stats.leaf_count + 1;
        
        // Get the previous leaf hash
        let prev_hash = self.head_page.as_ref()
            .and_then(|page| page.leaf_hashes.last().copied())
            .unwrap_or([0u8; 32]);
        
        // Create the new leaf
        let timestamp = Utc::now();
        let leaf = JournalLeaf {
            leaf_id,
            timestamp,
            prev_hash,
            container_id: container_id.clone(), // Clone to avoid move
            delta_payload: delta_payload.clone(),
            leaf_hash: [0u8; 32], // Will be set after calculating the hash
        };
        
        // Calculate the leaf hash
        let leaf_hash = self.calculate_leaf_hash(&leaf);
        let leaf = JournalLeaf { leaf_hash, ..leaf };
        
        // Add to the current page or create a new one
        if let Some(page_arc) = &mut self.head_page {
            // Get a mutable reference to the page
            let page = Arc::get_mut(page_arc).unwrap();
            
            // If the page is full, finalize it and create a new one
            if page.leaf_hashes.len() >= self.config.max_leaves_per_page {
                self.finalize_page()?;
                return self.add_leaf(container_id, delta_payload);
            }
            
            // Add the leaf to the current page
            page.leaf_hashes.push(leaf_hash);
            self.leaf_cache.put(leaf_id, Arc::new(leaf));
            self.stats.leaf_count += 1;
            
            // Update the Merkle root
            self.update_merkle_root()?;
        } else {
            // This is the first leaf, create a new page
            let page = self.create_new_page(leaf_hash, [0u8; 32])?;
            self.leaf_cache.put(leaf_id, Arc::new(leaf));
            self.stats.leaf_count += 1;
            self.head_page = Some(page);
        }
        
        Ok(leaf_id)
    }
    
    /// Finalizes the current page and prepares a new one
    fn finalize_page(&mut self) -> Result<(), JournalError> {
        if let Some(page) = self.head_page.clone() {
            // Get the current page hash
            let page_hash = page.page_hash;
            
            // Create a new page with the current page's hash as the previous_page_hash
            let new_page = self.create_new_page([0u8; 32], page_hash)?;
            
            // Persist the finalized page to disk
            self.persist_page(&page)?;
            
            // Update the head page and stats
            self.head_page = Some(new_page);
            self.stats.page_count += 1;
        }
        
        Ok(())
    }
    
    /// Creates a new page with the given leaf hash and previous page hash
    fn create_new_page(&self, first_leaf_hash: [u8; 32], previous_page_hash: [u8; 32]) -> Result<Arc<JournalPage>, JournalError> {
        let page_id = self.stats.page_count + 1;
        let leaf_hashes = vec![first_leaf_hash];
        
        // Create a Merkle tree for the page using our crate's merkle_tree module
        let merkle_tree = crate::merkle_tree::MerkleTree::new(leaf_hashes.clone());
        let merkle_root = *merkle_tree.root_hash();
        
        // Calculate the page hash
        let mut hasher = Sha256::new();
        hasher.update(&page_id.to_be_bytes());
        hasher.update(&merkle_root);
        hasher.update(&previous_page_hash);
        let page_hash: [u8; 32] = hasher.finalize().into();
        
        // Get timestamp proof (placeholder - would integrate with TSA in a real implementation)
        let ts_proof = if self.config.enable_timestamping {
            self.get_timestamp_proof(&merkle_root)?
        } else {
            Vec::new()
        };
        
        Ok(Arc::new(JournalPage {
            page_id,
            leaf_hashes,
            merkle_root,
            page_hash,
            previous_page_hash,
            ts_proof,
        }))
    }
    
    /// Calculates the hash of a leaf
    fn calculate_leaf_hash(&self, leaf: &JournalLeaf) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&leaf.leaf_id.to_be_bytes());
        hasher.update(leaf.timestamp.timestamp().to_be_bytes());
        hasher.update(&leaf.prev_hash);
        hasher.update(leaf.container_id.as_bytes());
        hasher.update(&leaf.delta_payload);
        hasher.finalize().into()
    }
    
    /// Updates the Merkle root for the current page
    fn update_merkle_root(&mut self) -> Result<(), JournalError> {
        if let Some(page_arc) = &mut self.head_page {
            // Get a mutable reference to the page
            let page = Arc::get_mut(page_arc).unwrap();
            
            // Use our merkle tree implementation
            let merkle_tree = crate::merkle_tree::MerkleTree::new(page.leaf_hashes.clone());
            page.merkle_root = *merkle_tree.root_hash();
            
            // Update the page hash
            let mut hasher = Sha256::new();
            hasher.update(&page.page_id.to_be_bytes());
            hasher.update(&page.merkle_root);
            hasher.update(&page.previous_page_hash);
            page.page_hash = hasher.finalize().into();
            
            Ok(())
        } else {
            Err(JournalError::PageNotFound(0))
        }
    }
    
    /// Gets a timestamp proof from the timestamp authority
    fn get_timestamp_proof(&self, _data: &[u8; 32]) -> Result<Vec<u8>, JournalError> {
        // In a real implementation, this would contact a TSA
        // For now, we'll return a dummy proof
        let mut proof = vec![0u8; 64]; // Example proof size
        proof[..32].copy_from_slice(_data);
        proof[32..].copy_from_slice(&[b't', b's', b'a', b'_', b'p', b'r', b'o', b'o', b'f']);
        Ok(proof)
    }
    
    /// Loads a page from disk
    fn load_page_from_disk(&self, page_id: u64) -> Result<Arc<JournalPage>, JournalError> {
        use std::fs::File;
        use std::io::Read;
        
        // Create the file path
        let file_path = format!("data/pages/page_{}.bin", page_id);
        
        // Open the file
        let mut file = File::open(&file_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                JournalError::PageNotFound(page_id)
            } else {
                JournalError::Io(e)
            }
        })?;
        
        // Read the file contents
        let mut data = Vec::new();
        file.read_to_end(&mut data).map_err(JournalError::Io)?;
        
        // Deserialize the page
        let mut serialized: SerializableJournalPage = bincode::deserialize(&data)
            .map_err(|e| JournalError::Deserialization(e.to_string()))?;
            
        // Set the page ID from the parameter to ensure consistency
        serialized.page_id = page_id;
            
        // Convert to the actual page type
        let page = JournalPage::from_serializable(serialized);
        
        Ok(Arc::new(page))
    }
    
    /// Loads a leaf from disk
    fn load_leaf_from_disk(&self, _leaf_id: u64) -> Result<Arc<JournalLeaf>, JournalError> {
        // TODO: Implement actual disk loading
        Err(JournalError::LeafNotFound(_leaf_id))
    }
    
    /// Persists a page to disk
    fn persist_page(&self, page: &JournalPage) -> Result<(), JournalError> {
        // Create the data directory if it doesn't exist
        std::fs::create_dir_all("data/pages").map_err(|e| JournalError::Io(e))?;
        
        // Create the file path
        let file_path = format!("data/pages/page_{}.bin", page.page_id);
        
        // Serialize the page data
        let serialized = bincode::serialize(&page.to_serializable())
            .map_err(|e| JournalError::Serialization(e.to_string()))?;
        
        // Write to disk with retry logic
        self.write_file_with_retry(&file_path, &serialized, 3)
    }
    
    /// Writes a file with retry logic
    fn write_file_with_retry(&self, path: &str, contents: &[u8], mut retries: u32) -> Result<(), JournalError> {
        loop {
            match std::fs::write(path, contents) {
                Ok(_) => return Ok(()),
                Err(e) if retries == 0 => return Err(JournalError::Io(e)),
                Err(_) => {
                    retries -= 1;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    }
    
    /// Verifies the integrity of the journal
    pub fn verify_integrity(&self) -> Result<(), JournalError> {
        // Verify the Merkle tree for each page
        if let Some(mut page) = self.head_page.clone() {
            let mut prev_page_hash = [0u8; 32];
            
            // Walk the chain of pages backwards
            loop {
                // Verify the Merkle tree
                let merkle_tree = crate::merkle_tree::MerkleTree::new(page.leaf_hashes.clone());
                if *merkle_tree.root_hash() != page.merkle_root {
                    return Err(JournalError::VerificationError(
                        format!("Merkle root mismatch for page {}", page.page_id)
                    ));
                }
                
                // Verify the page hash
                let mut hasher = Sha256::new();
                hasher.update(&page.page_id.to_be_bytes());
                hasher.update(&page.merkle_root);
                hasher.update(&page.previous_page_hash);
                let computed_hash: [u8; 32] = hasher.finalize().into();
                
                if computed_hash != page.page_hash {
                    return Err(JournalError::VerificationError(
                        format!("Page hash mismatch for page {}", page.page_id)
                    ));
                }
                
                // Verify the chain of hashes
                if page.previous_page_hash != prev_page_hash {
                    return Err(JournalError::VerificationError(
                        format!("Page hash chain broken at page {}", page.page_id)
                    ));
                }
                
                // Move to the previous page
                if page.previous_page_hash == [0u8; 32] {
                    break; // Reached the first page
                }
                
                // Load the previous page
                let prev_page = self.load_page_from_disk(page.page_id - 1)
                    .map_err(|_| JournalError::VerificationError(
                        format!("Failed to load previous page {}", page.page_id - 1)
                    ))?;
                
                prev_page_hash = page.page_hash;
                page = prev_page;
            }
        }
        
        Ok(())
    }
    
    /// Calculates statistics for the journal
    pub fn calculate_stats(&self) -> JournalStats {
        let mut stats = JournalStats {
            page_count: 0,
            leaf_count: 0,
            total_size_bytes: 0,
            earliest_timestamp: None,
            latest_timestamp: None,
        };
        
        // Count pages and leaves
        if let Some(page) = self.head_page.as_ref() {
            stats.page_count = page.page_id;
            stats.leaf_count = self.stats.leaf_count;
            
            // In a real implementation, we would calculate the total size on disk
            // For now, we'll just return the in-memory size
            stats.total_size_bytes = (std::mem::size_of_val(page) * (page.page_id as usize)) as u64;
        }
        
        stats
    }
    
    /// Saves the journal to disk
    pub fn save(&self) -> Result<(), JournalError> {
        // Create the data directory if it doesn't exist
        std::fs::create_dir_all("data").map_err(|e| JournalError::Io(e))?;
        
        // Save the head page
        if let Some(page) = &self.head_page {
            self.persist_page(page)?;
        }
        
        // Save the journal metadata
        let metadata = serde_json::to_vec(&self.stats)
            .map_err(|e| JournalError::Serialization(e.to_string()))?;
        
        std::fs::write("data/journal_meta.json", metadata)
            .map_err(|e| JournalError::Io(e))?;
        
        Ok(())
    }
    
    /// Loads the journal from disk
    pub fn load() -> Result<Self, JournalError> {
        // Create a new journal with default config
        let mut journal = Journal::new();
        
        // Load the metadata
        let metadata = std::fs::read("data/journal_meta.json")
            .map_err(|e| JournalError::Io(e))?;
            
        journal.stats = serde_json::from_slice(&metadata)
            .map_err(|e| JournalError::Serialization(e.to_string()))?;
        
        // Load the head page if it exists
        if journal.stats.page_count > 0 {
            let head_page = journal.load_page_from_disk(journal.stats.page_count)?;
            journal.head_page = Some(head_page);
        }
        
        Ok(journal)
    }
}
