// src/time/mod.rs

/// Defines the `TimeHierarchy` structure for managing time-based page aggregation.
pub mod hierarchy;
/// Defines the `TimeLevel` enum, representing different granularities of time (Minute, Hour, Day, etc.).
pub mod level;
/// Defines time units and their relationships within the time hierarchy.
pub mod unit;

// Re-export key structures or functions
// pub use hierarchy::TimeHierarchy;
pub use level::TimeLevel;
