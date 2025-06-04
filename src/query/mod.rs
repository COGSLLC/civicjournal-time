//! # Query Module
//!
//! Provides functionality for querying and verifying journal data, including:
//! - Leaf inclusion proofs
//! - Page integrity verification
//! - State reconstruction at specific points in time
//!
//! This module is essential for verifying the integrity of the journal and
//! retrieving historical states.

/// Query engine implementation for executing complex queries against the journal.
pub mod engine;
pub mod types;

pub use engine::QueryEngine;
pub use types::{
    QueryError, QueryPoint, LeafInclusionProof, PageIntegrityReport, ReconstructedState, DeltaReport,
};
