use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use crate::core::hash::sha256_hash_concat;
use crate::error::{CJError, Result as CJResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Status of a pending ticket.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PendingStatus {
    /// Still waiting for confirmation from the database
    Pending,
    /// Ticket confirmed and chain advanced
    Committed,
    /// Ticket failed permanently after exceeding retries or corruption
    FailedPermanent,
}

/// A pending turnstile entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingEntry {
    /// Previous committed leaf hash (hex)
    pub prev_hash: String,
    /// Original payload JSON
    pub payload_json: String,
    /// Unix timestamp for the payload
    pub timestamp: u64,
    /// Number of retry attempts
    pub retry_count: u32,
    /// Last error message if a retry failed
    pub last_error: Option<String>,
    /// Current status
    pub status: PendingStatus,
}

/// An orphan event logged when a database insert fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrphanEvent {
    /// Hash of the orphan leaf itself
    pub leaf_hash: String,
    /// Hash of the payload that failed
    pub orig_hash: String,
    /// Error message from the database
    pub error_msg: String,
    /// Timestamp of the original payload
    pub timestamp: u64,
}

impl PendingEntry {
    fn new(prev_hash: String, payload_json: String, timestamp: u64) -> Self {
        PendingEntry {
            prev_hash,
            payload_json,
            timestamp,
            retry_count: 0,
            last_error: None,
            status: PendingStatus::Pending,
        }
    }
}

/// Turnstile manager handling pending tickets and persistence.
#[derive(Debug)]
pub struct Turnstile {
    prev_leaf_hash: String,
    pending: HashMap<String, PendingEntry>,
    committed: HashSet<String>,
    orphans: Vec<OrphanEvent>,
    max_retries: u32,
    storage_path: Option<PathBuf>,
    log_orphans: bool,
}

#[derive(Serialize, Deserialize)]
struct PersistedState {
    prev_leaf_hash: String,
    pending: HashMap<String, PendingEntry>,
    committed: HashSet<String>,
    orphans: Vec<OrphanEvent>,
}

impl Turnstile {
    /// Create a new turnstile instance with the given starting hash and max retries.
    pub fn new(initial_prev_hash: String, max_retries: u32) -> Self {
        Self::new_with_storage(initial_prev_hash, max_retries, None, true)
    }

    /// Create a new turnstile instance with optional persistence path and
    /// orphan logging toggle.
    pub fn new_with_storage(
        initial_prev_hash: String,
        max_retries: u32,
        storage_path: Option<PathBuf>,
        log_orphans: bool,
    ) -> Self {
        let mut ts = Turnstile {
            prev_leaf_hash: initial_prev_hash,
            pending: HashMap::new(),
            committed: HashSet::new(),
            orphans: Vec::new(),
            max_retries,
            storage_path,
            log_orphans,
        };
        let _ = ts.load_state();
        ts
    }

    fn compute_hash(prev_hash_hex: &str, payload_json: &str) -> CJResult<String> {
        // Validate JSON
        serde_json::from_str::<Value>(payload_json)?;
        let prev_raw = hex::decode(prev_hash_hex).map_err(|e| CJError::new(e.to_string()))?;
        let digest = sha256_hash_concat(&[&prev_raw, payload_json.as_bytes()]);
        Ok(hex::encode(digest))
    }

    fn state_file(path: &Path) -> PathBuf {
        path.join("turnstile_state.json")
    }

    fn load_state(&mut self) -> CJResult<()> {
        let Some(path) = &self.storage_path else { return Ok(()); };
        let file = Self::state_file(path);
        if !file.exists() {
            return Ok(());
        }
        let data = fs::read(&file)?;
        let state: PersistedState = serde_json::from_slice(&data)?;
        self.prev_leaf_hash = state.prev_leaf_hash;
        self.pending = state.pending;
        self.committed = state.committed;
        self.orphans = state.orphans;
        Ok(())
    }

    fn persist_state(&self) -> CJResult<()> {
        let Some(path) = &self.storage_path else { return Ok(()); };
        fs::create_dir_all(path)?;
        let state = PersistedState {
            prev_leaf_hash: self.prev_leaf_hash.clone(),
            pending: self.pending.clone(),
            committed: self.committed.clone(),
            orphans: self.orphans.clone(),
        };
        let data = serde_json::to_vec(&state)?;
        fs::write(Self::state_file(path), data)?;
        Ok(())
    }

    fn log_orphan_leaf(&mut self, orig_hash: &str, msg: &str, timestamp: u64) -> CJResult<()> {
        if !self.log_orphans {
            return Ok(());
        }
        let json = serde_json::json!({
            "type": "orphan",
            "orig_hash": orig_hash,
            "error_msg": msg,
            "timestamp": timestamp,
        });
        let json_str = json.to_string();
        let orphan_hash = Self::compute_hash(&self.prev_leaf_hash, &json_str)?;
        self.prev_leaf_hash = orphan_hash.clone();
        self.committed.insert(orphan_hash.clone());
        self.orphans.push(OrphanEvent {
            leaf_hash: orphan_hash,
            orig_hash: orig_hash.to_string(),
            error_msg: msg.to_string(),
            timestamp,
        });
        Ok(())
    }

    /// Append a payload to the pending list and return the ticket hash.
    pub fn append(&mut self, payload_json: &str, timestamp: u64) -> CJResult<String> {
        let ticket = Self::compute_hash(&self.prev_leaf_hash, payload_json)?;
        let entry = PendingEntry::new(self.prev_leaf_hash.clone(), payload_json.to_string(), timestamp);
        self.pending.insert(ticket.clone(), entry);
        self.persist_state()?;
        Ok(ticket)
    }

    /// Confirm or reject a ticket.
    pub fn confirm_ticket(&mut self, leaf_hash: &str, status: bool, error_msg: Option<&str>) -> CJResult<()> {
        if status {
            self
                .pending
                .remove(leaf_hash)
                .ok_or_else(|| CJError::not_found("ticket"))?;
            self.prev_leaf_hash = leaf_hash.to_string();
            self.committed.insert(leaf_hash.to_string());
            return self.persist_state();
        }

        let timestamp;
        {
            let entry = self
                .pending
                .get_mut(leaf_hash)
                .ok_or_else(|| CJError::not_found("ticket"))?;
            entry.retry_count += 1;
            if let Some(msg) = error_msg {
                entry.last_error = Some(msg.to_string());
            }
            if entry.retry_count > self.max_retries {
                entry.status = PendingStatus::FailedPermanent;
            }
            timestamp = entry.timestamp;
            if let Some(msg) = error_msg {
                self.log_orphan_leaf(leaf_hash, msg, timestamp)?;
            }
        }
        self.persist_state()
    }

    /// Retry the oldest pending entry using the provided callback.
    pub fn retry_next_pending<F>(&mut self, mut callback: F) -> CJResult<i32>
    where
        F: FnMut(&str, &str, &str) -> i32,
    {
        let leaf_hash = match self
            .pending
            .iter()
            .filter(|(_, e)| e.status == PendingStatus::Pending)
            .min_by_key(|(_, e)| e.timestamp)
            .map(|(h, _)| h.clone())
        {
            Some(h) => h,
            None => return Ok(-1),
        };

        let timestamp;
        {
            let entry = self.pending.get_mut(&leaf_hash).expect("exists");
            let computed = Self::compute_hash(&entry.prev_hash, &entry.payload_json)?;
            if computed != leaf_hash {
                entry.status = PendingStatus::FailedPermanent;
                return Ok(-2);
            }
            timestamp = entry.timestamp;
            let prev_hash = entry.prev_hash.clone();
            let payload_json = entry.payload_json.clone();
            let rc = callback(&prev_hash, &payload_json, &leaf_hash);
            if rc == 1 {
                entry.status = PendingStatus::Committed;
                self.prev_leaf_hash = leaf_hash.clone();
                self.committed.insert(leaf_hash);
                self.persist_state()?;
                return Ok(0);
            } else {
                entry.retry_count += 1;
                if entry.retry_count >= self.max_retries {
                    entry.status = PendingStatus::FailedPermanent;
                    // drop entry before logging
                } else {
                    entry.last_error = Some("retry failed".to_string());
                    // drop entry before logging
                }
            }
        }
        let entry = self.pending.get_mut(&leaf_hash).expect("exists");
        if entry.retry_count >= self.max_retries {
            self.log_orphan_leaf(&leaf_hash, "retry failed", timestamp)?;
            self.persist_state()?;
            Ok(2)
        } else {
            self.log_orphan_leaf(&leaf_hash, "retry failed", timestamp)?;
            self.persist_state()?;
            Ok(1)
        }
    }

    /// Check if a leaf exists in pending or committed lists.
    pub fn leaf_exists(&self, leaf_hash: &str) -> CJResult<bool> {
        if self.pending.contains_key(leaf_hash) || self.committed.contains(leaf_hash) {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get the latest committed leaf hash.
    pub fn latest_leaf_hash(&self) -> String {
        self.prev_leaf_hash.clone()
    }

    /// Number of pending entries.
    pub fn pending_count(&self) -> usize {
        self.pending.values().filter(|e| e.status == PendingStatus::Pending).count()
    }

    /// List up to max_entries pending hashes.
    pub fn list_pending(&self, max_entries: usize) -> Vec<String> {
        self.pending
            .iter()
            .filter(|(_, e)| e.status == PendingStatus::Pending)
            .take(max_entries)
            .map(|(h, _)| h.clone())
            .collect()
    }

    /// Access logged orphan events.
    pub fn orphan_events(&self) -> &[OrphanEvent] {
        &self.orphans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_append_and_confirm() {
        let mut ts = Turnstile::new("00".repeat(32), 3);
        let ticket = ts.append("{\"foo\":\"bar\"}", 0).unwrap();
        assert_eq!(
            ticket,
            "671fde0a1b5143bb2f28c9f08f5db13fed479b6313f56f8145260615d8cb05b4"
        );
        assert_eq!(ts.pending_count(), 1);
        ts.confirm_ticket(&ticket, true, None).unwrap();
        assert_eq!(ts.pending_count(), 0);
        assert_eq!(ts.latest_leaf_hash(), ticket);
    }

    #[test]
    fn test_retry_detects_corruption() {
        let mut ts = Turnstile::new("00".repeat(32), 1);
        let ticket = ts.append("{\"x\":1}", 0).unwrap();
        // Tamper with the stored payload to cause a hash mismatch
        ts.pending.get_mut(&ticket).unwrap().payload_json = "{}".to_string();
        let rc = ts.retry_next_pending(|_, _, _| 1).unwrap();
        assert_eq!(rc, -2);
        assert!(matches!(ts.pending.get(&ticket).unwrap().status, PendingStatus::FailedPermanent));
    }

    #[test]
    fn test_confirm_ticket_success_side_effects() {
        let mut ts = Turnstile::new("00".repeat(32), 1);
        let ticket = ts.append("{\"a\":1}", 1).unwrap();
        assert!(ts.pending.contains_key(&ticket));
        ts.confirm_ticket(&ticket, true, None).unwrap();
        // Expect removal from pending and moved to committed
        assert!(!ts.pending.contains_key(&ticket));
        assert!(ts.committed.contains(&ticket));
        assert_eq!(ts.prev_leaf_hash, ticket);
        assert_eq!(ts.pending_count(), 0);
    }

    #[test]
    fn test_confirm_ticket_failure_records_orphan_and_retries() {
        let mut ts = Turnstile::new("00".repeat(32), 0);
        let ticket = ts.append("{\"x\":1}", 5).unwrap();
        ts.confirm_ticket(&ticket, false, Some("err")).unwrap();
        // retry count should increment and exceed max_retries causing permanent failure
        let entry = ts.pending.get(&ticket).unwrap();
        assert_eq!(entry.retry_count, 1);
        assert!(matches!(entry.status, PendingStatus::FailedPermanent));
        assert_eq!(entry.last_error.as_deref(), Some("err"));
        // orphan event logged
        assert_eq!(ts.orphans.len(), 1);
        let orphan = &ts.orphans[0];
        assert_eq!(orphan.orig_hash, ticket);
        assert_eq!(orphan.error_msg, "err");
        assert_eq!(orphan.timestamp, 5);
        assert_eq!(ts.prev_leaf_hash, orphan.leaf_hash);
    }

    #[test]
    fn test_confirm_ticket_not_found() {
        let mut ts = Turnstile::new("00".repeat(32), 1);
        let err = ts.confirm_ticket("ff".repeat(32).as_str(), true, None).unwrap_err();
        match err {
            CJError::NotFound(_) => {}
            _ => panic!("expected NotFound error"),
        }
    }
}
