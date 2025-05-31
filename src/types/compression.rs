use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported compression algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        Self::Zstd
    }
}

impl fmt::Display for CompressionAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zstd => write!(f, "zstd"),
            Self::Lz4 => write!(f, "lz4"),
            Self::Snappy => write!(f, "snappy"),
            Self::None => write!(f, "none"),
        }
    }
}
