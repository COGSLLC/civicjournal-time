use std::fmt;
use thiserror::Error;

/// Errors that can occur during configuration
#[derive(Debug, Error)]
pub enum ConfigError {
    /// I/O error occurred
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    
    /// TOML parsing error
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    
    /// Validation error
    #[error("Validation error: {0}")]
    Validation(String),
    
    /// Invalid configuration value
    #[error("Invalid value for {field}: {value} - {reason}")]
    InvalidValue {
        /// The field that had an invalid value
        field: String,
        /// The invalid value
        value: String,
        /// Reason the value is invalid
        reason: String,
    },
}

impl ConfigError {
    /// Create a new invalid value error
    pub fn invalid_value(field: impl Into<String>, value: impl fmt::Display, reason: impl Into<String>) -> Self {
        Self::InvalidValue {
            field: field.into(),
            value: value.to_string(),
            reason: reason.into(),
        }
    }
}
