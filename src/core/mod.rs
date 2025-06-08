// src/core/mod.rs

/// Defines the `JournalLeaf` structure, representing individual entries in the journal.
pub mod leaf;
/// Defines the `JournalPage` structure, which aggregates journal leaves and/or thrall pages.
pub mod page;
/// Implements Merkle tree construction, root calculation, and proof generation/verification.
pub mod merkle;
/// Utility functions and type aliases related to cryptographic hashing (SHA256).
pub mod hash;
/// Manages the creation, roll-up, and retrieval of pages across different time levels.
pub mod time_manager;
/// Manages the creation and storage of system snapshots.
pub mod snapshot_manager;
/// Defines data structures related to system snapshots.
pub mod snapshot;

// Re-export key structures or functions if needed
// pub use leaf::JournalLeaf;
// pub use page::JournalPage;
// pub use merkle::calculate_merkle_root;
// pub use hash::sha256_hash;
pub use time_manager::TimeHierarchyManager;
pub use snapshot_manager::{SnapshotManager, SnapshotError};
pub use snapshot::{SnapshotContainerState, SnapshotPagePayload};

// Shared items for testing, accessible within the crate.
// Placed here to ensure they are compiled and accessible for all test modules within `core`.


