//! # CivicJournal Time
//!
//! A hierarchical time-series journal system using merkle-chains and pages for efficient historical state management.

#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]

use std::sync::OnceLock;

/// Public API for interacting with the CivicJournal system.
pub mod api;
pub mod config;
pub mod query;
/// Core data structures and logic for journal pages, leaves, and Merkle trees.
pub mod core;
pub mod error;
/// Foreign Function Interface (FFI) for exposing CivicJournal functionality to other languages.
pub mod ffi;
/// Turnstile workflow implementation
pub mod turnstile;
/// Storage backends for persisting journal data.
pub mod storage;
/// Time-related utilities, including hierarchy levels and time units.
pub mod time;
pub mod types;
#[cfg(feature = "demo")]
/// Command line scaffolding for Demo Mode.
pub mod demo_cli;
/// Utilities for testing purposes only, compiled with `#[cfg(test)]`.
pub mod test_utils;

// Re-export commonly used types
pub use config::Config;
pub use error::{CJError, Result as CJResult};
pub use types::{
    CompressionAlgorithm,
    LogLevel,
    StorageType,
    LevelRollupConfig, TimeHierarchyConfig, TimeLevel,
};

/// Obtain the built-in default configuration.
///
/// This is handy for quick experiments or generating a template
/// configuration file. The same values are used when no `config.toml`
/// is found during [`init`].
pub fn default_config() -> Config {
    Config::default()
}

/// Global configuration instance
static CONFIG: OnceLock<Config> = OnceLock::new();

/// Initialize the global configuration and logger
///
/// This function should be called early in your application's main function.
/// It loads the configuration from the specified path or uses the default configuration
/// if the file doesn't exist or can't be parsed.
///
/// # Arguments
///
/// * `config_path` - Path to the configuration file. If `None`, looks for "config.toml" in the current directory.
///
/// # Errors
///
/// Returns an error if the configuration file exists but can't be parsed, or if logging
/// initialization fails.
pub fn init(config_path: Option<&str>) -> CJResult<&'static Config> {
    // Load configuration without logging initialized yet
    let config_path = config_path.unwrap_or("config.toml");
    let config = match Config::load(config_path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to load config from {}: {}", config_path, e);
            eprintln!("Using default configuration");
            let mut config = Config::default();
            // Apply environment variables to the default config
            if let Err(e) = config.apply_env_vars() {
                eprintln!("Failed to apply environment variables: {}", e);
            }
            config
        }
    };

    // Initialize logging with the loaded configuration
    init_logging(Some(&config.logging.level))?;

    // Store the configuration in the global state
    let config_ref = CONFIG.get_or_init(|| config);
    
    log::info!("CivicJournal Time initialized with config from {}", config_path);
    log::debug!("Configuration: {:#?}", config_ref);
    
    Ok(config_ref)
}

/// Get a reference to the global configuration
///
/// # Panics
///
/// Panics if the configuration has not been initialized by calling `init()`.
pub fn config() -> &'static Config {
    CONFIG.get().expect("Configuration not initialized. Call init() first.")
}

/// Initialize the logging system
#[cfg(feature = "logging")]
fn init_logging(config: Option<&crate::types::LogLevel>) -> CJResult<()> {
    use log::LevelFilter;
    use std::str::FromStr;
    
    // Determine the log level from the config or environment
    let log_level = match config {
        Some(cfg) => cfg.to_string().to_lowercase(),
        None => std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
    };
    
    // Parse the log level
    let log_filter = LevelFilter::from_str(&log_level).unwrap_or(LevelFilter::Info);
    
    // Configure the logger
    let mut builder = env_logger::Builder::new();
    
    // Set the log level
    builder.filter_level(log_filter);
    
    // Configure format
    builder.format_timestamp(Some(env_logger::TimestampPrecision::Millis));
    builder.format_module_path(true);
    builder.format_target(false);
    
    // Initialize the logger
    builder.try_init().map_err(|e| {
        CJError::new(format!("Failed to initialize logger: {}", e))
    })?;
    
    Ok(())
}

/// A simple function to confirm the library is linked.
pub fn hello() -> &'static str {
    "Hello from CivicJournal-Time!"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hello() {
        assert_eq!(hello(), "Hello from CivicJournal-Time!");
    }
}
