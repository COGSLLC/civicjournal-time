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
