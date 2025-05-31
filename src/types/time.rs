use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for a single time hierarchy level
/// Defines how pages at this level might be retained or deleted after rollup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RollupRetentionPolicy {
    /// Keep pages indefinitely after rollup.
    KeepIndefinitely,
    /// Delete pages after a certain duration post-rollup.
    DeleteAfterSecs(u64),
    /// Keep a certain number of rolled-up pages.
    KeepNPages(usize),
}

#[derive(Debug, Clone, Serialize, Deserialize)] // Added for TimeLevel
pub struct TimeLevel {
    /// Name of the time level (e.g., "minute", "hour")
    pub name: String,
    /// Duration of this time level in seconds
    pub duration_seconds: u64,
    /// Configuration for roll-up operations specific to this level.
    pub rollup_config: RollupConfig,
    /// Policy for retaining or deleting pages at this level after they've been rolled up.
    pub retention_policy: Option<RollupRetentionPolicy>,
}

impl TimeLevel {
    /// Create a new time level
    pub fn new(
        name: impl Into<String>,
        duration_seconds: u64,
        rollup_config: RollupConfig,
        retention_policy: Option<RollupRetentionPolicy>,
    ) -> Self {
        Self {
            name: name.into(),
            duration_seconds,
            rollup_config,
            retention_policy,
        }
    }

    /// Get the duration of this time level
    pub fn duration(&self) -> Duration {
        Duration::from_secs(self.duration_seconds)
    }
}

/// Configuration for the time hierarchy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeHierarchyConfig {
    /// List of time levels in ascending order of duration
    pub levels: Vec<TimeLevel>,
}

impl Default for TimeHierarchyConfig {
    fn default() -> Self {
        Self {
            levels: vec![
                TimeLevel::new("raw", 1, RollupConfig::default(), None), // Smallest duration, pages primarily finalize by count/age
                TimeLevel::new("minute", 60, RollupConfig::default(), Some(RollupRetentionPolicy::KeepIndefinitely)),
                TimeLevel::new("hour", 3600, RollupConfig::default(), Some(RollupRetentionPolicy::KeepIndefinitely)),
                TimeLevel::new("day", 86400, RollupConfig::default(), Some(RollupRetentionPolicy::KeepIndefinitely)),
                TimeLevel::new("month", 2592000, RollupConfig::default(), Some(RollupRetentionPolicy::KeepIndefinitely)), // 30 days
                TimeLevel::new("year", 31536000, RollupConfig::default(), Some(RollupRetentionPolicy::KeepIndefinitely)), // 365 days
                TimeLevel::new("decade", 315360000, RollupConfig::default(), Some(RollupRetentionPolicy::KeepIndefinitely)), // 10 years
                TimeLevel::new("century", 3153600000, RollupConfig::default(), Some(RollupRetentionPolicy::KeepIndefinitely)), // 100 years
            ],
        }
    }
}

/// Configuration for roll-up operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollupConfig {
    /// Maximum number of leaves before forcing a roll-up
    pub max_leaves_per_page: usize,
    /// Maximum age of a page before it should be rolled up (in seconds)
    pub max_page_age_seconds: u64,
    /// Whether to force roll-up operations on shutdown
    pub force_rollup_on_shutdown: bool,
}

impl Default for RollupConfig {
    fn default() -> Self {
        Self {
            max_leaves_per_page: 1000,
            max_page_age_seconds: 3600, // 1 hour
            force_rollup_on_shutdown: true,
        }
    }
}
