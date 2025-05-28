use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use thiserror::Error;

use crate::recovery::{Journal, JournalEntry, Fork};
use crate::schema::CivicObject;

#[derive(Debug, Error)]
pub enum ForkError {
    #[error("Invalid branch")]
    InvalidBranch,
    
    #[error("Fork resolution failed: {0}")]
    ResolutionFailed(String),
    
    #[error("No fork detected")]
    NoForkDetected,
    
    #[error("Multiple resolution strategies applicable")]
    MultipleStrategiesApplicable,
    
    #[error("Cannot determine canonical branch")]
    CannotDetermineCanonicalBranch,
}

/// Enhanced fork detection and resolution capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkDetails {
    /// The fork identifier
    pub id: String,
    /// When the fork was detected
    pub detected_at: DateTime<Utc>,
    /// The sequence number where the fork occurred
    pub fork_point_sequence: u64,
    /// The hash where the fork occurred
    pub fork_point_hash: [u8; 32],
    /// The branches that diverge from the fork point
    pub branches: Vec<BranchInfo>,
    /// Whether the fork has been resolved
    pub resolved: bool,
    /// Which branch was selected as canonical (if resolved)
    pub canonical_branch_id: Option<String>,
    /// When the fork was resolved (if resolved)
    pub resolved_at: Option<DateTime<Utc>>,
    /// The resolution strategy that was used (if resolved)
    pub resolution_strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    /// Branch identifier
    pub id: String,
    /// The first entry's hash in this branch
    pub branch_hash: [u8; 32],
    /// The sequence number of the first entry in this branch
    pub sequence: u64,
    /// The timestamp of the first entry in this branch
    pub timestamp: DateTime<Utc>,
    /// The branch length (number of entries)
    pub length: usize,
    /// The branch creator/originator
    pub originator: Option<String>,
    /// A computed trust score (0.0-1.0) for this branch
    pub trust_score: f64,
}

/// Resolution strategies for forks
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ResolutionStrategy {
    /// Use the longest branch (most entries)
    LongestBranch,
    
    /// Use the branch with the earliest timestamp
    EarliestTimestamp,
    
    /// Use the branch with the highest trust score
    HighestTrustScore,
    
    /// Use a branch with a specific originator
    SpecificOriginator(&'static str),
    
    /// Manually selected branch
    ManualSelection(&'static str),
    
    /// Merge compatible branches (if possible)
    MergeCompatible,
}

pub struct ForkDetector {
    /// The journal to analyze
    journal: Arc<Journal>,
    /// Known forks
    forks: Vec<ForkDetails>,
    /// Resolution strategies to try (in order)
    resolution_strategies: Vec<ResolutionStrategy>,
}

impl ForkDetector {
    pub fn new(journal: Arc<Journal>) -> Self {
        Self {
            journal,
            forks: Vec::new(),
            resolution_strategies: vec![
                ResolutionStrategy::HighestTrustScore,
                ResolutionStrategy::LongestBranch,
                ResolutionStrategy::EarliestTimestamp,
            ],
        }
    }
    
    /// Set the resolution strategies (in order of preference)
    pub fn with_strategies(mut self, strategies: Vec<ResolutionStrategy>) -> Self {
        self.resolution_strategies = strategies;
        self
    }
    
    /// Detect all forks in the journal
    pub fn detect_all_forks(&mut self) -> Vec<ForkDetails> {
        let forks = self.journal.detect_forks();
        let mut result = Vec::new();
        
        for fork in forks {
            // Get all entries after the fork point
            let entries: Vec<_> = self.journal.entries.iter()
                .filter(|e| e.previous_hash == fork.fork_point)
                .collect();
                
            if entries.is_empty() {
                continue;
            }
            
            // Group by branch
            let mut branches = Vec::new();
            
            for branch_hash in &fork.branches {
                let branch_entries: Vec<_> = self.journal.entries.iter()
                    .filter(|e| &e.hash() == branch_hash || self.is_descendant_of(*e, *branch_hash))
                    .collect();
                    
                if branch_entries.is_empty() {
                    continue;
                }
                
                let first_entry = branch_entries.iter()
                    .min_by_key(|e| e.sequence)
                    .unwrap();
                
                branches.push(BranchInfo {
                    id: format!("branch_{}", hex::encode(branch_hash)),
                    branch_hash: *branch_hash,
                    sequence: first_entry.sequence,
                    timestamp: first_entry.timestamp,
                    length: branch_entries.len(),
                    originator: None, // Would be extracted from entry in real impl
                    trust_score: self.calculate_trust_score(&branch_entries),
                });
            }
            
            // Get the sequence number of the fork point
            let fork_point_sequence = entries.iter()
                .map(|e| e.sequence)
                .min()
                .unwrap_or(0)
                .saturating_sub(1);
            
            // Create fork details
            let fork_details = ForkDetails {
                id: format!("fork_{}", hex::encode(fork.fork_point)),
                detected_at: Utc::now(),
                fork_point_sequence,
                fork_point_hash: fork.fork_point,
                branches,
                resolved: false,
                canonical_branch_id: None,
                resolved_at: None,
                resolution_strategy: None,
            };
            
            result.push(fork_details);
        }
        
        self.forks = result.clone();
        result
    }
    
    /// Check if an entry is a descendant of a specific branch
    fn is_descendant_of(&self, entry: &JournalEntry, branch_hash: [u8; 32]) -> bool {
        let mut current_hash = entry.previous_hash;
        
        for _ in 0..100 { // Limit the recursion depth
            if current_hash == branch_hash {
                return true;
            }
            
            let prev_entry = self.journal.entries.iter()
                .find(|e| e.hash() == current_hash);
                
            match prev_entry {
                Some(e) => current_hash = e.previous_hash,
                None => return false,
            }
        }
        
        false
    }
    
    /// Calculate a trust score for a branch
    fn calculate_trust_score(&self, entries: &[&JournalEntry]) -> f64 {
        // This is a placeholder for a real trust score calculation
        // In a real implementation, this would consider factors like:
        // - Signature verification
        // - Known originators
        // - Branch consistency
        // - External validation
        
        // For now, just return a value between 0.0 and 1.0
        let length_factor = (entries.len() as f64).min(100.0) / 100.0;
        let recency_factor = if let Some(latest) = entries.iter().max_by_key(|e| e.timestamp) {
            let age = Utc::now().signed_duration_since(latest.timestamp);
            let hours = age.num_seconds() as f64 / 3600.0;
            ((-hours / 24.0).exp()).max(0.0).min(1.0) // Decay with age
        } else {
            0.0
        };
        
        0.3 * length_factor + 0.7 * recency_factor
    }
    
    /// Resolve a fork using the configured strategies
    pub fn resolve_fork(&mut self, fork_id: &str) -> Result<BranchInfo, ForkError> {
        // Find the fork's index
        let fork_index = self.forks.iter().position(|f| f.id == fork_id)
            .ok_or(ForkError::NoForkDetected)?;
            
        // Check if fork has branches
        if self.forks[fork_index].branches.is_empty() {
            return Err(ForkError::NoForkDetected);
        }
        
        // If already resolved, return the canonical branch
        if self.forks[fork_index].resolved {
            let branch_id = self.forks[fork_index].canonical_branch_id.as_ref()
                .ok_or(ForkError::CannotDetermineCanonicalBranch)?;
                
            let branch = self.forks[fork_index].branches.iter()
                .find(|b| &b.id == branch_id)
                .ok_or(ForkError::CannotDetermineCanonicalBranch)?;
                
            return Ok(branch.clone());
        }
        
        // Create a local copy of the fork for analysis
        let fork_copy = self.forks[fork_index].clone();
        
        // Try each strategy in order
        let mut applicable_strategies = Vec::new();
        
        for strategy in &self.resolution_strategies {
            if self.is_strategy_applicable(&fork_copy, strategy) {
                applicable_strategies.push(*strategy);
            }
        }
        
        if applicable_strategies.is_empty() {
            return Err(ForkError::CannotDetermineCanonicalBranch);
        }
        
        // Use the first applicable strategy
        let strategy = &applicable_strategies[0];
        let branch = self.select_branch_using_strategy(&fork_copy, strategy)?;
        
        // Mark the fork as resolved
        self.forks[fork_index].resolved = true;
        self.forks[fork_index].canonical_branch_id = Some(branch.id.clone());
        self.forks[fork_index].resolved_at = Some(Utc::now());
        self.forks[fork_index].resolution_strategy = Some(format!("{:?}", strategy));
        
        // Actually resolve the fork in the journal
        let fork_struct = Fork {
            fork_point: fork_copy.fork_point_hash,
            branches: fork_copy.branches.iter().map(|b| b.branch_hash).collect(),
        };
        
        // Since we can't modify the Arc<Journal> directly, we'll just record the resolution
        // In a real implementation, we would clone the journal, apply the changes, and update references
        // but for simplicity we'll just mark it as resolved in our local state
            
        Ok(branch.clone())
    }
    
    /// Check if a resolution strategy is applicable to a fork
    fn is_strategy_applicable(&self, fork: &ForkDetails, strategy: &ResolutionStrategy) -> bool {
        match strategy {
            ResolutionStrategy::LongestBranch => true, // Always applicable
            
            ResolutionStrategy::EarliestTimestamp => true, // Always applicable
            
            ResolutionStrategy::HighestTrustScore => true, // Always applicable
            
            ResolutionStrategy::SpecificOriginator(originator) => {
                // Check if any branch has this originator
                fork.branches.iter().any(|b| b.originator.as_ref().map(|s| s.as_str()) == Some(originator))
            }
            
            ResolutionStrategy::ManualSelection(branch_id) => {
                // Check if this branch exists
                fork.branches.iter().any(|b| b.id == *branch_id)
            }
            
            ResolutionStrategy::MergeCompatible => {
                // Check if branches can be merged (would require deeper analysis)
                false // Not implemented in this example
            }
        }
    }
    
    /// Select a branch using the specified strategy
    fn select_branch_using_strategy(&self, fork: &ForkDetails, strategy: &ResolutionStrategy) -> Result<BranchInfo, ForkError> {
        match strategy {
            ResolutionStrategy::LongestBranch => {
                fork.branches.iter()
                    .max_by_key(|b| b.length)
                    .cloned()
                    .ok_or(ForkError::CannotDetermineCanonicalBranch)
            }
            
            ResolutionStrategy::EarliestTimestamp => {
                fork.branches.iter()
                    .min_by_key(|b| b.timestamp)
                    .cloned()
                    .ok_or(ForkError::CannotDetermineCanonicalBranch)
            }
            
            ResolutionStrategy::HighestTrustScore => {
                fork.branches.iter()
                    .max_by(|a, b| a.trust_score.partial_cmp(&b.trust_score).unwrap_or(std::cmp::Ordering::Equal))
                    .cloned()
                    .ok_or(ForkError::CannotDetermineCanonicalBranch)
            }
            
            ResolutionStrategy::SpecificOriginator(originator) => {
                fork.branches.iter()
                    .find(|b| b.originator.as_ref().map(|s| s.as_str()) == Some(originator))
                    .cloned()
                    .ok_or(ForkError::CannotDetermineCanonicalBranch)
            }
            
            ResolutionStrategy::ManualSelection(branch_id) => {
                fork.branches.iter()
                    .find(|b| b.id == *branch_id)
                    .cloned()
                    .ok_or(ForkError::CannotDetermineCanonicalBranch)
            }
            
            ResolutionStrategy::MergeCompatible => {
                // This would require a more complex implementation
                Err(ForkError::CannotDetermineCanonicalBranch)
            }
        }
    }
    
    /// Auto-resolve all forks using the configured strategies
    pub fn auto_resolve_all(&mut self) -> Vec<Result<BranchInfo, ForkError>> {
        let fork_ids: Vec<_> = self.forks.iter()
            .filter(|f| !f.resolved)
            .map(|f| f.id.clone())
            .collect();
            
        let mut results = Vec::new();
        
        for fork_id in fork_ids {
            results.push(self.resolve_fork(&fork_id));
        }
        
        results
    }
    
    /// Create a report of all forks and their resolution status
    pub fn create_report(&self) -> String {
        let mut report = String::new();
        
        report.push_str("=== CivicJournal Fork Analysis Report ===\n\n");
        report.push_str(&format!("Total forks detected: {}\n", self.forks.len()));
        report.push_str(&format!("Resolved forks: {}\n", self.forks.iter().filter(|f| f.resolved).count()));
        report.push_str(&format!("Unresolved forks: {}\n\n", self.forks.iter().filter(|f| !f.resolved).count()));
        
        if !self.forks.is_empty() {
            report.push_str("Fork Details:\n");
            report.push_str("------------\n\n");
            
            for fork in &self.forks {
                report.push_str(&format!("Fork ID: {}\n", fork.id));
                report.push_str(&format!("Detected: {}\n", fork.detected_at));
                report.push_str(&format!("Fork point: Sequence {}, Hash {}\n", 
                    fork.fork_point_sequence, 
                    hex::encode(&fork.fork_point_hash[..8]))); // Show first 8 bytes of hash
                report.push_str(&format!("Branches: {}\n", fork.branches.len()));
                
                for (i, branch) in fork.branches.iter().enumerate() {
                    report.push_str(&format!("  Branch {}: ID {}, Length {}, Trust Score {:.2}\n", 
                        i + 1,
                        branch.id,
                        branch.length,
                        branch.trust_score));
                }
                
                if fork.resolved {
                    report.push_str(&format!("Status: RESOLVED using strategy {}\n", 
                        fork.resolution_strategy.as_ref().unwrap_or(&"Unknown".to_string())));
                    report.push_str(&format!("Canonical branch: {}\n", 
                        fork.canonical_branch_id.as_ref().unwrap_or(&"Unknown".to_string())));
                    report.push_str(&format!("Resolved at: {}\n", 
                        fork.resolved_at.unwrap_or_else(Utc::now)));
                } else {
                    report.push_str("Status: UNRESOLVED\n");
                }
                
                report.push_str("\n");
            }
        }
        
        report
    }
}

// Extension trait to add advanced fork detection to Journal
pub trait ForkDetection {
    fn detect_extended_forks(&self) -> Vec<ForkDetails>;
    fn resolve_fork_with_strategy(&mut self, fork_id: &str, strategy: ResolutionStrategy) -> Result<BranchInfo, ForkError>;
}

impl ForkDetection for Journal {
    fn detect_extended_forks(&self) -> Vec<ForkDetails> {
        let detector = &mut ForkDetector::new(Arc::new(self.clone()));
        detector.detect_all_forks()
    }
    
    fn resolve_fork_with_strategy(&mut self, fork_id: &str, strategy: ResolutionStrategy) -> Result<BranchInfo, ForkError> {
        let detector = &mut ForkDetector::new(Arc::new(self.clone()))
            .with_strategies(vec![strategy]);
            
        detector.detect_all_forks();
        detector.resolve_fork(fork_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::JournalEntry;
    use chrono::TimeZone;
    
    // Helper to create a test journal with a fork
    fn create_test_journal_with_fork() -> Journal {
        let mut journal = Journal::new();
        
        // Create a chain of 5 entries
        let mut prev_hash = [0u8; 32];
        
        for i in 0..5 {
            let entry = JournalEntry {
                sequence: i,
                timestamp: Utc.with_ymd_and_hms(2023, 1, 1, 12, 0, i as u32).unwrap(),
                object_id: "test".to_string(),
                delta_hash: [0u8; 32],
                previous_hash: prev_hash,
                signature: vec![1, 2, 3], // Mock signature
            };
            
            prev_hash = entry.hash();
            journal.entries.push(entry);
        }
        
        // Create a fork at entry 3
        let fork_base_hash = journal.entries[2].hash();
        
        // Branch 1 (already created above)
        
        // Branch 2
        let branch2_entry = JournalEntry {
            sequence: 3,
            timestamp: Utc.with_ymd_and_hms(2023, 1, 1, 12, 30, 0).unwrap(), // 30 minutes later
            object_id: "test".to_string(),
            delta_hash: [1u8; 32], // Different delta
            previous_hash: fork_base_hash,
            signature: vec![1, 2, 3], // Mock signature
        };
        
        let branch2_hash = branch2_entry.hash();
        journal.entries.push(branch2_entry);
        
        // Add an entry to branch 2
        let branch2_next = JournalEntry {
            sequence: 4,
            timestamp: Utc.with_ymd_and_hms(2023, 1, 1, 13, 0, 0).unwrap(),
            object_id: "test".to_string(),
            delta_hash: [1u8; 32],
            previous_hash: branch2_hash,
            signature: vec![1, 2, 3], // Mock signature
        };
        
        journal.entries.push(branch2_next);
        
        journal
    }
    
    #[test]
    fn test_fork_detection() {
        let journal = create_test_journal_with_fork();
        
        // Detect forks
        let forks = journal.detect_extended_forks();
        
        // Should find one fork
        assert_eq!(forks.len(), 1);
        
        // Fork should have two branches
        assert_eq!(forks[0].branches.len(), 2);
    }
    
    #[test]
    fn test_fork_resolution_longest_branch() {
        let mut journal = create_test_journal_with_fork();
        
        // Detect forks
        let forks = journal.detect_extended_forks();
        
        // Resolve using longest branch strategy
        let result = journal.resolve_fork_with_strategy(&forks[0].id, ResolutionStrategy::LongestBranch);
        
        // Should successfully resolve
        assert!(result.is_ok());
        
        // Should select branch 2 (with 2 entries vs branch 1's 1 entry after the fork)
        let branch = result.unwrap();
        assert_eq!(branch.length, 2);
    }
    
    #[test]
    fn test_fork_resolution_earliest_timestamp() {
        let mut journal = create_test_journal_with_fork();
        
        // Detect forks
        let forks = journal.detect_extended_forks();
        
        // Resolve using earliest timestamp strategy
        let result = journal.resolve_fork_with_strategy(&forks[0].id, ResolutionStrategy::EarliestTimestamp);
        
        // Should successfully resolve
        assert!(result.is_ok());
        
        // Should select branch 1 (with earlier timestamp)
        let branch = result.unwrap();
        assert_eq!(branch.sequence, 3);
        assert_eq!(branch.timestamp.minute(), 0); // Branch 1 is at minute 0, branch 2 is at minute 30
    }
}
