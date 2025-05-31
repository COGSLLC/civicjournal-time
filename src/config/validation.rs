//! Configuration validation for CivicJournal Time
//!
//! This module contains functions for validating the application configuration
//! to ensure all values are within acceptable ranges and consistent with each other.

use std::path::Path;

use std::collections::HashSet;

use super::{
    Config,
    RollupConfig, TimeHierarchyConfig,
};
use crate::error::CJError;
use super::error::ConfigError; // For return types in validation functions
use crate::StorageType;      // For StorageType enum used in tests

/// Validates the application configuration.
///
/// This function performs comprehensive validation of the entire configuration,
/// checking for consistency between different sections and ensuring all values
/// are within acceptable ranges.
///
/// # Errors
///
/// Returns a `ConfigError` if any validation check fails.
pub fn validate_config(config: &Config) -> Result<(), CJError> {
    // Validate time hierarchy first as other validations depend on it
    validate_time_hierarchy(&config.time_hierarchy)?;
    
    // Validate rollup configuration against time hierarchy
    validate_rollup_config(&config.rollup, &config.time_hierarchy)?;
    
    // Validate storage configuration
    validate_storage_config(&config.storage)?;
    
    // Validate compression configuration
    validate_compression_config(&config.compression)?;
    
    // Validate logging configuration
    validate_logging_config(&config.logging)?;
    
    // Validate metrics configuration
    validate_metrics_config(&config.metrics)?;
    
    // Validate retention configuration against time hierarchy
    validate_retention_config(&config.retention, &config.time_hierarchy)?;
    
    // Cross-section validations
    validate_cross_section(&config)?;
    
    Ok(())
}

// Original simpler validate_time_hierarchy function removed (was lines 55-65)

/// Validates the rollup configuration.
fn validate_rollup_config(
    config: &RollupConfig,
    _time_hierarchy: &TimeHierarchyConfig, // time_hierarchy might be needed for future, more complex cross-validation
) -> Result<(), CJError> {
    if config.max_leaves_per_page == 0 {
        return Err(ConfigError::invalid_value(
            "rollup.max_leaves_per_page",
            config.max_leaves_per_page,
            "max_leaves_per_page must be greater than 0"
        ).into());
    }

    if config.max_page_age_seconds == 0 {
        return Err(ConfigError::invalid_value(
            "rollup.max_page_age_seconds",
            config.max_page_age_seconds,
            "max_page_age_seconds must be greater than 0"
        ).into());
    }
    
    // Further validation for rollup config can be added here if necessary.
    // For instance, ensuring max_page_age_seconds is reasonable in context of time hierarchy levels,
    // but that would require more specific rules.
    Ok(())
}

/// Validates the storage configuration.
fn validate_storage_config(config: &super::StorageConfig) -> Result<(), CJError> {
    // Validate base path for file storage
    if let StorageType::File = config.storage_type {
        // Check if base path is provided
        if config.base_path.is_empty() {
            return Err(ConfigError::invalid_value(
                "storage.base_path",
                "",
                "Base path cannot be empty for file storage"
            ).into());
        }
        
        let base_path = Path::new(&config.base_path);
        
        // Check if base path exists and is a directory
        match std::fs::metadata(base_path) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(ConfigError::invalid_value(
                        "storage.base_path",
                        base_path.display().to_string(),
                        "Base path must be a directory"
                    ).into());
                }
                
                // Check write permissions
                let test_file = base_path.join(".civicjournal_test");
                match std::fs::File::create(&test_file) {
                    Ok(_) => {
                        // Clean up test file
                        let _ = std::fs::remove_file(test_file);
                    }
                    Err(e) => {
                        return Err(ConfigError::invalid_value(
                            "storage.base_path",
                            base_path.display().to_string(),
                            format!("Cannot write to directory: {}", e)
                        ).into());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Directory doesn't exist, check if parent is writable
                if let Some(parent) = base_path.parent() {
                    if !parent.exists() {
                        return Err(ConfigError::invalid_value(
                            "storage.base_path",
                            base_path.display().to_string(),
                            "Parent directory does not exist"
                        ).into());
                    }
                    
                    // Check if we can create the directory
                    if let Err(e) = std::fs::create_dir_all(base_path) {
                        return Err(ConfigError::invalid_value(
                            "storage.base_path",
                            base_path.display().to_string(),
                            format!("Cannot create directory: {}", e)
                        ).into());
                    }
                    
                    // Clean up the test directory
                    let _ = std::fs::remove_dir(base_path);
                }
            }
            Err(e) => {
                return Err(ConfigError::invalid_value(
                    "storage.base_path",
                    base_path.display().to_string(),
                    format!("Error accessing path: {}", e)
                ).into());
            }
        }
    }
    
    // Validate max_open_files
    const MIN_OPEN_FILES: u32 = 10;  // Reasonable minimum for most systems
    const MAX_OPEN_FILES: u32 = 1024 * 1024;  // Reasonable maximum to prevent resource exhaustion
    
    if config.max_open_files < MIN_OPEN_FILES {
        return Err(ConfigError::invalid_value(
            "storage.max_open_files",
            config.max_open_files,
            format!("Maximum number of open files must be at least {}", MIN_OPEN_FILES)
        ).into());
    }
    
    if config.max_open_files > MAX_OPEN_FILES {
        log::warn!(
            "High value for max_open_files ({}). This may consume significant system resources.",
            config.max_open_files
        );
    }
    
    Ok(())
}

/// Validates the compression configuration.
fn validate_compression_config(config: &super::CompressionConfig) -> Result<(), CJError> {
    // If compression is disabled, no need to validate further
    if !config.enabled {
        return Ok(());
    }
    
    // Check compression level is within valid range for the selected algorithm
    match config.algorithm {
        super::CompressionAlgorithm::Zstd => {
            const ZSTD_MIN_LEVEL: u8 = 1;
            const ZSTD_MAX_LEVEL: u8 = 22;
            
            if config.level < ZSTD_MIN_LEVEL || config.level > ZSTD_MAX_LEVEL {
                return Err(ConfigError::invalid_value(
                    "compression.level",
                    config.level,
                    format!("Zstd compression level must be between {} and {}", 
                           ZSTD_MIN_LEVEL, ZSTD_MAX_LEVEL)
                ).into());
            }
            
            // Recommend levels 1-4 for fast compression, 5-12 for balanced, 13-22 for high compression
            if config.level >= 15 {
                log::warn!(
                    "Using high Zstd compression level ({}). This may impact performance.",
                    config.level
                );
            }
        }
        super::CompressionAlgorithm::Lz4 => {
            const LZ4_MIN_LEVEL: u8 = 1;
            const LZ4_MAX_LEVEL: u8 = 12;
            
            if config.level < LZ4_MIN_LEVEL || config.level > LZ4_MAX_LEVEL {
                return Err(ConfigError::invalid_value(
                    "compression.level",
                    config.level,
                    format!("LZ4 compression level must be between {} and {}", 
                           LZ4_MIN_LEVEL, LZ4_MAX_LEVEL)
                ).into());
            }
            
            // LZ4 levels above 9 use a different algorithm that's much slower
            if config.level >= 10 {
                log::info!(
                    "Using high LZ4 compression level ({}). Consider using Zstd for better compression ratios.",
                    config.level
                );
            }
        }
        super::CompressionAlgorithm::Snappy => {
            // Snappy doesn't use compression levels, so no validation needed
            if config.level != 0 {
                log::warn!(
                    "Compression level {} is ignored for Snappy compression",
                    config.level
                );
            }
        }
        super::CompressionAlgorithm::None => {
            // No validation needed for no compression
            if config.level != 0 {
                log::warn!(
                    "Compression level {} is ignored when compression is disabled",
                    config.level
                );
            }
        }
    }
    
    Ok(())
}

/// Validates the logging configuration.
fn validate_logging_config(config: &super::LoggingConfig) -> Result<(), CJError> {
    // If both console and file logging are disabled, that's fine - we'll just log to the void
    if !config.console && !config.file {
        log::warn!("Both console and file logging are disabled. No logs will be captured.");
        return Ok(());
    }
    
    // If file logging is enabled, validate the log file path
    if config.file {
        if config.file_path.is_empty() {
            return Err(ConfigError::missing_value("logging.file_path").into());
        }
        
        // Check if the log file path is absolute
        let log_path = Path::new(&config.file_path);
        let abs_log_path = if log_path.is_absolute() {
            log_path.to_path_buf()
        } else {
            // If relative, make it absolute relative to the current directory
            let current_dir = std::env::current_dir()
                .map_err(ConfigError::Io)?;
            current_dir.join(log_path)
        };
        
        // Check if the parent directory exists and is writable
        if let Some(parent) = abs_log_path.parent() {
            if !parent.exists() {
                return Err(ConfigError::invalid_value(
                    "logging.file_path",
                    &config.file_path,
                    "Parent directory does not exist"
                ).into());
            }
            
            // Check if the parent directory is writable
            let test_file = parent.join(".civicjournal_test_write");
            if let Err(e) = std::fs::File::create(&test_file) {
                return Err(ConfigError::invalid_value(
                    "logging.file_path",
                    &config.file_path,
                    format!("Cannot write to log directory: {}", e)
                ).into());
            }
            
            // Clean up test file
            let _ = std::fs::remove_file(test_file);
        }
        
        // Check if we can open the file for appending
        if let Err(e) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(&abs_log_path)
        {
            return Err(ConfigError::invalid_value(
                "logging.file_path",
                &config.file_path,
                format!("Cannot open log file for writing: {}", e)
            ).into());
        }
        
        // Warn about potential disk usage since we don't have built-in rotation
        log::warn!(
            "File logging is enabled at '{}' but log rotation is not built-in. Consider using an external log rotation solution to prevent excessive disk usage.",
            abs_log_path.display()
        );
    }
    
    Ok(())
}

/// Validates the metrics configuration.
fn validate_metrics_config(config: &super::MetricsConfig) -> Result<(), CJError> {
    // If metrics are disabled, no need to validate further
    if !config.enabled {
        return Ok(());
    }
    
    // Validate push interval
    if config.push_interval_seconds == 0 {
        return Err(ConfigError::validation_error(
            "Metrics push interval must be greater than 0"
        ).into());
    }
    
    // If an endpoint is provided, it should be a valid URL
    if !config.endpoint.is_empty() {
        if let Err(e) = url::Url::parse(&config.endpoint) {
            return Err(ConfigError::invalid_value(
                "metrics.endpoint",
                &config.endpoint,
                format!("Invalid URL format: {}", e)
            ).into());
        }
    }
    
    Ok(())
}

/// Validates the retention configuration.
fn validate_retention_config(
    config: &super::RetentionConfig,
    _time_hierarchy: &super::TimeHierarchyConfig,
) -> Result<(), CJError> {
    // If retention is disabled, no need to validate further
    if !config.enabled {
        return Ok(());
    }
    
    // Check cleanup interval
    if config.cleanup_interval_seconds == 0 {
        return Err(ConfigError::validation_error(
            "Cleanup interval must be greater than 0"
        ).into());
    }
    
    // If period_seconds is 0, it means retain forever, which is valid
    if config.period_seconds > 0 {
        // If a retention period is set, ensure it's at least as long as the cleanup interval
        if config.period_seconds < config.cleanup_interval_seconds {
            return Err(ConfigError::validation_error(
                "Retention period must be greater than or equal to cleanup interval"
            ).into());
        }
    }
    
    Ok(())
}

/// Performs cross-section validation of the configuration.
fn validate_cross_section(config: &Config) -> Result<(), CJError> {
    // Example: If using file storage with compression, ensure the compression library is available
    if let StorageType::File = config.storage.storage_type {
        match config.compression.algorithm {
            super::CompressionAlgorithm::Zstd => {
                #[cfg(not(feature = "zstd-compression"))]
                return Err(ConfigError::validation_error(
                    "Zstd compression requires the 'zstd' feature to be enabled"
                ).into());
            }
            super::CompressionAlgorithm::Lz4 => {
                #[cfg(not(feature = "lz4-compression"))]
                return Err(ConfigError::validation_error(
                    "LZ4 compression requires the 'lz4' feature to be enabled"
                ).into());
            }
            super::CompressionAlgorithm::Snappy => {
                #[cfg(not(feature = "snappy-compression"))]
                return Err(ConfigError::validation_error(
                    "Snappy compression requires the 'snappy' feature to be enabled"
                ).into());
            }
            super::CompressionAlgorithm::None => {}
        }
    }
    
    // Add more cross-section validations as needed
    
    Ok(())
}

/// Validate time hierarchy configuration
fn validate_time_hierarchy(config: &TimeHierarchyConfig) -> Result<(), CJError> {
    if config.levels.is_empty() {
        return Err(ConfigError::validation_error("Time hierarchy must have at least one level").into());
    }

    // Check for duplicate level names
    let mut names = HashSet::new();
    for level in &config.levels {
        if !names.insert(&level.name) {
            return Err(ConfigError::validation_error(format!("Duplicate time level name: {}", level.name)).into());
        }
        if level.duration_seconds == 0 {
            return Err(ConfigError::validation_error(format!("Duration for level {} must be greater than 0", level.name)).into());
        }
    }

    // Check that levels are in ascending order of duration
    for i in 1..config.levels.len() {
        if config.levels[i].duration_seconds <= config.levels[i - 1].duration_seconds {
            return Err(ConfigError::validation_error(format!(
                "Time level durations must be in ascending order. Found {} after {}",
                config.levels[i].duration_seconds,
                config.levels[i - 1].duration_seconds
            )).into());
        }
    }

    Ok(())
}




#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        CompressionConfig, RetentionConfig, StorageConfig,
    };
    use crate::types::{CompressionAlgorithm, TimeLevel}; // Import the TimeLevel STRUCT and CompressionAlgorithm

    #[test]
    fn test_validate_time_hierarchy() {
        // Valid hierarchy
        let valid = TimeHierarchyConfig {
            levels: vec![TimeLevel {
                    rollup_config: RollupConfig::default(),
                    retention_policy: None,
                name: "test".to_string(),
                duration_seconds: 60,
            }],
        };
        assert!(validate_time_hierarchy(&valid).is_ok());

        // Empty hierarchy
        let empty = TimeHierarchyConfig { levels: vec![] };
        assert!(validate_time_hierarchy(&empty).is_err());

        // Duplicate names
        let duplicate = TimeHierarchyConfig {
            levels: vec![
                TimeLevel {
                    rollup_config: RollupConfig::default(),
                    retention_policy: None,
                    name: "test".to_string(),
                    duration_seconds: 60,
                },
                TimeLevel {
                    rollup_config: RollupConfig::default(),
                    retention_policy: None,
                    name: "test".to_string(),
                    duration_seconds: 3600,
                },
            ],
        };
        assert!(validate_time_hierarchy(&duplicate).is_err());

        // Non-ascending durations
        let bad_order = TimeHierarchyConfig {
            levels: vec![
                TimeLevel {
                    rollup_config: RollupConfig::default(),
                    retention_policy: None,
                    name: "hour".to_string(),
                    duration_seconds: 3600,
                },
                TimeLevel {
                    rollup_config: RollupConfig::default(),
                    retention_policy: None,
                    name: "minute".to_string(),
                    duration_seconds: 60,
                },
            ],
        };
        assert!(validate_time_hierarchy(&bad_order).is_err());
    }

    #[test]
    fn test_validate_storage_config() { // Renamed test function for clarity
        // Valid storage
        let valid = StorageConfig { // Assuming StorageConfig is in parent module
            storage_type: StorageType::File, // Corrected field name and type
            base_path: "./test_data_validation".to_string(), // Use a test-specific path
            max_open_files: 100,
        };
        assert!(validate_storage_config(&valid).is_ok());
        // Clean up test directory
        let _ = std::fs::remove_dir_all("./test_data_validation");

        // Zero max open files
        let zero_files = StorageConfig {
            storage_type: StorageType::File, // Corrected field name and type
            base_path: "./test_data_validation_zero".to_string(),
            max_open_files: 0,
        };
        assert!(validate_storage_config(&zero_files).is_err());
        // Clean up test directory
        let _ = std::fs::remove_dir_all("./test_data_validation_zero");
    }

    #[test]
    fn test_validate_compression() {
        // Valid zstd
        let valid_zstd = CompressionConfig {
            enabled: true,
            algorithm: CompressionAlgorithm::Zstd,
            level: 3,
        };
        assert!(validate_compression_config(&valid_zstd).is_ok());

        // Invalid zstd level
        let bad_zstd = CompressionConfig {
            enabled: true,
            algorithm: CompressionAlgorithm::Zstd,
            level: 30,
        };
        assert!(validate_compression_config(&bad_zstd).is_err());

        // Valid lz4
        let valid_lz4 = CompressionConfig {
            enabled: true,
            algorithm: CompressionAlgorithm::Lz4,
            level: 3,
        };
        assert!(validate_compression_config(&valid_lz4).is_ok());

        // Disabled compression (should always pass)
        let disabled = CompressionConfig {
            enabled: false,
            algorithm: CompressionAlgorithm::None, // Using None as it's disabled
            level: 100,
        };
        assert!(validate_compression_config(&disabled).is_ok());
    }

    #[test]
    fn test_validate_retention() {
        // Valid retention
        let valid = RetentionConfig {
            enabled: true,
            period_seconds: 3600,
            cleanup_interval_seconds: 300,
        };
        assert!(validate_retention_config(&valid, &TimeHierarchyConfig::default()).is_ok());

        // Zero period when enabled
        let zero_period = RetentionConfig {
            enabled: true,
            period_seconds: 0,
            cleanup_interval_seconds: 300,
        };
        assert!(validate_retention_config(&zero_period, &TimeHierarchyConfig::default()).is_ok(), "Retention with period_seconds = 0 (retain forever) should be valid");

        // Zero cleanup interval
        let zero_interval = RetentionConfig {
            enabled: true,
            period_seconds: 3600,
            cleanup_interval_seconds: 0,
        };
        assert!(validate_retention_config(&zero_interval, &TimeHierarchyConfig::default()).is_err());
    }
}
