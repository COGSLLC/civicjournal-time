// src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CJError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error (JSON): {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Hashing error: {0}")]
    HashingError(String),

    #[error("Merkle tree error: {0}")]
    MerkleTreeError(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Time hierarchy error: {0}")]
    TimeHierarchyError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Operation not allowed: {0}")]
    NotAllowed(String),

    #[error("An unknown error occurred: {0}")]
    Unknown(String),
}

// You might want to implement conversions from other specific error types as well
// For example, if a crypto library has its own error type:
// #[error("Cryptography error: {0}")]
// CryptoError(#[from] some_crypto_lib::Error),
