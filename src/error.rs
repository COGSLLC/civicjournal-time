//! Error types for the CivicJournal Time system
//!
//! This module defines the error types used throughout the CivicJournal Time system.
//! The main error type is `CJError`, which can represent various error conditions
//! that might occur during the operation of the system.

use thiserror::Error;

/// Main error type for the CivicJournal Time system
#[derive(Error, Debug)]
pub enum CJError {
    /// I/O operation failed
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error
    #[error("Serialization error (JSON): {0}")]
    SerdeJson(#[from] serde_json::Error),
    
    /// TOML serialization/deserialization error
    #[error("Configuration error (TOML): {0}")]
    Toml(#[from] toml::de::Error),
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(#[from] crate::config::ConfigError),

    /// Hashing operation failed
    #[error("Hashing error: {0}")]
    HashingError(String),
    
    /// Cryptographic operation failed
    #[cfg(feature = "crypto")]
    #[error("Crypto error: {0}")]
    CryptoError(#[from] ring::error::Unspecified),

    /// Merkle tree operation failed
    #[error("Merkle tree error: {0}")]
    MerkleTreeError(String),
    
    /// Storage operation failed
    #[error("Storage error: {0}")]
    StorageError(String),

    /// Compression operation failed
    #[error("Compression error: {0}")]
    CompressionError(String),

    /// Decompression operation failed
    #[error("Decompression error: {0}")]
    DecompressionError(String),

    /// Invalid file format encountered (e.g., during header parsing)
    #[error("Invalid file format: {0}")]
    InvalidFileFormat(String),
    
    /// Time hierarchy operation failed
    #[error("Time hierarchy error: {0}")]
    TimeHierarchyError(String),
    
    /// Invalid input provided
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    
    /// Requested resource not found
    #[error("Not found: {0}")]
    NotFound(String),

    /// Requested page not found in storage
    #[error("Page L{level}P{page_id} not found")]
    PageNotFound {
        /// The level of the page that was not found.
        level: u8,
        /// The ID of the page that was not found.
        page_id: u64
    },
    
    /// Operation not supported
    #[error("Operation not supported: {0}")]
    NotSupported(String),
    
    /// Operation timed out
    #[error("Operation timed out: {0}")]
    Timeout(String),
    
    /// Resource is already in use
    #[error("Resource already in use: {0}")]
    AlreadyInUse(String),
    
    /// Resource is not initialized
    #[error("Resource not initialized: {0}")]
    NotInitialized(String),
    
    /// Operation not permitted
    #[error("Operation not allowed: {0}")]
    NotAllowed(String),
    
    /// Unknown or unexpected error
    #[error("An unknown error occurred: {0}")]
    Unknown(String),
}

/// Result type alias for operations that can fail with a [CJError]
pub type Result<T> = std::result::Result<T, CJError>;

impl CJError {
    /// Create a new error with a string message
    pub fn new<S: Into<String>>(msg: S) -> Self {
        CJError::InvalidInput(msg.into())
    }

    /// Create a new invalid input error
    pub fn invalid_input<S: Into<String>>(msg: S) -> Self {
        CJError::InvalidInput(msg.into())
    }

    /// Create a new not found error
    pub fn not_found<S: Into<String>>(what: S) -> Self {
        CJError::NotFound(what.into())
    }
    
    /// Create a new not supported error
    pub fn not_supported<S: Into<String>>(what: S) -> Self {
        CJError::NotSupported(what.into())
    }
    
    /// Create a new timeout error
    pub fn timeout<S: Into<String>>(what: S) -> Self {
        CJError::Timeout(what.into())
    }
    
    /// Create a new already in use error
    pub fn already_in_use<S: Into<String>>(what: S) -> Self {
        CJError::AlreadyInUse(what.into())
    }
    
    /// Create a new not initialized error
    pub fn not_initialized<S: Into<String>>(what: S) -> Self {
        CJError::NotInitialized(what.into())
    }
}

// Implement From for common error types
impl From<&str> for CJError {
    fn from(s: &str) -> Self {
        CJError::new(s)
    }
}

impl From<String> for CJError {
    fn from(s: String) -> Self {
        CJError::new(s)
    }
}

impl From<crate::query::types::QueryError> for CJError {
    fn from(e: crate::query::types::QueryError) -> Self {
        CJError::new(e.to_string())
    }
}

// Implement From for other common error types
impl From<Box<dyn std::error::Error + Send + Sync>> for CJError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        CJError::new(err.to_string())
    }
}

impl From<std::num::TryFromIntError> for CJError {
    fn from(err: std::num::TryFromIntError) -> Self {
        CJError::new(format!("Integer conversion error: {}", err))
    }
}

impl From<std::string::FromUtf8Error> for CJError {
    fn from(err: std::string::FromUtf8Error) -> Self {
        CJError::new(format!("UTF-8 conversion error: {}", err))
    }
}

#[cfg(feature = "async")]
impl From<tokio::task::JoinError> for CJError {
    fn from(err: tokio::task::JoinError) -> Self {
        CJError::new(format!("Async task error: {}", err))
    }
}

