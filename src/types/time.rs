use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for a single time hierarchy level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeLevel {
    /// Name of the time level (e.g., "minute", "hour")
    pub name: String,
    /// Duration of this time level in seconds
    pub duration_seconds: u64,
}

impl TimeLevel {
    /// Create a new time level
    pub fn new(name: impl Into<String>, duration_seconds: u64) -> Self {
        Self {
            name: name.into(),
            duration_seconds,
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
                TimeLevel::new("minute", 60),
                TimeLevel::new("hour", 3600),
                TimeLevel::new("day", 86400),
                TimeLevel::new("month", 2592000), // 30 days
                TimeLevel::new("year", 31536000), // 365 days
                TimeLevel::new("decade", 315360000), // 10 years
                TimeLevel::new("century", 3153600000), // 100 years
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
