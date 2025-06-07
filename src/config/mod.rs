//! Configuration management for CivicJournal Time
//!
//! This module handles loading, validating, and providing access to the
//! application configuration. It supports loading configuration from files,
//! environment variables, and programmatic overrides.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod error;
pub mod validation;

#[cfg(test)]
#[path = "tests/validation_tests.rs"]
mod validation_tests;

#[cfg(test)]
#[path = "tests/config_mod_tests.rs"]
mod config_mod_tests;

// Publicly re-export key configuration types from the types module
pub use crate::types::time::{LevelRollupConfig, TimeHierarchyConfig, TimeLevel};

use std::{
    env,
    fs,
    path::{Path, PathBuf},
};
use directories::ProjectDirs;

use serde::{Deserialize, Serialize};

use crate::{
    CompressionAlgorithm,
    LogLevel,
    StorageType,
    // LevelRollupConfig, // Removed, publicly re-exported above
    // TimeHierarchyConfig, // Removed, publicly re-exported above
    // TimeLevel, // Removed, publicly re-exported above
};

/// Re-export the error type
pub use error::ConfigError;

/// The environment variable prefix for configuration overrides
const ENV_PREFIX: &str = "CJ_";

/// The application name used for finding config directories
const APP_NAME: &str = "civicjournal-time";

/// Main configuration structure for the CivicJournal Time system.
///
/// This struct holds all configuration options for the application.
/// It can be loaded from a TOML file, environment variables, or created programmatically.
///
/// # Example
///
/// ```no_run
/// use civicjournal_time::config::Config;
///
/// // If a provided path is not found, it falls back to defaults.
/// // Note: The load function expects a path. To trigger default loading logic
/// // when no specific file is intended, provide a path that's unlikely to exist.
/// let config = Config::load("path/that/hopefully/does/not/exist.toml").unwrap();
///
/// // Or specify a custom path (this file also likely doesn't exist in test context)
/// let config = Config::load("path/to/my_specific_config.toml").unwrap();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Time hierarchy configuration
    pub time_hierarchy: TimeHierarchyConfig,
    
    // The 'rollup' field (RollupConfig) has been removed.
    // Rollup settings are now per-level in TimeHierarchyConfig.levels[N].rollup_config (LevelRollupConfig).
    // The global 'force_rollup_on_shutdown' is now a direct field in Config.
    
    /// Storage configuration
    pub storage: StorageConfig,
    
    /// Compression configuration
    pub compression: CompressionConfig,
    
    /// Logging configuration
    pub logging: LoggingConfig,
    
    /// Metrics configuration
    pub metrics: MetricsConfig,
    
    /// Retention policy configuration
    pub retention: RetentionConfig,

    /// Snapshot system configuration
    pub snapshot: SnapshotConfig,

    /// Whether to force roll-up operations on system shutdown.
    pub force_rollup_on_shutdown: bool,
}

/// Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Storage type
    #[serde(rename = "type")]
    pub storage_type: StorageType,
    /// Base path for file storage (ignored for memory storage)
    pub base_path: String,
    /// Maximum number of open files
    pub max_open_files: u32,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            storage_type: StorageType::File,
            base_path: "./data".to_string(),
            max_open_files: 100,
        }
    }
}

/// Compression configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    /// Whether compression is enabled
    pub enabled: bool,
    /// Compression algorithm to use
    pub algorithm: CompressionAlgorithm,
    /// Compression level (1-22 for zstd, 0-16 for lz4)
    pub level: u8,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            algorithm: CompressionAlgorithm::Zstd,
            level: 3,
        }
    }
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level
    pub level: LogLevel,
    /// Whether to log to console
    pub console: bool,
    /// Whether to log to file
    pub file: bool,
    /// Path to log file (if file logging is enabled)
    pub file_path: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            console: true,
            file: false,
            file_path: "./civicjournal.log".to_string(),
        }
    }
}

/// Metrics configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Whether metrics collection is enabled
    pub enabled: bool,
    /// Metrics push interval in seconds
    pub push_interval_seconds: u64,
    /// Metrics endpoint (if using push-based metrics)
    pub endpoint: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            push_interval_seconds: 60,
            endpoint: "".to_string(),
        }
    }
}

/// Retention policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    /// Whether retention policy is enabled
    pub enabled: bool,
    /// Retention period in seconds (0 = keep forever)
    pub period_seconds: u64,
    /// How often to run cleanup in seconds
    pub cleanup_interval_seconds: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            period_seconds: 0, // Keep forever by default
            cleanup_interval_seconds: 3600, // 1 hour
        }
    }
}

/// Configuration for snapshot retention policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRetentionConfig {
    /// Whether snapshot retention policies are active.
    pub enabled: bool,
    /// Maximum number of snapshots to retain. Oldest snapshots beyond this count will be candidates for pruning.
    /// `None` means no limit by count.
    pub max_count: Option<u32>,
    /// Maximum age of a snapshot in seconds. Snapshots older than this will be candidates for pruning.
    /// `None` means no limit by age.
    pub max_age_seconds: Option<u64>,
}

impl Default for SnapshotRetentionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_count: None,
            max_age_seconds: None,
        }
    }
}

/// Configuration for automatic snapshot creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotAutomaticCreationConfig {
    /// Whether automatic snapshot creation is enabled.
    pub enabled: bool,
    /// Cron-style schedule string for periodic snapshot creation (e.g., "0 0 * * * *" for daily at midnight).
    /// If set, this takes precedence over `interval_seconds`.
    pub cron_schedule: Option<String>,
    /// Interval in seconds for periodic snapshot creation.
    /// Used if `cron_schedule` is not set. `None` or 0 means no interval-based creation.
    pub interval_seconds: Option<u64>,
}

impl Default for SnapshotAutomaticCreationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cron_schedule: None,
            interval_seconds: None,
        }
    }
}

/// Configuration for the snapshot system.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotConfig {
    /// Globally enable or disable the snapshot feature.
    pub enabled: bool,
    /// The dedicated hierarchical level for storing snapshot pages.
    /// This level should be distinct from regular journal and rollup levels.
    /// Typically, a high value like 250-255 is used.
    pub dedicated_level: u8,
    /// Configuration for snapshot retention policies.
    pub retention: SnapshotRetentionConfig,
    /// Configuration for automatic snapshot creation.
    pub automatic_creation: SnapshotAutomaticCreationConfig,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dedicated_level: 250, // Default dedicated level for snapshots
            retention: SnapshotRetentionConfig::default(),
            automatic_creation: SnapshotAutomaticCreationConfig::default(),
        }
    }
}

impl Default for Config {
    /// Creates a default configuration with sensible defaults.
    ///
    /// This is used when no configuration file is found or when the configuration
    /// file cannot be parsed.
    fn default() -> Self {
        Config {
            time_hierarchy: TimeHierarchyConfig {
                levels: vec![
                    TimeLevel::new("raw", 1, LevelRollupConfig::default(), None), // Smallest duration, pages primarily finalize by count/age
                    TimeLevel { name: "minute".to_string(), duration_seconds: 60, rollup_config: LevelRollupConfig::default(), retention_policy: None },
                    TimeLevel { name: "hour".to_string(), duration_seconds: 3600, rollup_config: LevelRollupConfig::default(), retention_policy: None },
                    TimeLevel { name: "day".to_string(), duration_seconds: 86400, rollup_config: LevelRollupConfig::default(), retention_policy: None },
                ]
            },
            storage: StorageConfig::default(),
            compression: CompressionConfig::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
            retention: RetentionConfig::default(),
            snapshot: SnapshotConfig::default(),
            force_rollup_on_shutdown: true,
        }
    }
}

impl Config {
    /// Loads the configuration from the specified path or the default location.
    ///
    /// The configuration is loaded in the following order:
    /// 1. From the specified file path (if provided)
    /// 2. From `./config.toml` in the current directory
    /// 3. From the OS-specific config directory
    /// 4. From environment variables with the `CJ_` prefix
    /// 5. From built-in defaults
    ///
    /// # Arguments
    ///
    /// * `path` - An optional path to a configuration file. If `None`, the default
    ///   locations will be searched.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration file exists but cannot be read or parsed.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        
        // Try to load from the specified path
        match fs::read_to_string(path) {
            Ok(config_str) => {
                let mut config: Config = toml::from_str(&config_str).map_err(|e| {
                    ConfigError::validation_error(format!("Failed to parse config file: {}", e))
                })?;
                
                // Apply environment variable overrides
                config.apply_env_vars()?;
                
                // Validate the configuration
                config.validate()?;
                
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::warn!("Config file not found at {}, using defaults", path.display());
                let mut config = Self::default();
                config.apply_env_vars()?;
                Ok(config)
            }
            Err(e) => {
                Err(ConfigError::file_not_found(
                    format!("Failed to read config file {}: {}", path.display(), e)
                ))
            }
        }
    }

    /// Applies environment variable overrides to the configuration.
    ///
    /// Environment variables should be prefixed with `CJ_` and use `_` as a separator.
    /// For example, to set the log level, use `CJ_LOGGING_LEVEL=debug`.
    ///
    /// # Errors
    ///
    /// Returns an error if any environment variable cannot be parsed.
    pub fn apply_env_vars(&mut self) -> Result<(), ConfigError> {
        for (key, value) in env::vars() {
            if let Some(stripped) = key.strip_prefix(ENV_PREFIX) {
                // Skip empty values
                if value.trim().is_empty() {
                    continue;
                }
                
                // Handle environment variable overrides
                match stripped.to_lowercase().as_str() {
                    "logging_level" => {
                        self.logging.level = value.parse().map_err(|_| {
                            ConfigError::invalid_value("logging.level", &value, "Invalid log level")
                        })?;
                    },
                    // Add more environment variable handlers as needed
                    _ => {}
                }
            }
        }
        
        Ok(())
    }

    /// Validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        match validation::validate_config(self) {
            Ok(()) => Ok(()),
            Err(cj_error) => match cj_error {
                crate::error::CJError::ConfigError(config_error) => Err(config_error),
                _ => Err(ConfigError::Other(cj_error.to_string())),
            },
        }
    }

    /// Returns the path to the directory where configuration files should be stored.
    ///
    /// This is OS-specific:
    /// - Linux: `$HOME/.config/civicjournal-time`
    /// - macOS: `$HOME/Library/Application Support/com.civicjournal.time`
    /// - Windows: `%APPDATA%\\CivicJournal\\civicjournal-time`
    pub fn config_dir() -> Option<PathBuf> {
        ProjectDirs::from("com", "civicjournal", APP_NAME)
            .map(|proj_dirs| proj_dirs.config_dir().to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.time_hierarchy.levels.len(), 4);
        assert!(config.time_hierarchy.levels[0].rollup_config.max_items_per_page > 0);
        assert!(config.time_hierarchy.levels[0].rollup_config.max_page_age_seconds > 0);
    }
}
