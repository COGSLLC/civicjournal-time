//! Types and data structures for querying the journal and verifying data integrity.
//!
//! This module defines the core types used throughout the query system,
//! including error types, query parameters, and result types for various
//! journal operations.

use crate::core::leaf::JournalLeaf;
use crate::core::merkle::MerkleProof;
use chrono::{DateTime, Utc};
use serde_json;

/// Errors that can occur during query operations.
///
/// This enum represents all possible error conditions that can occur
/// when querying the journal or verifying data integrity.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    /// An error originating from the core journal functionality
    #[error("Core error: {0}")]
    CoreError(#[from] crate::error::CJError),
    
    /// The requested page was not found in the journal
    #[error("Page not found: Level {level}, ID {page_id}")]
    PageNotFound { 
        /// The hierarchy level where the page was expected
        level: u8, 
        /// The ID of the missing page
        page_id: u64 
    },
    
    /// The requested leaf was not found in the journal
    #[error("Leaf not found with hash {0:?}")]
    LeafNotFound(#[doc = "The hash of the missing leaf"] [u8; 32]),
    
    /// The requested container was not found in the journal
    #[error("Container not found: {0}")]
    ContainerNotFound(#[doc = "The ID of the missing container"] String),
    
    /// An integrity check failed during query execution
    #[error("Integrity check failed: {0}")]
    IntegrityError(#[doc = "Description of the integrity failure"] String),
    
    /// The query parameters were invalid or malformed
    #[error("Invalid query parameters: {0}")]
    InvalidParameters(#[doc = "Explanation of the parameter error"] String),
    
    /// The requested feature is not yet implemented
    #[error("Feature not yet implemented: {0}")]
    NotImplemented(#[doc = "Description of the unimplemented feature"] String),
    
    /// The data for a leaf was not found in storage
    #[error("Leaf data not found in storage for hash: {0:?}")]
    LeafDataNotFound(#[doc = "The hash of the leaf with missing data"] [u8; 32]),
}

/// Represents a specific point in the journal that can be used as a query parameter.
///
/// This enum allows querying the journal using different types of references,
/// providing flexibility in how journal data is accessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryPoint {
    /// Query by a specific point in time.
    ///
    /// The journal will be queried as it existed at the specified timestamp.
    Timestamp(
        /// The UTC timestamp to query
        DateTime<Utc>
    ),
    
    /// Query by a specific page in the journal hierarchy.
    ///
    /// This allows direct access to a specific page at a given level
    /// in the time-based hierarchy.
    PageId { 
        /// The level in the time hierarchy (0 = leaves, 1+ = rollup levels).
        level: u8, 
        /// The ID of the page within its level.
        page_id: u64 
    },
    
    /// Query by the hash of a specific leaf.
    ///
    /// Looks up a leaf by its cryptographic hash, which provides
    /// a content-addressable way to reference journal entries.
    LeafHash(
        /// The SHA-256 hash of the leaf
        [u8; 32]
    ),
}

/// Cryptographic proof that a specific leaf exists in a journal page's Merkle tree.
///
/// This proof allows verification that a leaf was included in a specific page
/// without requiring access to the entire page contents.
#[derive(Debug, Clone)]
pub struct LeafInclusionProof {
    /// The leaf data that this proof verifies.
    pub leaf: JournalLeaf,
    
    /// The ID of the page containing this leaf
    pub page_id: u64,
    
    /// The hierarchy level of the page containing this leaf
    pub level: u8,
    
    /// The Merkle proof for this leaf's inclusion in the page.
    /// Contains the sibling hashes needed to verify the path from the leaf to the root.
    pub proof: MerkleProof,
    
    /// The Merkle root of the page that this proof is relative to.
    /// This should match the `page_hash` field of the containing `JournalPage`.
    pub page_merkle_root: [u8; 32],
}

/// Report on the integrity of a journal page.
///
/// This structure provides detailed information about the results of
/// integrity verification for a single journal page.
#[derive(Debug, Clone)]
pub struct PageIntegrityReport {
    /// The ID of the page that was checked
    pub page_id: u64,
    
    /// The hierarchy level of the checked page
    pub level: u8,
    
    /// Whether the page passed all integrity checks.
    /// A value of `false` indicates one or more issues were found.
    pub is_valid: bool,
    
    /// Any issues found during integrity checking.
    /// Each string describes a specific integrity problem that was detected.
    pub issues: Vec<String>,
}

/// The reconstructed state of a container at a specific point in time.
///
/// This structure represents the complete state of a container as it existed
/// at a specific point in the journal's history.
#[derive(Debug, Clone)]
pub struct ReconstructedState {
    /// The ID of the container this state belongs to
    pub container_id: String,
    
    /// The point in time or journal position this state represents.
    pub at_point: QueryPoint,
    
    /// The reconstructed state data as a JSON value.
    /// The structure of this data is application-specific and depends on
    /// the container type and the deltas that were applied.
    pub state_data: serde_json::Value,
}

/// A report of changes to a container between two points in time.
///
/// This structure provides a complete audit trail of all changes made to a
/// container within a specified range of the journal.
#[derive(Debug, Clone)]
pub struct DeltaReport {
    /// The ID of the container these deltas apply to
    pub container_id: String,
    
    /// The starting point for this delta report (inclusive).
    /// All changes after this point are included in the report.
    pub from_point: QueryPoint,
    
    /// The ending point for this delta report (inclusive).
    /// Changes up to and including this point are included.
    pub to_point: QueryPoint,
    
    /// The sequence of leaf changes between the two points, in chronological order.
    /// Each leaf represents a single atomic change to the container's state.
    pub deltas: Vec<JournalLeaf>,
}
