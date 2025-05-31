# CivicJournal Time - Development Plan

## Status Update (2025-05-31)

### Completed Tasks

- [x] Implement core time hierarchy management
- [x] Add page finalization logic
- [x] Implement rollup mechanism
- [x] Add comprehensive test coverage
- [x] Fix test reliability issues
- [x] Document architecture and test patterns

## Next Steps

### Short Term (Next 2 Weeks)

1. **Performance Optimization**
   - [ ] Profile rollup operations
   - [ ] Optimize page storage and retrieval
   - [ ] Add metrics collection

2. **API Development**
   - [ ] Implement public API endpoints
   - [ ] Add query interface
   - [ ] Document API usage

3. **Storage Improvements**
   - [ ] Add compression support
   - [ ] Implement retention policies
   - [ ] Add backup/restore functionality

### Medium Term (Next 2 Months)

1. **Distributed Features**
   - [ ] Add replication support
   - [ ] Implement sharding
   - [ ] Add consensus protocol

2. **Advanced Features**
   - [ ] Add support for custom rollup functions
   - [ ] Implement time travel queries
   - [ ] Add support for custom time zones

## Original Planning Document

---

# CivicJournal Time - Development Plan

## 0. Introduction

This document outlines the development plan for CivicJournal Time, aiming to implement the core functionalities as described in `ARCHITECTURE.MD` and `CivicJournalSpec.txt`. The plan is divided into phases, with specific tasks and considerations for each.

## 1. Phase 1: Core Journaling Engine

This phase focuses on building the fundamental data structures and logic for creating and linking journal entries and pages.

### Task 1.1: Implement `JournalLeaf`

**File**: `src/core/leaf.rs`

**Objective**: Define the `JournalLeaf` structure and its hash calculation.

**Details**:
*   Define the struct fields as per `CivicJournalSpec.txt`.
*   Implement a function to calculate `LeafHash`.

```rust
// src/core/leaf.rs (Illustrative)
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use serde::{Serialize, Deserialize};

// Placeholder for a global or page-specific unique ID generator
fn get_next_leaf_id() -> u64 { /* ... */ 0 }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JournalLeaf {
    pub leaf_id: u64,                    // LeafID: unique, incrementing
    pub timestamp: DateTime<Utc>,        // Timestamp: e.g. 2025-06-01T12:00:00Z
    pub prev_hash: Option<[u8; 32]>,     // PrevHash: SHA256 of previous LeafHash (Option for first leaf)
    pub container_id: String,            // ContainerID: "proposal:XYZ" or "user:ABC"
    pub delta_payload: String,           // DeltaPayload: JSON/YAML patch or full record (String for now, consider Vec<u8> or a generic type)
    pub leaf_hash: [u8; 32],             // LeafHash: SHA256(...)
}

impl JournalLeaf {
    pub fn new(
        timestamp: DateTime<Utc>,
        prev_hash: Option<[u8; 32]>,
        container_id: String,
        delta_payload: String,
    ) -> Self {
        let leaf_id = get_next_leaf_id(); // This needs a proper source
        let mut hasher = Sha256::new();
        hasher.update(leaf_id.to_be_bytes());
        hasher.update(timestamp.to_rfc3339().as_bytes());
        if let Some(ph) = prev_hash {
            hasher.update(ph);
        }
        hasher.update(container_id.as_bytes());
        hasher.update(delta_payload.as_bytes());
        let leaf_hash: [u8; 32] = hasher.finalize().into();

        JournalLeaf {
            leaf_id,
            timestamp,
            prev_hash,
            container_id,
            delta_payload,
            leaf_hash,
        }
    }
}
```

### Task 1.2: Implement `JournalPage`

**File**: `src/core/page.rs`

**Objective**: Define the `JournalPage` structure, manage its leaf/thrall hashes, and calculate its hash.

**Details**:
*   Define struct fields as per `CivicJournalSpec.txt`.
*   Store `LeafHashes` (for level 0) or `PageHash` of thralls (for higher levels).
*   Implement `PageHash` calculation.

```rust
// src/core/page.rs (Illustrative)
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use serde::{Serialize, Deserialize};
use crate::core::merkle::MerkleRoot; // Assuming MerkleRoot is [u8; 32]

// Placeholder for a global or level-specific unique ID generator
fn get_next_page_id() -> u64 { /* ... */ 0 }


#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum PageContentHash {
    LeafHash([u8; 32]),
    ThrallPageHash([u8; 32]),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JournalPage {
    pub page_id: u64,                      // PageID: e.g. 42
    pub level: u8,                         // Level: time-hierarchy level (0…6)
    pub start_time: DateTime<Utc>,         // StartTime: page time window
    pub end_time: DateTime<Utc>,           // EndTime: page time window
    pub content_hashes: Vec<PageContentHash>, // Array of N LeafHash or ThrallPageHash values
    pub merkle_root: MerkleRoot,           // MerkleRoot: root of Merkle-tree over content_hashes[]
    pub prev_page_hash: Option<[u8; 32]>,  // PrevPageHash: SHA256 of prior JournalPage.PageHash (Option for first page in a chain/level)
    pub page_hash: [u8; 32],               // PageHash: SHA256(...)
    pub ts_proof: Option<Vec<u8>>,         // TSProof: external timestamp proof for MerkleRoot (Option for now)
    // Thralls[]: Implicitly handled by content_hashes for higher levels.
    // If explicit PageIDs of thralls are needed, add a field:
    // pub thrall_page_ids: Option<Vec<u64>>,
}

impl JournalPage {
    pub fn new(
        level: u8,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        content_hashes: Vec<PageContentHash>,
        merkle_root: MerkleRoot,
        prev_page_hash: Option<[u8; 32]>,
        // ts_proof: Option<Vec<u8>>, // Add if needed at construction
    ) -> Self {
        let page_id = get_next_page_id(); // This needs a proper source
        let mut hasher = Sha256::new();
        hasher.update(page_id.to_be_bytes());
        hasher.update(level.to_be_bytes());
        hasher.update(start_time.to_rfc3339().as_bytes());
        hasher.update(end_time.to_rfc3339().as_bytes());
        hasher.update(merkle_root); // merkle_root is [u8; 32]
        if let Some(pph) = prev_page_hash {
            hasher.update(pph);
        }
        let page_hash: [u8; 32] = hasher.finalize().into();

        JournalPage {
            page_id,
            level,
            start_time,
            end_time,
            content_hashes,
            merkle_root,
            prev_page_hash,
            page_hash,
            ts_proof: None, // Initialize as None
        }
    }
}
```

### Task 1.3: Implement `MerkleTree`

**File**: `src/core/merkle.rs`

**Objective**: Implement Merkle tree construction and root calculation.

**Details**:
*   Function to build a Merkle tree from a list of `PageContentHash`.
*   Function to get the Merkle root.
*   Stub for Merkle proof generation (full implementation can be deferred).

```rust
// src/core/merkle.rs (Illustrative)
use sha2::{Digest, Sha256};
use crate::core::page::PageContentHash; // Or just use Vec<[u8; 32]>

pub type MerkleRoot = [u8; 32];
pub type MerkleProof = Vec<([u8; 32], bool)>; // (hash, is_right_node)

fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

pub fn calculate_merkle_root(hashes: &Vec<PageContentHash>) -> Option<MerkleRoot> {
    if hashes.is_empty() {
        return None;
    }

    let mut current_level: Vec<[u8; 32]> = hashes.iter().map(|h| {
        match h {
            PageContentHash::LeafHash(lh) => *lh,
            PageContentHash::ThrallPageHash(tph) => *tph,
        }
    }).collect();

    if current_level.len() == 1 { // Single item, its hash is the root
        return Some(current_level[0]);
    }

    // Ensure even number of hashes for pairing by duplicating the last if odd
    if current_level.len() % 2 != 0 {
        current_level.push(*current_level.last().unwrap());
    }

    while current_level.len() > 1 {
        let mut next_level = Vec::new();
        for chunk in current_level.chunks_exact(2) {
            next_level.push(hash_pair(&chunk[0], &chunk[1]));
        }
        current_level = next_level;
        if current_level.len() % 2 != 0 && current_level.len() > 1 {
             current_level.push(*current_level.last().unwrap());
        }
    }
    current_level.pop()
}

pub fn generate_merkle_proof(_hashes: &Vec<PageContentHash>, _index: usize) -> Option<MerkleProof> {
    // Stub: Full implementation later
    None
}
```

### Task 1.4: Implement Basic Hash Chaining [COMPLETED]

**Objective**: Ensure `PrevHash` in `JournalLeaf` and `PrevPageHash` in `JournalPage` are correctly managed during creation. This will be integrated into the API and roll-up logic.

**Status**: 
*   `JournalPage.prev_page_hash` is managed by `TimeHierarchyManager` when creating new pages, using its record of the last finalized page hash for that level.
*   `JournalLeaf.prev_hash` is part of `LeafContent` provided to `TimeHierarchyManager`. The responsibility for determining this hash lies with the creator of the `JournalLeaf` instance, external to `TimeHierarchyManager`.

## 2. Phase 2: Storage Backend

This phase focuses on persisting and retrieving `JournalPage` objects.

### Task 2.1: Define/Refine `StorageBackend` Trait

**File**: `src/storage/mod.rs`

**Objective**: Define a clear trait for storage operations, ensuring it supports asynchronous operations and is thread-safe for use by `TimeHierarchyManager`.

**Details**:
*   Ensure trait bounds like `Send + Sync + 'static` are present if the backend is to be shared across async tasks.
*   Review and confirm essential methods: `store_page`, `load_page`, `page_exists`.
*   Consider if methods like `load_last_finalized_page_hash(level)` or similar metadata retrieval functions are needed directly on the trait, or if `TimeHierarchyManager` will manage this state itself (currently, it tracks `last_finalized_page_hashes` internally after loading/storing).

use crate::core::page::JournalPage;
use crate::types::error::CJError; // Or a specific StorageError

#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync + 'static { // Ensure these bounds match actual needs
    async fn store_page(&self, page: &JournalPage) -> Result<(), CJError>;
    async fn load_page(&self, level: u8, page_id: u64) -> Result<Option<JournalPage>, CJError>;
    async fn page_exists(&self, level: u8, page_id: u64) -> Result<bool, CJError>;
    // Potentially other methods for managing storage state or metadata, e.g.:
    // async fn load_last_finalized_page_hash(&self, level: u8) -> Result<Option<[u8; 32]>, CJError>;
}

### Task 2.2: Implement Page Finalization and Persistence in `TimeHierarchyManager` - **COMPLETED**

**File**: `src/core/time_manager.rs`

**Objective**: Enable `TimeHierarchyManager` to finalize active pages and persist them using the `StorageBackend`.

**Details**:
*   **Finalization Condition**: Determine when an active `JournalPage` should be finalized. This could be based on:
    *   Its time window expiring (calculated from `page.creation_timestamp`, `page.level`, and `Config::time_hierarchy` settings).
    *   Reaching a maximum number of content hashes (if defined in `Config`).
    *   Explicit finalization command (future consideration).
*   **Persistence**: Upon finalization, the `JournalPage` (including its calculated `MerkleRoot` and `PageHash`) should be passed to `StorageBackend::store_page`.
*   **State Update**: `TimeHierarchyManager` must update its internal state:
    *   The finalized page should no longer be considered an 'active page' for its level.
    *   The `last_finalized_page_hashes` map for the level should be updated with the finalized page's hash.
    *   A new active page for that level should be created (or prepared to be created on next leaf addition), linking to the just-finalized page via `prev_page_hash`.
*   **Error Handling**: Robustly handle potential errors from the storage backend during persistence.
*   **Integration**: This logic will primarily be integrated into `TimeHierarchyManager::add_leaf` (to check if the current page needs finalization before or after adding the new leaf) or as part of a periodic task within `TimeHierarchyManager`.
*   **Status (2025-05-31)**: Core logic implemented. `add_leaf` now correctly adds content, calls `page.recalculate_merkle_root_and_page_hash()`, checks finalization conditions (max items, time window), stores the page via `StorageBackend`, updates `last_finalized_page_hashes`, and removes the page from active pages. Error handling for storage ops is via `?` operator.

### Task 2.2: Implement `FileStorage`

**File**: `src/storage/file.rs`

**Objective**: Implement file-based persistence.

**Details**:
*   Adhere to directory structure: `/journal/level_<L>/page_<ID>.json`.
*   Use `serde_json` for `JournalPage` serialization/deserialization.
*   Create `.civicjournal-time` marker file at the base path.
*   Manage `max_open_files` if applicable (more advanced).

### Task 2.3: Implement `MemoryStorage`

**File**: `src/storage/memory.rs`

**Objective**: Implement basic in-memory storage (useful for testing).

**Details**:
*   Use `std::collections::HashMap` or `dashmap::DashMap` for thread-safe concurrent access.
    *   Key: `(u8, u64)` for `(level, page_id)`.
    *   Value: `JournalPage`.

## 3. Phase 3: Time Hierarchy & Roll-up

This phase implements the logic for managing time levels and aggregating pages.

### Task 3.1: Implement `TimeHierarchyManager` (Conceptual)

**Location**: Likely a new module, e.g., `src/journal_manager.rs` or within `src/time/hierarchy.rs`.

**Objective**: Orchestrate page management across time levels.

**Details**:
*   Keep track of active (open) pages for each configured time level.
*   Determine the correct level 0 page for a new `JournalLeaf` based on its timestamp and `TimeHierarchyConfig`.
*   Requires access to `Config`.

### Task 3.2: Implement Page Lifecycle Management

**Objective**: Handle page creation, sealing, and transitions.

**Details**:
*   **Page Creation**: Create new `JournalPage` instances when a new time slot begins for a level (e.g., a new minute starts for level 0).
*   **Page Sealing**: "Seal" a page when:
    *   It reaches `max_leaves_per_page` (from `RollupConfig`).
    *   It reaches `max_page_age_seconds` (from `RollupConfig`).
    *   On `force_rollup_on_shutdown` (if applicable).
    *   Sealing involves finalizing its Merkle root, calculating its `PageHash`, and persisting it.

### Task 3.3: Implement Roll-up Logic

**Objective**: Aggregate sealed pages into parent-level pages.

**Details**:
*   When a page at level `L` is sealed, its `PageHash` becomes a `PageContentHash::ThrallPageHash` entry in the active page of level `L+1`.
*   This may trigger a cascade of sealing and roll-ups up the hierarchy.
*   The `Thralls[]` field in `JournalPage` (if explicitly storing child PageIDs) would be populated here.

## 4. Phase 4: Basic API

This phase exposes core functionality.

### Task 4.1: Implement `Append Leaf` API

**Location**: `src/api/sync_api.rs` and/or `src/api/async_api.rs`.

**Objective**: Provide a function to add a new journal entry.

**Details**:
*   Input: `container_id: String`, `delta_payload: String` (and potentially `timestamp: DateTime<Utc>`).
*   Logic:
    1.  Determine the correct level 0 `JournalPage` (or create if necessary) via `TimeHierarchyManager`.
    2.  Get `PrevHash` from the last leaf in that page (if any).
    3.  Create `JournalLeaf`.
    4.  Add `leaf.leaf_hash` to the page's `content_hashes`.
    5.  Update the page's Merkle root (can be deferred until sealing for performance).
    6.  Persist the leaf (if leaves are stored individually) or update the page in storage.
    7.  Check if the page needs to be sealed and rolled up.

### Task 4.2: Implement `Get Page` API

**Location**: `src/api/sync_api.rs` and/or `src/api/async_api.rs`.

**Objective**: Retrieve a specific `JournalPage`.

**Details**:
*   Input: `level: u8`, `page_id: u64`.
*   Logic: Use the `StorageBackend` to load the page.

## 5. Phase 5: Enhancements & Remaining Features

These are crucial but can be built upon the core established in Phases 1-4.

*   **Task 5.1: Query Engine**: Implement advanced queries (leaf inclusion proofs, page-chain integrity, state replay, time-range, container-centric).
*   **Task 5.2: External Anchoring (`TSProof`)**: Integrate with a Time Stamping Authority (TSA) or OpenTimestamps to get `TSProof` for `MerkleRoot`s. Store and retrieve these proofs.
*   **Task 5.3: FFI (C and WASM bindings)**: Expose the public API.
*   **Task 5.4: Metrics and Retention Policies**: Implement functionality based on `[metrics]` and `[retention]` config sections.
*   **Task 5.5: Comprehensive Testing**: Unit, integration, and end-to-end tests for all features.

## 6. Addressing Existing Issues & Code Hygiene

These tasks should be addressed alongside development or as dedicated efforts.

*   **Task 6.1: Add `async_api` feature to `Cargo.toml`**:
    ```toml
    # Cargo.toml
    [features]
    default = ["file-storage", "zstd-compression", "logging", "async"]
    # ...
    async_api = [] # Add this if it's a distinct feature
    # If async API depends on the "async" feature:
    # async_api = ["async"]
    ```
    If `async_api.rs` is always part of the `async` feature, then code using it should be `#[cfg(feature = "async")]`. If it's a separate opt-in, declare it.
*   **Task 6.2: Clarify Error Strategy**:
    *   Review `src/error.rs`, `src/config/error.rs`, `src/types/error.rs`.
    *   Ensure `CJError` in `src/error.rs` is the primary library error.
    *   Implement `From` traits for converting specific errors (e.g., `ConfigError`, `StorageError`) into `CJError`.
*   **Task 6.3: Clean Up Unused Imports**: Regularly remove unused imports as code is implemented.
*   **Task 6.4: Resolve `cfg` warnings**: Ensure all `#[cfg]` attributes correspond to declared features or valid configurations.

## 7. Tooling & Conventions

*   **Code Formatting**: Use `rustfmt`.
*   **Linting**: Use `clippy`.
*   **Continuous Integration (CI)**: Set up CI (e.g., GitHub Actions) to run checks, tests, format, and clippy on pushes/PRs.

This plan provides a roadmap. Tasks within phases can often be parallelized or reordered based on dependencies and priorities.
