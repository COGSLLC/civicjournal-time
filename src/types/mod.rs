//! Core type definitions for CivicJournal Time

mod compression;
mod error;
mod log_level;
mod storage;
/// Contains types related to time hierarchies, levels, and configurations.
pub mod time;

pub use compression::CompressionAlgorithm;
pub use error::ConfigError;
pub use log_level::LogLevel;
pub use storage::StorageType;
pub use time::*;
