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

// Re-export key structures or functions if needed
// pub use leaf::JournalLeaf;
// pub use page::JournalPage;
// pub use merkle::calculate_merkle_root;
// pub use hash::sha256_hash;

// Shared items for testing, accessible within the crate.
// Placed here to ensure they are compiled and accessible for all test modules within `core`.

#[cfg(test)]
use tokio::sync::Mutex;
#[cfg(test)]
use lazy_static::lazy_static;
#[cfg(test)]
use std::sync::atomic::Ordering;

#[cfg(test)]
lazy_static! {
    pub(crate) static ref SHARED_TEST_ID_MUTEX: Mutex<()> = Mutex::new(());
}

#[cfg(test)] // Still want these items only compiled for test builds
pub(crate) fn reset_global_ids() {
    // These paths assume NEXT_PAGE_ID is pub(crate) in crate::core::page
    // and NEXT_LEAF_ID is pub(crate) in crate::core::leaf
    crate::core::page::NEXT_PAGE_ID.store(0, Ordering::SeqCst);
    crate::core::leaf::NEXT_LEAF_ID.store(0, Ordering::SeqCst);
}
