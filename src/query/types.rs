// src/query/types.rs
use crate::core::leaf::JournalLeaf;
use crate::core::merkle::MerkleProof;
use chrono::{DateTime, Utc};
use serde_json; // Added for ReconstructedState

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("Core error: {0}")]
    CoreError(#[from] crate::error::CJError),
    #[error("Page not found: Level {level}, ID {page_id}")]
    PageNotFound { level: u8, page_id: u64 },
    #[error("Leaf not found with hash {0:?}")]
    LeafNotFound([u8; 32]),
    #[error("Container not found: {0}")]
    ContainerNotFound(String),
    #[error("Integrity check failed: {0}")]
    IntegrityError(String),
    #[error("Invalid query parameters: {0}")]
    InvalidParameters(String),
    #[error("Feature not yet implemented: {0}")]
    NotImplemented(String),
    #[error("Leaf data not found in storage for hash: {0:?}")]
    LeafDataNotFound([u8; 32]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryPoint {
    Timestamp(DateTime<Utc>),
    PageId { level: u8, page_id: u64 },
    LeafHash([u8; 32]),
}

#[derive(Debug, Clone)]
pub struct LeafInclusionProof {
    pub leaf: JournalLeaf,
    pub page_id: u64,
    pub level: u8,
    pub proof: MerkleProof,
    pub page_merkle_root: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct PageIntegrityReport {
    pub page_id: u64,
    pub level: u8,
    pub is_valid: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ReconstructedState {
    pub container_id: String,
    pub at_point: QueryPoint,
    pub state_data: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct DeltaReport {
    pub container_id: String,
    pub from_point: QueryPoint,
    pub to_point: QueryPoint,
    pub deltas: Vec<JournalLeaf>,
}
