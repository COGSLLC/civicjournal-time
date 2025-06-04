// src/core/leaf.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
// Remove unused import if CJError is not used in this file after changes.
// use crate::error::CJError;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) static NEXT_LEAF_ID: AtomicU64 = AtomicU64::new(0);

/// A single “delta” (change) record for a container.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JournalLeaf {
    /// unique, incrementing
    pub leaf_id: u64,
    /// e.g. 2025-06-01T12:00:00Z
    pub timestamp: DateTime<Utc>,
    /// SHA256 of previous LeafHash. None if this is the first leaf in a chain.
    pub prev_hash: Option<[u8; 32]>,
    /// “proposal:XYZ” or “user:ABC”
    pub container_id: String,
    /// JSON/YAML patch or full record
    pub delta_payload: serde_json::Value,
    /// SHA256(LeafID ∥ Timestamp ∥ PrevHash ∥ ContainerID ∥ DeltaPayload)
    pub leaf_hash: [u8; 32],
}

impl JournalLeaf {
    /// Creates a new `JournalLeaf`.
    ///
    /// Initializes a leaf with a unique, atomically generated `leaf_id`, a timestamp,
    /// an optional previous hash, a container ID, and a delta payload. It then calculates
    /// the `leaf_hash` based on these fields.
    ///
    /// # Arguments
    ///
    /// * `timestamp` - The timestamp for when the event or data represented by this leaf occurred.
    /// * `prev_hash` - An optional SHA256 hash of a preceding `JournalLeaf` or related entry, forming a chain.
    /// * `container_id` - An identifier for the data container or source related to this leaf.
    /// * `delta_payload` - The actual data payload, typically a JSON value representing a change or event.
    ///
    /// # Errors
    ///
    /// Returns `serde_json::Error` if serialization of the `delta_payload` to JSON bytes fails.
    pub fn new(
        timestamp: DateTime<Utc>,
        prev_hash: Option<[u8; 32]>,
        container_id: String,
        delta_payload: serde_json::Value,
    ) -> Result<Self, serde_json::Error> {
        let leaf_id = NEXT_LEAF_ID.fetch_add(1, Ordering::SeqCst);
        
        let mut hasher = Sha256::new();
        hasher.update(leaf_id.to_be_bytes());
        hasher.update(timestamp.to_rfc3339().as_bytes());
        if let Some(ph) = prev_hash {
            hasher.update(ph);
        }
        hasher.update(container_id.as_bytes());
        
        // Serialize delta_payload to bytes for hashing.
        // Using serde_json::to_vec for potentially more canonical representation than to_string.
        // Note: For true canonical JSON, a library like serde_jcs might be needed if complex objects
        // with varying key orders are expected and must produce identical hashes.
        let payload_bytes = serde_json::to_vec(&delta_payload)?;
        hasher.update(&payload_bytes);
        
        let leaf_hash: [u8; 32] = hasher.finalize().into();

        Ok(JournalLeaf {
            leaf_id,
            timestamp,
            prev_hash,
            container_id,
            delta_payload,
            leaf_hash,
        })
    }

    // TODO: Implement validation methods if needed
}

/// Represents the actual data payload for a leaf, versioned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LeafData {
    V1(LeafDataV1),
}

/// Version 1 of the leaf data payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LeafDataV1 {
    pub timestamp: DateTime<Utc>,      // Timestamp of the data content itself
    pub content_type: String,        // e.g., "text/plain", "application/json-patch+json"
    pub content: Vec<u8>,            // The actual data
    pub author: String,              // Identifier for the author/source of this specific data
    pub signature: String,           // Cryptographic signature of the content by the author
}

#[cfg(test)]
mod tests {
    use crate::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids}; // Import for test synchronization
    use super::*; // Imports JournalLeaf and its methods
    use chrono::Utc;
    use serde_json::json;

    #[tokio::test]
    async fn test_create_journal_leaf() {
        let _guard = SHARED_TEST_ID_MUTEX.lock().await; // Lock for test synchronization
        reset_global_ids(); // Reset global ID counters
        let timestamp = Utc::now();
        let prev_hash = None;
        let container_id = "test_container".to_string();
        let delta_payload = json!({ "key": "value" });

        let leaf_result = JournalLeaf::new(
            timestamp,
            prev_hash,
            container_id.clone(),
            delta_payload.clone(),
        );

        assert!(leaf_result.is_ok());
        let leaf = leaf_result.unwrap();

        assert_eq!(leaf.timestamp, timestamp);
        assert_eq!(leaf.prev_hash, prev_hash);
        assert_eq!(leaf.container_id, container_id);
        assert_eq!(leaf.delta_payload, delta_payload);
        
        // Check that leaf_id is set (first leaf gets ID 0)
        assert_eq!(leaf.leaf_id, 0);

        // Verify leaf_hash is not all zeros (basic check that hashing occurred)
        assert_ne!(leaf.leaf_hash, [0u8; 32]);

        // Test with a previous hash (second leaf gets ID 1)
        let prev_hash_value = [1u8; 32];
        let leaf_with_prev_hash_result = JournalLeaf::new(
            timestamp,
            Some(prev_hash_value),
            container_id.clone(),
            delta_payload.clone(),
        );
        assert!(leaf_with_prev_hash_result.is_ok());
        let leaf_with_prev_hash = leaf_with_prev_hash_result.unwrap();
        assert_eq!(leaf_with_prev_hash.leaf_id, 1);
        assert_eq!(leaf_with_prev_hash.prev_hash, Some(prev_hash_value));
        assert_ne!(leaf_with_prev_hash.leaf_hash, leaf.leaf_hash, "Leaf hash should differ when prev_hash differs");
    }
}
