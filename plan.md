# CivicJournal Time - Development Plan

## Status Update (2025-06-05)

### Completed Tasks

- [x] Implement core time hierarchy management
- [x] Add page finalization logic
- [x] Implement rollup mechanism
- [x] Add comprehensive test coverage
- [x] Fix test reliability issues
- [x] Document architecture and test patterns
- [x] Fix Async Append Leaf API: Resolved compilation errors and test failures related to the async Append Leaf API and associated configuration changes (TimeLevel struct updates, validation logic, global ID synchronization in tests). All tests now pass under `async_api` feature.
- [x] Code Cleanup: Addressed compiler warnings (unused imports, missing documentation) after recent fixes.
- [x] Implement Retention Policies: Enhanced `TimeHierarchyManager::apply_retention_policies` to support per-level retention (`KeepIndefinitely`, `DeleteAfterSecs`, `KeepNPages`) with fallback to global config. Added comprehensive tests for various policy scenarios.
- [x] Implement asynchronous `backup_journal` for `FileStorage` (zip-based).
- [x] Implement synchronous and asynchronous restore functionality for `FileStorage`.
- [x] Refactor common test configuration to `src/test_utils.rs`.
- [x] Resolve various compilation issues and warnings related to async backup and test refactoring.
- [x] Refactor `JournalPage` to store full `JournalLeaf` objects (L0) or thrall hashes (L1+) using `PageContent` enum, removing `PageContentHash` and updating all related logic.
- [x] Stabilize FileStorage & Core: Resolved numerous compilation errors and test failures in `FileStorage`, `core::page`, and related test suites. This included fixing issues from the `PageContentHash` removal, addressing duplicate test module definitions, and resolving a panic in Merkle tree handling within tests. All tests are now passing.
- [x] Implement Special Event-Triggered Rollups: Added capability for rollups to be triggered by external system events, complementing time/size-based triggers, and updated documentation.
- [x] Implement `.cjt` file format with 6-byte header for journal page files.
- [x] Add compression support (Zstd, Lz4, Snappy) for stored pages.
- [x] Implement error handling for storage operations, including simulation of storage errors for testing.
- [x] Update all documentation to reflect current implementation details.
- [x] Stabilize C and WASM FFI bindings for Turnstile and query features.
- [x] Implement Query Engine with core query functionality including leaf inclusion proofs, state reconstruction, delta reports, and page chain integrity checks.


## Next Steps

### Short Term (Next 2 Weeks)

1. **API Development**
   - [x] Implement core async API for leaf appending and page retrieval
   - [x] Add sync API wrapper for blocking operations
   - [x] Implement Query Interface (`src/query/`):
     - [x] **`get_leaf_inclusion_proof(leaf_hash: [u8; 32])`**
       - [x] Complete implementation in `FileStorage` and `MemoryStorage`
       - [x] Add comprehensive tests for proof generation and verification
       - [ ] Optimize with more efficient page searching and indexing
     - [x] **`reconstruct_container_state(container_id: String, at_timestamp: DateTime<Utc>)`**
       - [x] Basic implementation for state reconstruction
       - [ ] Add performance optimizations for large datasets
       - [x] Add tests for various state reconstruction scenarios
     - [x] **`get_delta_report(container_id: String, from: DateTime<Utc>, to: DateTime<Utc>)`**
       - [x] Implement time-range based querying
       - [ ] Add pagination support for large result sets
     - [x] **`get_page_chain_integrity(level: u8, from: Option<u64>, to: Option<u64>)`**
       - [x] Implement chain verification logic
       - [x] Add tests for chain verification
   - [ ] Document API usage with examples
   - [ ] Add OpenAPI/Swagger documentation for HTTP endpoints

2. **Testing & Validation**
   - [ ] Add unit tests for all API endpoints
   - [ ] Implement integration tests for end-to-end flows
   - [ ] Add performance benchmarks for critical paths
   - [ ] Document test coverage and add coverage reporting

3. **Testing Gaps**
   - [ ] Add tests for parent levels with `max_leaves_per_page > 1`
   - [ ] Implement tests for hierarchical rollups triggered by `max_page_age_seconds`
   - [ ] Add edge case tests for rollup behavior
   - [ ] Test error conditions and recovery scenarios

3. **Documentation Updates**
   - [x] Update `plan.md` with current status (completed)
   - [ ] Update `ROLLUP.md` with latest implementation details
   - [ ] Update `ARCHITECTURE.md` with current module structure
   - [ ] Ensure all public APIs have complete documentation
   - [ ] Add examples for common use cases

4. **Performance Optimization**
   - [ ] Profile rollup operations
   - [ ] Optimize page storage and retrieval
   - [ ] Add metrics collection
   - [ ] Implement caching for frequently accessed pages

3. **Storage Improvements**
   - [x] Add compression support
   - [x] Implement retention policies
   - [x] Add backup functionality (async, zip-based for FileStorage)
   - [x] Add restore functionality (async, zip-based for FileStorage)

### Storage Backend Enhancements

- **Implement `load_leaf_by_hash` in `FileStorage` and `MemoryStorage`**:
  - This is a required method of the `StorageBackend` trait.
  - Current implementations are stubs returning `Ok(None)`.
  - **Context**: `JournalPage` (L0) now stores full `JournalLeaf` objects within its `PageContent::Leaves(Vec<JournalLeaf>)` variant. Higher level pages store `PageContent::ThrallHashes(Vec<[u8; 32]>)`.
  - **Implementation Strategy for `load_leaf_by_hash`**:
    - The method will need to iterate through relevant L0 pages (initially active, then expanding to stored/archived pages) to find the page containing the leaf.
    - Once the correct L0 page is loaded, the `JournalLeaf` can be retrieved directly from its `content`.
    - An index (e.g., `DashMap<LeafHash, (Level, PageID)>`) could be introduced later for performance, but the initial implementation will rely on page scanning.

### Current Refactoring & Cleanup (Immediate Next Steps)

- [x] **Merge Test Modules in `src/storage/file.rs`**: Consolidated test modules and resolved warnings.
- [x] **Address Compiler Warnings**: Addressed all compiler warnings, including documentation and dead code.
- [ ] **Code Organization**: Review and reorganize modules for better maintainability
- [ ] **Error Handling**: Standardize error types and improve error messages

### Medium Term (Next 2 Months)

1. **Performance & Scalability**
   - [ ] Implement indexing for faster leaf lookups
   - [ ] Add support for parallel processing of rollups
   - [ ] Optimize memory usage for large datasets
   - [ ] Add support for batch operations

2. **Advanced Features**
   - [ ] Implement time travel queries
   - [ ] Add support for custom time zones
   - [ ] Add support for custom rollup functions
   - [ ] Implement data validation schemas

3. **Distributed Features**
   - [ ] Add replication support
   - [ ] Implement sharding
   - [ ] Add consensus protocol for distributed operation

4. **Archival & Cold Storage**
   - [ ] Implement archival snapshotting
   - [ ] Add support for cold storage integration
   - [ ] Design and implement verification for archived data

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
    pub leaf_id: u64,                    // Unique, incrementing ID for the leaf
    pub timestamp: i64,                  // Unix timestamp (seconds) of when the leaf was recorded
    pub prev_hash: Option<[u8; 32]>,     // Hash of the previous leaf in the same container's logical chain (if any)
    pub container_id: String,            // Identifier for the data entity this leaf pertains to
    pub delta_payload: Vec<u8>,          // The actual data/change, as a byte vector
    pub leaf_hash: [u8; 32],             // SHA256 hash of key leaf fields
}

impl JournalLeaf {
    pub fn new(
        timestamp: i64,
        prev_hash: Option<[u8; 32]>,
        container_id: String,
        delta_payload: Vec<u8>,
    ) -> Self {
        let leaf_id = get_next_leaf_id(); // This needs a proper source
        let mut hasher = Sha256::new();
        hasher.update(leaf_id.to_be_bytes());
        hasher.update(timestamp.to_string().as_bytes());
        if let Some(ph) = prev_hash {
            hasher.update(ph);
        }
        hasher.update(container_id.as_bytes());
        hasher.update(delta_payload);
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
*   Store `Leaves(Vec<JournalLeaf>)` for level 0 or `ThrallHashes(Vec<[u8; 32]>)` for higher levels.
*   Implement `PageHash` calculation.

```rust
// src/core/page.rs (Illustrative)
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use serde::{Serialize, Deserialize};

// Placeholder for a global or level-specific unique ID generator
fn get_next_page_id() -> String { /* ... */ "0".to_string() }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum PageContent {
    Leaves(Vec<JournalLeaf>),
    ThrallHashes(Vec<[u8; 32]>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JournalPage {
    pub page_id: String,                 // Unique identifier for the page
    pub level: u8,                       // Time-hierarchy level (0-6)
    pub start_time: i64,                 // Unix timestamp for the start of the page's time window
    pub end_time: i64,                   // Unix timestamp for the end of the page's time window
    pub content: PageContent,            // Enum: Leaves or ThrallHashes
    pub merkle_root: [u8; 32],           // Root of Merkle tree over the page's content
    pub prev_page_hash: Option<[u8; 32]>,// Hash of the previous JournalPage at the same level (if any)
    pub page_hash: [u8; 32],             // SHA256 hash of key page fields
    pub ts_proof: Option<Vec<u8>>,       // External timestamp proof for MerkleRoot (e.g., RFC3161, optional)
}

impl JournalPage {
    pub fn new(
        level: u8,
        start_time: i64,
        end_time: i64,
        content: PageContent,
        merkle_root: [u8; 32],
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
*   **Status (As of 2025-05-31):** Core logic implemented. Recent work involved extensive unit testing and fixes (e.g., `test_add_leaf_to_new_page_and_check_active`, `test_single_rollup_max_items`) to ensure correct behavior, especially with `max_leaves_per_page = 1` leading to immediate finalization and rollup. Sealing due to `max_page_age_seconds` and `force_rollup_on_shutdown` also covered by existing logic and tests.

### Task 3.3: Implement Roll-up Logic

**Objective**: Aggregate sealed pages into parent-level pages.

**Details**:
*   When a page at level `L` is sealed, its `PageHash` becomes a `PageContentHash::ThrallPageHash` entry in the active page of level `L+1`.
*   This may trigger a cascade of sealing and roll-ups up the hierarchy.
*   The `Thralls[]` field in `JournalPage` (if explicitly storing child PageIDs) would be populated here.
*   **Status (As of 2025-05-31):** Core logic implemented via `perform_rollup` method. Recent work involved extensive unit testing and fixes to validate single and cascading rollups, ensuring correct `prev_page_hash` linking and finalization propagation, particularly stressed by `max_leaves_per_page = 1` configuration. Key tests like `test_add_leaf_to_new_page_and_check_active` and `test_single_rollup_max_items` now pass, confirming this behavior.

## 4. Phase 4: Basic API

This phase exposes core functionality.

### Task 4.1: Implement `Append Leaf` API

*   **Objective**: Provide a public API function to append new leaves to the journal.
*   **Status (As of 2025-05-31):**
    *   Initial implementation complete in `src/api/async_api.rs` under the `async_api` feature flag.
    *   `Journal` struct created, encapsulating `TimeHierarchyManager`.
    *   `Journal::new(config: &'static Config)` method implemented to initialize the journal with configured storage.
    *   `Journal::append_leaf(...)` async method implemented to add new leaves.
    *   Basic unit test `test_journal_new_and_append` passes, verifying instantiation and single leaf append.
    *   `Journal` struct is re-exported from `src/api/mod.rs`.
*   **Next Steps:**
    1.  **Comprehensive Testing:** Add more tests for multiple appends, appends triggering rollups, and error conditions.
    2.  **API Documentation:** Add detailed `rustdoc` comments for the `Journal` struct and its methods.
    3.  **Review & Document Usage Pattern:** Confirm and document the library usage pattern (e.g., `init()` then `Journal::new(config())`).

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
*   **Status (As of 2025-05-31):**
    *   Core `append_leaf` logic implemented in both sync and async APIs.
    *   Extensive work on `TimeHierarchyManager` for page creation, sealing, and rollup logic, including use of per-level configurations.
    *   Robust error handling for storage backend failures (e.g., during `store_page`) implemented. This includes failure injection capabilities in `MemoryStorage` for testing.
    *   Comprehensive async tests (`test_journal_append_leaf_storage_error`) verify correct error propagation.
    *   Related cascading rollup tests (`test_cascading_rollup_max_items`) fixed and passing.

### Task 4.2: Implement `Get Page` API

**Location**: `src/api/sync_api.rs` and/or `src/api/async_api.rs`.

**Objective**: Retrieve a specific `JournalPage`.

**Details**:
*   Input: `level: u8`, `page_id: u64`.
*   Logic: Use the `StorageBackend` to load the page.
*   **Status (As of 2025-05-31):**
    *   **Async API (`src/api/async_api.rs`):**
        *   Implemented `Journal::get_page(level: u8, page_id: u64)`.
        *   Added `PageNotFound { level: u8, page_id: u64 }` error variant to `CJError`.
        *   Added `TimeHierarchyManager::get_page_from_storage` async method for controlled storage access.
        *   Includes test `test_journal_get_non_existent_page`.
    *   **Sync API (`src/api/sync_api.rs`):**
        *   Implemented `Journal::new(&'static Config)` and `Journal::get_page(level: u8, page_id: u64)`.
        *   Uses a Tokio runtime (`rt.block_on(...)`) to call the async `TimeHierarchyManager::get_page_from_storage`.
        *   Includes test `test_journal_get_non_existent_page_sync`.
    *   Both implementations correctly handle `PageNotFound` errors.


## X. Phase X: Backup and Restore

### Task X.1: Implement Journal Backup

*   **Objective**: Provide functionality to back up the entire journal to a specified location.
*   **Status (As of 2025-06-01):** Completed. The `FileStorage::backup_journal` method (trait implementation) creates a zip archive of the journal directory at the specified `backup_path`.
*   **Location**: `src/storage/file.rs`
*   **Details**:
    *   The backup is created as a zip file (e.g., `backup.zip`).
    *   Uses `spawn_blocking` for I/O, writes to a temporary file, and then atomically renames it to the final backup path.
    *   The inherent method `backup_journal_to_directory_raw` (previously `backup_journal`) still exists for raw directory copies if needed, but the primary mechanism is zip-based.
    *   `test_backup_journal` was updated to verify:
        *   Correct creation of the backup zip file.
        *   Presence of expected level directories and page files within the unzipped archive.
    *   Resolved `PermissionDenied` error on Windows by ensuring the zip file path was not mistakenly treated as a directory.
    *   All tests pass.
*   **Next Steps:**
    1.  Consider adding a backup manifest file (see Task X.3).

### Task X.2: Implement Journal Restore

*   **Objective**: Provide functionality to restore a journal from a zip backup.
*   **Status (As of 2025-06-01):** Completed.
*   **Location**: `src/storage/file.rs` (trait `StorageBackend` and `FileStorage` implementation).
*   **Details**:
    *   The `restore_journal` method takes a `backup_path` (path to the zip file) and a `target_journal_dir` (absolute path to the directory where the journal contents should be restored, e.g., `FileStorage_base_path/journal/`).
    *   The method first clears the `target_journal_dir` by removing it and recreating it. This ensures a clean restore and acts as the overwrite strategy.
    *   It then extracts the contents of the backup zip file into the `target_journal_dir`.
    *   Uses `spawn_blocking` for file I/O operations during extraction.
    *   `test_restore_journal` was implemented to verify:
        *   Setup of source storage, creation of pages, and backup to a zip file.
        *   Setup of target storage, including placing a dummy file to ensure it's removed by restore.
        *   Successful restoration from the zip backup.
        *   Verification that the dummy file is gone.
        *   Verification that all original pages exist in the target storage (using `page_exists` and direct file system checks).
        *   Loading of a restored page and comparison of its metadata with the original.
    *   All tests pass.
*   **Next Steps:**
    1.  Consider more robust error handling for corrupted zip files during restore.
    2.  Integrate with backup manifest (Task X.3) for enhanced verification during restore, if implemented.

### Task X.3: Implement Backup Manifest File

*   **Objective**: Create a manifest file within each backup to store metadata and aid in verification/restore.
*   **Status (As of 2025-05-31):** Not started.
*   **Details**:
    *   **Backup Manifest File (Task X.3) - COMPLETED**
        *   **Implemented:**
            *   Defined a JSON manifest (`backup_manifest.json`) structure (`BackupManifest`, `ManifestFileEntry`, etc.) with manifest versioning, backup timestamp, source storage details (type, path, compression), backup tool version, and a list of all backed-up journal page files.
            *   Each file entry in the manifest includes its relative path (using `/` separators), SHA256 hash of its *uncompressed* page data (post-header), original uncompressed size, and its stored (potentially compressed) size within the backup.
            *   The main `FileStorage::backup_journal` function (which creates a Zip archive) now generates this manifest. It dynamically determines page-specific compression, decompresses page data for hashing and original size calculation, and includes `backup_manifest.json` at the root of the zip.
            *   `FileStorage::restore_journal` now attempts to read `backup_manifest.json` from the backup archive, parse it, and log key details. It proceeds with restore even if the manifest is absent or unparsable (logging a warning).
            *   All associated tests are passing.
        *   **Note:** Full verification of backup integrity using the manifest during restore (e.g., hash checking) is a potential future enhancement beyond basic parsing and logging.

## 5. Phase 5: Enhancements & Remaining Features

These are crucial but can be built upon the core established in Phases 1-4.

*   **Task 5.1: Query Engine**: Implement advanced queries (leaf inclusion proofs, page-chain integrity, state replay, time-range, container-centric). **(Completed)**
*   **Task 5.2: External Anchoring (`TSProof`)**: Integrate with a Time Stamping Authority (TSA) or OpenTimestamps to get `TSProof` for `MerkleRoot`s. Store and retrieve these proofs.
*   **Task 5.3: FFI (C and WASM bindings)**: Expose the public API. **(Completed)**
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
