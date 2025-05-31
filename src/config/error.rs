//! Error types for configuration handling
//!
//! This module defines error types related to configuration loading, parsing,
//! and validation. The main error type is `ConfigError`, which is designed to
//! provide detailed error information for debugging and user feedback.

use std::io;
use thiserror::Error;

/// Errors that can occur during configuration loading, parsing, and validation.
///
/// This error type is designed to provide detailed information about what went
/// wrong during configuration processing, making it easier to diagnose and fix
/// configuration issues.
#[derive(Error, Debug)]
pub enum ConfigError {
    /// I/O error occurred while reading the configuration file.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Error parsing the configuration file (invalid TOML).
    #[error("Failed to parse configuration: {0}")]
    ParseError(#[from] toml::de::Error),
    
    /// Error serializing the configuration.
    #[error("Failed to serialize configuration: {0}")]
    SerializeError(#[from] toml::ser::Error),

    /// Configuration validation failed.
    #[error("Configuration validation failed: {0}")]
    ValidationError(String),

    /// Missing required configuration value.
    #[error("Missing required configuration: {0}")]
    MissingValue(String),

    
    /// Invalid configuration value.
    #[error("Invalid value for '{field}': '{value}'. {reason}")]
    InvalidValue {
        /// The name of the field that has an invalid value.
        field: String,
        /// The invalid value.
        value: String,
        /// The reason the value is invalid.
        reason: String,
    },
    
    /// Configuration file not found.
    #[error("Configuration file not found: {0}")]
    FileNotFound(String),
    
    /// Environment variable error.
    #[error("Environment variable error: {0}")]
    EnvVarError(#[from] std::env::VarError),
    
    /// Other configuration error.
    #[error("Configuration error: {0}")]
    Other(String),
}

impl ConfigError {
    /// Creates a new invalid value error.
    pub fn invalid_value<S1, S2, S3>(field: S1, value: S2, reason: S3) -> Self
    where
        S1: Into<String>,
        S2: std::fmt::Display,
        S3: Into<String>,
    {
        ConfigError::InvalidValue {
            field: field.into(),
            value: value.to_string(),
            reason: reason.into(),
        }
    }
    
    /// Creates a new missing value error.
    pub fn missing_value<S: Into<String>>(field: S) -> Self {
        ConfigError::MissingValue(field.into())
    }
    
    /// Creates a new validation error.
    pub fn validation_error<S: Into<String>>(message: S) -> Self {
        ConfigError::ValidationError(message.into())
    }
    
    /// Creates a new file not found error.
    pub fn file_not_found<S: Into<String>>(path: S) -> Self {
        ConfigError::FileNotFound(path.into())
    }
}


// Implement FromStr for ConfigError to support parsing from strings
impl std::str::FromStr for ConfigError {
    type Err = Self;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ConfigError::Other(s.to_string()))
    }
}
