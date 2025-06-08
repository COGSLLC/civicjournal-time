## Revised CivicJournal-Time: Snapshot and Rollup Specification

This document details the integrated mechanism for data aggregation (Rollups) and system state capture (Snapshots) within CivicJournal-Time, emphasizing their synergistic relationship for efficient storage, verifiable history, and performant state reconstruction.

### 1. Rollup Mechanism (Aggregation & Hierarchical Folding)

The rollup mechanism is fundamental to CivicJournal-Time's hierarchical structure, grouping and aggregating `JournalLeaf` events into progressively coarser time epochs.

**1.1. Core Principles:**

*   **Immutable Storage:** Ensures every raw change (`JournalLeaf`) is committed into a Merkle-rooted `JournalPage` at Level 0.
*   **Automatic Aggregation:** `Level N` pages are aggregated into `Level N+1` pages when configured thresholds (e.g., `max_items_per_page` or `max_page_age_seconds`) are met.
*   **Hierarchical Propagation:** Page data or hashes are propagated upward into parent pages, maintaining cryptographic links and verifiability.
*   **Efficient Storage:** Discard empty pages and summarize changes to reduce data volume at higher levels.

**1.2. Payload Propagation (Net Patches - Optimized Aggregation):**

*   When a child page rolls up to its parent, it propagates **"net patches"** (a `HashMap<ObjectID, FieldPatch>`) for affected containers.
*   A `FieldPatch` represents the **final state** of a specific database field/object within that child page's epoch.
*   The parent page merges these `net_patches` by overwriting older values for the same `ObjectID`, effectively summarizing all intermediate changes within its epoch.
*   This means higher-level rollup pages contain the "end state" for every database field that was modified during their time window. The "beginning state" for an object within an epoch is implicitly derived from the state of the preceding rollup page or snapshot.

**1.3. Reconstruction Enabled by Rollups:**

*   To reconstruct the state at a specific `time T`, the system efficiently utilizes these net patches. It loads the highest-level page whose time window encompasses `T`, applies its net patches (or descends to lower levels if more granularity is needed), and then applies subsequent deltas/net patches up to `T`. This significantly reduces the need to replay all individual leaves.

### 2. Snapshot System (Full System State Capture)

The Snapshot System provides a mechanism to capture a comprehensive, "flattened" representation of the system's state at any specified `as_of_timestamp`. These snapshots serve as new verifiable baselines for storage management, query performance, and data retention.

**2.1. Goals:**

*   **System State Baseline:** Create a complete system state snapshot at a specified `as_of_timestamp`.
*   **Cryptographic Linkage:** Ensure snapshots are cryptographically linked into the existing journal hash chain, maintaining full verifiability.
*   **Archival Enablement ("Snap Off"):** Enable the archival (e.g., to cold storage) of `JournalLeaf` data and `JournalPage` files that are fully encompassed and now superseded by a successfully created snapshot. The term "snap off" refers to this archiving/deletion of older granular data **after** a new snapshot baseline is established.
*   **Verifiable History:** Maintain the ability to verify the entire journal history, including periods covered by snapshots and archived data.
*   **Queryability:** Snapshots themselves should be queryable and verifiable.

**2.2. Snapshot Creation Process:**

*   A new API method, e.g., `create_snapshot(as_of_timestamp, container_ids: Option<Vec<String>>)`, will be introduced.
*   **Granularity:** Snapshots can be system-wide or **per-container / multi-container**, allowing for flexible state capture and archival of specific data subsets.
*   **State Reconstruction for Snapshot:** To create a snapshot, the system will efficiently reconstruct the system state up to the `as_of_timestamp`. This process will heavily leverage the **net patches in higher-level rollup pages** to avoid replaying individual leaves, optimizing performance.
*   **Concurrent Operations:** Snapshot creation operates on a consistent view of the journal up to the `as_of_timestamp`. It is a **non-blocking operation**; new `JournalLeaf` appends continue to be processed and added to the live journal without interruption. The snapshot "slips in" after the last leaf accounted for, and new leaves carry on.
*   **Cryptographic Chaining:** The resulting snapshot payload (containing the flattened state) will be encapsulated within a special `JournalPage` that is cryptographically linked into the existing journal hash chain.

**2.3. Snapshot Page Level:**

*   Snapshots will reside on a **dedicated "Snapshot Level"** within the time hierarchy, distinct from the delta-log levels (Level 0, Hour, Day, etc.). This clearly distinguishes them as full state captures rather than incremental deltas or aggregated rollup pages.

**2.4. Restoration from Snapshot:**

*   System state restoration from a snapshot is achieved by:
    1.  **Loading the Snapshot:** The system's state is initialized or reset to the exact, flattened state captured in the snapshot's payload.
    2.  **Rolling Forward:** Any `JournalLeaf` deltas or rollup pages that were recorded *after* the snapshot's `as_of_timestamp` are then applied sequentially to bring the system up to the desired current or future point in time.

**2.5. "Snap Off" / Archival Management (Superseded Granular Data):**

*   The "snap off" process refers to the **archival or deletion from hot storage of individual `JournalLeaf` data and `JournalPage` files from delta-log levels (L0, L1, etc.)** once they are entirely superseded and covered by a new, cryptographically confirmed snapshot. This allows for efficient management of storage resources.
*   **Crucially, "snap off" does NOT mean deleting the snapshot itself.** The snapshot serves as the new, durable baseline for the period it covers.
*   Deleting a snapshot page (from the dedicated Snapshot Level) is a separate and highly sensitive operation. If granular data has been "snapped off" based on a snapshot, deleting that snapshot could lead to **irreversible data loss and break the journal's verifiable history** for the period it covered.
*   Therefore, policies must ensure that snapshots are retained as long as they are the authoritative record for any time period where underlying granular data has been archived or removed. Deletion of snapshots should be extremely restricted and only considered if the full history (including all granular data) is still available through other means or if the snapshot is truly obsolete and superseded by a newer, comprehensive snapshot that also covers its period.

### 2.6. Core Data Structures and File Layout

This section details the primary Rust data structures defined for the snapshot system and their locations within the `civicjournal-time` crate.

#### 2.6.1. Snapshot-Specific Structures (`src/core/snapshot.rs`)

A new file, `src/core/snapshot.rs`, has been created to house the core data structures specific to snapshots:

```rust
// src/core/snapshot.rs
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

/// Represents the captured state of a single container at the snapshot's `as_of_timestamp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotContainerState {
    /// Identifier for the container.
    pub container_id: String,
    /// Serialized state payload of the container.
    pub state_payload: Vec<u8>,
    /// Hash of the last JournalLeaf applied to this container to reach this state
    /// at or before the `as_of_timestamp`.
    pub last_leaf_hash_applied: [u8; 32],
    /// Timestamp of the last JournalLeaf applied.
    pub last_leaf_timestamp_applied: DateTime<Utc>,
}

/// The primary payload stored within a JournalPage when its content is a snapshot.
/// This structure holds all the data defining a specific snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotPagePayload {
    /// A unique identifier for this snapshot instance (user-defined or system-generated).
    pub snapshot_id: String,
    /// The specific point-in-time that this snapshot represents. All container states
    /// are as of this timestamp.
    pub as_of_timestamp: DateTime<Utc>,
    /// Timestamp indicating when this snapshot was actually created/finalized.
    pub created_at_timestamp: DateTime<Utc>,
    /// A list of all container states included in this snapshot.
    /// If `container_ids` was specified during creation, this will only contain
    /// states for those containers. Otherwise, it's system-wide.
    pub container_states: Vec<SnapshotContainerState>,
    /// Optional hash of the JournalPage on its original delta-log level that
    /// chronologically immediately precedes or contains the `as_of_timestamp`.
    /// This helps link the snapshot back to the precise point in the main journal history.
    pub preceding_journal_page_hash: Option<[u8; 32]>,
    /// Optional hash of the previous snapshot's JournalPage on the dedicated snapshot level.
    /// This forms a chronological chain of snapshots if multiple snapshots exist.
    pub previous_snapshot_page_hash_on_snapshot_level: Option<[u8; 32]>,
    /// The Merkle root calculated over all `container_states` included in this snapshot.
    /// This ensures the integrity and completeness of the set of captured container states.
    pub container_states_merkle_root: [u8; 32],
    // TODO: Consider adding metadata like version, or specific algorithm IDs if needed.
}
```

#### 2.6.2. `PageContent` Enum Update (`src/core/page.rs`)

The `PageContent` enum, located in `src/core/page.rs`, has been updated to include a variant for snapshots.

**Import added:**
```rust
// At the top of src/core/page.rs, with other crate::core imports:
use crate::core::snapshot::SnapshotPagePayload;
```

**Enum variant added:**
```rust
// Within pub enum PageContent { ... }
    /// Contains a full system snapshot payload.
    /// This variant is used for pages on the dedicated Snapshot Level.
    Snapshot(SnapshotPagePayload),
```

#### 2.6.3. Core Module Update (`src/core/mod.rs`)

The `src/core/mod.rs` file has been updated to declare the new `snapshot` module and re-export its primary types:

**Module declaration added:**
```rust
// In src/core/mod.rs
/// Defines data structures related to system snapshots.
pub mod snapshot;
```

**Type re-export added:**
```rust
// In src/core/mod.rs
pub use snapshot::{SnapshotContainerState, SnapshotPagePayload};
```


### 3. Further Considerations and Open Questions

This section outlines key areas and questions that need further design, implementation details, and thorough testing as the snapshot feature is developed.

**3.1. Configuration (`[snapshot]` section in TOML):**

The snapshot system's behavior is configured via a `[snapshot]` section in the main `config.toml` file. This corresponds to the `SnapshotConfig` struct in `src/config/mod.rs`.

Key configuration aspects include:

*   **Global Enablement:**
    *   A top-level `enabled` boolean within the `[snapshot]` section (and `SnapshotConfig` struct) to globally activate or deactivate the snapshot feature.

*   **Snapshot Level Index:**
    *   The dedicated level for snapshot pages is configured via `snapshot.dedicated_level`.
      The default value is `250`, chosen to be well above any typical rollup levels.

*   **Retention Rules (`[snapshot.retention]` / `SnapshotRetentionConfig` struct):**
    *   `enabled`: A boolean to activate snapshot retention policies. If `false`, snapshots are kept indefinitely unless manually pruned.
    *   `max_count`: An optional integer specifying the maximum number of snapshots to retain. Oldest snapshots beyond this count become candidates for pruning.
    *   `max_age_seconds`: An optional integer specifying the maximum age of a snapshot in seconds. Snapshots older than this become candidates for pruning.

*   **Automatic Creation (`[snapshot.automatic_creation]` / `SnapshotAutomaticCreationConfig` struct):**
    *   `enabled`: A boolean to activate automatic snapshot creation.
    *   `cron_schedule`: An optional cron-style string (e.g., `"0 0 3 * * *"` for daily at 3 AM UTC) for scheduling periodic snapshots. This takes precedence if set.
    *   `interval_seconds`: An optional integer specifying an interval in seconds for periodic snapshot creation (e.g., `86400` for every 24 hours). Used if `cron_schedule` is not set.

**Example TOML Structure:**

```toml
[snapshot]
# Globally enable or disable the snapshot feature.
enabled = false

# Configuration for snapshot retention policies.
[snapshot.retention]
# Whether snapshot retention policies are active.
enabled = false

# Maximum number of snapshots to retain.
# max_count = 10

# Maximum age of a snapshot in seconds (e.g., 30 days).
# max_age_seconds = 2592000

# Configuration for automatic snapshot creation.
[snapshot.automatic_creation]
# Whether automatic snapshot creation is enabled.
enabled = false

# Cron-style schedule string (e.g., daily at 3 AM UTC).
# cron_schedule = "0 0 3 * * *"

# Interval in seconds (e.g., every 24 hours).
# interval_seconds = 86400
```

**Corresponding Rust Structs (in `src/config/mod.rs`):**

```rust
// Main snapshot configuration
pub struct SnapshotConfig {
    pub enabled: bool,
    pub retention: SnapshotRetentionConfig,
    pub automatic_creation: SnapshotAutomaticCreationConfig,
    // Potentially: pub snapshot_level_index: usize, (once defined)
}

// Snapshot retention policies
pub struct SnapshotRetentionConfig {
    pub enabled: bool,
    pub max_count: Option<u32>,
    pub max_age_seconds: Option<u64>,
}

// Automatic snapshot creation
pub struct SnapshotAutomaticCreationConfig {
    pub enabled: bool,
    pub cron_schedule: Option<String>,
    pub interval_seconds: Option<u64>,
}
```

**3.2. Trigger Mechanisms:**
*   **API Endpoint:** Ensure a clear API for manual/programmatic snapshot creation (already partially defined with `create_snapshot`).
*   **Special Event Triggers:** Similar to rollups, should snapshots support application-defined "special signals" or event triggers for automated creation based on domain-specific events?
*   **Scheduled Triggers:** Built-in scheduling capabilities (e.g., daily, weekly) for regular snapshot creation.

**3.3. Concurrency and Consistency:**

The snapshot creation process is designed to be non-blocking with respect to new journal appends, ensuring system availability while providing a consistent "as-of" view of the data.

The mechanism involves the following steps:

1.  **Snapshot Initiation:**
    *   An API, such as `create_snapshot(as_of_timestamp: DateTime<Utc>, container_ids: Option<Vec<String>>)`, is invoked.
    *   A dedicated `SnapshotManager` module handles the snapshot creation workflow.

2.  **Determining the Journal Scope:**
    *   The `SnapshotManager` identifies all `JournalPage`s (from finalized storage) and `JournalLeaf` entries (from the active L0 page) required to reconstruct container states up to the specified `as_of_timestamp`.
    *   This involves:
        *   Querying the `StorageBackend` for finalized pages whose time windows are relevant (e.g., `page.start_time <= as_of_timestamp`).
        *   Requesting relevant leaves from the `TimeHierarchyManager`'s active L0 page.

3.  **Consistent Read of Active L0 Page Data:**
    *   To get a consistent view of leaves from the active L0 page that are pertinent to the `as_of_timestamp` (i.e., `leaf.timestamp <= as_of_timestamp`):
        *   The `SnapshotManager` requests these leaves from the `TimeHierarchyManager`.
        *   The `TimeHierarchyManager` briefly acquires a lock on its `active_pages` structure (specifically for L0).
        *   It clones the `JournalLeaf` entries from the active L0 page that meet the timestamp criteria.
        *   The lock is released immediately after cloning. This short-lived lock ensures that the snapshot process works with a consistent set of leaves from the active page, while new appends (especially those with timestamps after `as_of_timestamp`) can proceed with minimal interruption.

4.  **State Reconstruction:**
    *   For each container (either specified or all system containers if `container_ids` is `None`):
        *   The `SnapshotManager` reconstructs the container's state as of `as_of_timestamp`. This process is similar to the `QueryEngine::reconstruct_container_state` logic.
        *   It utilizes the data from relevant finalized pages and the cloned, consistent set of leaves obtained from the active L0 page.
        *   Since finalized pages are immutable and the L0 leaves are a consistent clone, the reconstruction accurately reflects the state at `as_of_timestamp`.

5.  **Snapshot Data Assembly:**
    *   The reconstructed state for each container is packaged into a `SnapshotContainerState` struct.
    *   A `SnapshotPagePayload` is assembled, containing:
        *   A unique `snapshot_id`.
        *   The `as_of_timestamp`.
        *   A `created_at_timestamp` (the time the snapshot process completed).
        *   A collection of `SnapshotContainerState`s.
        *   `preceding_journal_page_hash`: The hash of the most recent L0 `JournalPage` whose `end_time <= as_of_timestamp`. This cryptographically links the snapshot to the main journal sequence.
        *   `previous_snapshot_page_hash_on_snapshot_level`: The hash of the preceding snapshot on the dedicated snapshot level, if one exists, forming a chain of snapshots.
        *   `container_states_merkle_root`: A Merkle root calculated from all `SnapshotContainerState`s within this snapshot, ensuring the integrity of the snapshot's content.

6.  **Storing the Snapshot:**
    *   A new `JournalPage` is created with `PageContent::Snapshot(snapshot_payload)`.
    *   This snapshot page is written by the `SnapshotManager` to the `StorageBackend` on a dedicated "Snapshot Level," distinct from the regular time hierarchy levels. Storage of this page should be an atomic operation.

**Non-Blocking Guarantees and Coordination:**

*   **Reading Finalized Pages:** Accessing immutable, finalized pages from storage is inherently non-blocking for new appends.
*   **Active L0 Page Access:** The critical interaction with the active L0 page involves a very short-duration lock by the `TimeHierarchyManager` solely for cloning relevant leaf data. This minimizes impact on concurrent append operations.
*   **Independent Reconstruction:** The state reconstruction and snapshot assembly processes are performed by the `SnapshotManager` (potentially in a separate background task or thread pool) using the cloned data, without holding long-lived locks on the primary journaling path or active page structures.
*   **Rollup Queue Coordination:** The snapshot process operates largely independently of the rollup queue. Since snapshots rely on already finalized pages or a consistent clone of active L0 data, ongoing rollups of older data do not affect the consistency of a snapshot being created for a specific `as_of_timestamp`. The `SnapshotManager` will read from the same finalized page state that the rollup mechanism would.

**3.4. Query Engine Integration:**
*   **Snapshot Page Recognition:** Update the `QueryEngine` to recognize and correctly interpret `JournalPage` instances containing `PageContent::Snapshot`.
*   **State Reconstruction Optimization:** Modify query logic to leverage snapshots for faster state reconstruction. If a query's target time `T_query` is after a snapshot's `as_of_timestamp` (`T_snapshot`), the engine should be able to start from `T_snapshot` and apply subsequent deltas, rather than replaying from the beginning of time or an earlier rollup.

**3.5. Archival Safety ("Snap Off" Process):**
*   **Policy Enforcement:** Define and implement strict policies for the "snap off" process.
    *   Consider making snapshot pages (on the Snapshot Level) non-deletable by default through the standard archival process.
    *   If deletion of underlying rollup/leaf files is permitted after a snapshot(this should be configurable), explore mechanisms like a two-step confirmation or a "grace period" before actual deletion.
*   **Integrity Checks:** Ensure robust integrity checks are in place before any granular data is removed based on a snapshot's existence.

**3.6. Testing and Validation Strategy:**
*   Develop comprehensive unit and integration tests covering:
    *   Snapshot creation for system-wide state.
    *   Snapshot creation for specific subsets of containers.
    *   State restoration from a snapshot.
    *   State restoration from a snapshot plus subsequent deltas/rollup pages.
    *   Verification of snapshot integrity.
    *   Edge cases, such as:
        *   Snapshots at the very beginning or end of a journal's life.
        *   Handling of out-of-order leaves if they occur near snapshot boundaries (though the system aims to prevent this).
        *   Snapshots created during high write loads.
    *   Interaction between snapshot creation and concurrent rollup operations.
    *   "Snap off" process and its impact on querying and data availability.

---

