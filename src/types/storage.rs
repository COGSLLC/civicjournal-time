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

use std::str::FromStr;

impl FromStr for StorageType {
    type Err = String;

    /// Parse a string into a StorageType
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "memory" => Ok(Self::Memory),
            "file" => Ok(Self::File),
            _ => Err(format!("Invalid storage type: '{}'", s)),
        }
    }
}
