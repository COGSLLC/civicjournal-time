//! Type definitions for configuration

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use strum::{Display, EnumString, VariantNames};

/// Supported storage types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
pub enum StorageType {
    /// In-memory storage (not persistent)
    Memory,
    /// File-based storage (persistent)
    File,
}

impl Default for StorageType {
    fn default() -> Self {
        StorageType::File
    }
}

/// Supported compression algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
pub enum CompressionAlgorithm {
    /// Zstandard compression
    Zstd,
    /// LZ4 compression
    Lz4,
    /// Snappy compression
    Snappy,
    /// No compression
    None,
}

impl Default for CompressionAlgorithm {
    fn default() -> Self {
        CompressionAlgorithm::Zstd
    }
}

/// Log level configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, Serialize, Deserialize)]
#[strum(serialize_all = "lowercase")]
pub enum LogLevel {
    /// Trace level (most verbose)
    Trace,
    /// Debug level
    Debug,
    /// Info level
    Info,
    /// Warning level
    Warn,
    /// Error level (least verbose)
    Error,
}

impl Default for LogLevel {
    fn default() -> Self {
        LogLevel::Info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_storage_type_parsing() {
        assert_eq!(
            StorageType::from_str("memory").unwrap(),
            StorageType::Memory
        );
        assert_eq!(StorageType::from_str("file").unwrap(), StorageType::File);
        assert!(StorageType::from_str("invalid").is_err());
    }

    #[test]
    fn test_compression_algorithm_parsing() {
        assert_eq!(
            CompressionAlgorithm::from_str("zstd").unwrap(),
            CompressionAlgorithm::Zstd
        );
        assert_eq!(
            CompressionAlgorithm::from_str("lz4").unwrap(),
            CompressionAlgorithm::Lz4
        );
        assert_eq!(
            CompressionAlgorithm::from_str("snappy").unwrap(),
            CompressionAlgorithm::Snappy
        );
        assert_eq!(
            CompressionAlgorithm::from_str("none").unwrap(),
            CompressionAlgorithm::None
        );
        assert!(CompressionAlgorithm::from_str("invalid").is_err());
    }

    #[test]
    fn test_log_level_parsing() {
        assert_eq!(LogLevel::from_str("trace").unwrap(), LogLevel::Trace);
        assert_eq!(LogLevel::from_str("debug").unwrap(), LogLevel::Debug);
        assert_eq!(LogLevel::from_str("info").unwrap(), LogLevel::Info);
        assert_eq!(LogLevel::from_str("warn").unwrap(), LogLevel::Warn);
        assert_eq!(LogLevel::from_str("error").unwrap(), LogLevel::Error);
        assert!(LogLevel::from_str("invalid").is_err());
    }
}
