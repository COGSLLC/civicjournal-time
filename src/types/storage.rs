use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported storage backends
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageType {
    /// In-memory storage (for testing)
    Memory,
    /// File-based storage (persistent)
    File,
}

impl Default for StorageType {
    fn default() -> Self {
        Self::File
    }
}

impl fmt::Display for StorageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory => write!(f, "memory"),
            Self::File => write!(f, "file"),
        }
    }
}

impl StorageType {
    /// Parse a string into a StorageType
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "memory" => Some(Self::Memory),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}
