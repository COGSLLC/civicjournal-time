use crate::config::{
    CompressionAlgorithm, CompressionConfig, Config, LogLevel, LoggingConfig, RetentionConfig,
    LevelRollupConfig, StorageConfig, StorageType, TimeHierarchyConfig,
};
use crate::error::CJError;
use crate::config::validation::validate_config;

fn create_test_config() -> Config {
    Config {
        time_hierarchy: TimeHierarchyConfig::default(),
        storage: StorageConfig {
            storage_type: StorageType::File,
            base_path: "./data".to_string(),
            max_open_files: 100,
        },
        compression: CompressionConfig {
            enabled: true,
            algorithm: CompressionAlgorithm::Zstd,
            level: 3,
        },
        logging: LoggingConfig {
            level: LogLevel::Info,
            console: true,
            file: true,
            file_path: "./logs/civicjournal.log".to_string(),
        },
        metrics: Default::default(),
        retention: RetentionConfig {
            enabled: true,
            period_seconds: 30 * 24 * 60 * 60, // 30 days
            cleanup_interval_seconds: 3600, // 1 hour
        },
        force_rollup_on_shutdown: true,
    }
}

#[test]
fn test_valid_config() -> Result<(), CJError> {
    let mut config = create_test_config();

    let unique_log_dir = "./temp_test_logs_valid_config";
    let unique_storage_dir = "./temp_test_data_valid_config";

    // Update config to use unique paths
    if config.logging.file {
        config.logging.file_path = format!("{}/app.log", unique_log_dir);
    }
    config.storage.base_path = unique_storage_dir.to_string();

    // Create unique directories
    if config.logging.file {
        std::fs::create_dir_all(unique_log_dir)
            .expect("Failed to create unique_log_dir for test_valid_config");
    }
    std::fs::create_dir_all(unique_storage_dir)
        .expect("Failed to create unique_storage_dir for test_valid_config");

    // Perform validation
    let validation_result = validate_config(&config);

    // Clean up unique directories immediately, regardless of validation outcome
    if config.logging.file {
        let _ = std::fs::remove_dir_all(unique_log_dir);
    }
    let _ = std::fs::remove_dir_all(unique_storage_dir);

    // Assert validation was successful
    validation_result?;

    Ok(())
}

#[test]
fn test_invalid_time_hierarchy() {
    let mut config = create_test_config();
    
    // Disable all time levels by clearing the vector
    config.time_hierarchy.levels.clear();
    
    let result = validate_config(&config);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Time hierarchy must have at least one level"));
}

#[test]
fn test_storage_validation() -> Result<(), CJError> {
    let mut config = create_test_config();
    // let storage_base_path = "./test_data_storage_validation"; // This variable was unused

    // Backup and disable file logging for the entire test to isolate storage validation
    let original_logging_config = config.logging.clone();
    config.logging.file = false;

    // Test with empty base path for file storage
    config.storage.base_path = "".to_string();
    let result = validate_config(&config);
    assert!(result.is_err(), "Validation should fail with empty base path");
    
    // Test with invalid base path
    config.storage.base_path = "/invalid/path/that/does/not/exist".to_string();
    let result = validate_config(&config);
    assert!(result.is_err(), "Validation should fail with invalid base path");
    
    // Test with valid base path (should be created if needed)
    config.storage.base_path = "./test_data".to_string();
    let storage_path = std::path::Path::new(&config.storage.base_path);
    std::fs::create_dir_all(storage_path).expect("Failed to create storage_path for test_storage_validation");
    let result = validate_config(&config);
    assert!(result.is_ok(), "Validation should pass with valid base path. Error: {:?}", result.err());
    
    // Clean up test directory
    let _ = std::fs::remove_dir_all(storage_path);
    
    // Test with minimum valid max_open_files
    // Ensure base_path is valid again as it might have been cleaned up
    config.storage.base_path = "./test_data_aux".to_string(); // Use a different path to avoid conflict if previous cleanup failed
    let aux_storage_path = std::path::Path::new(&config.storage.base_path);
    std::fs::create_dir_all(aux_storage_path).expect("Failed to create aux_storage_path for test_storage_validation");
    config.storage.max_open_files = 10;
    // File logging is already disabled globally for this test
    let result = validate_config(&config);
    assert!(result.is_ok(), "Validation should pass with minimum max_open_files. Error: {:?}", result.err());
    let _ = std::fs::remove_dir_all(aux_storage_path); // Clean up aux path
    
    // Test with invalid max_open_files
    config.storage.max_open_files = 0;
    let result = validate_config(&config);
    assert!(result.is_err(), "Validation should fail with zero max_open_files");

    // Restore original logging config before exiting
    config.logging = original_logging_config;
    Ok(())
}

#[test]
fn test_invalid_compression_level() {
    let mut config = create_test_config();
    
    // Set invalid compression level for Zstd
    config.compression.level = 30; // Zstd max is 22
    
    let result = validate_config(&config);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Zstd compression level"));
}

#[test]
fn test_logging_config() {
    let mut config = create_test_config();
    
    // Test with file logging enabled but no file path
    config.logging.file = true;
    config.logging.file_path = "".to_string();
    let result = validate_config(&config);
    assert!(result.is_err(), "Validation should fail with empty file path when file logging is enabled");
    
    // Test with invalid log level (this should be a compile-time check with the LogLevel enum)
    // So we don't need to test invalid string values
    
    // Test with valid configuration
    config.logging.file_path = "./test_logs/civicjournal.log".to_string();
    config.logging.level = LogLevel::Debug;
    if let Some(parent_dir) = std::path::Path::new(&config.logging.file_path).parent() {
        std::fs::create_dir_all(parent_dir).expect("Failed to create log parent dir for test_logging_config");
    }
    let result = validate_config(&config);
    assert!(result.is_ok(), "Validation should pass with valid logging config");
    
    // Clean up test directory
    if let Some(parent_dir) = std::path::Path::new(&config.logging.file_path).parent() {
        if parent_dir == std::path::Path::new("./test_logs") { // Only remove if it's the specific test dir
             let _ = std::fs::remove_dir_all(parent_dir);
        }
    }
    
    // Test with file logging disabled - should pass even with invalid path
    config.logging.file = false;
    config.logging.file_path = "".to_string();
    let result = validate_config(&config);
    assert!(result.is_ok(), "Validation should pass with file logging disabled");
}

#[test]
fn test_rollup_validation() {
    // TODO: This test needs to be rewritten to validate per-level rollup configurations
    // within the TimeHierarchyConfig. For now, it's partially disabled to allow compilation.
    let mut config = Config::default();

    // Test invalid max_leaves_per_page (now max_items_per_page for a specific level)
    // Simulating setting L0's max_items_per_page to 0
    if !config.time_hierarchy.levels.is_empty() {
        let original_max_items = config.time_hierarchy.levels[0].rollup_config.max_items_per_page;
        config.time_hierarchy.levels[0].rollup_config.max_items_per_page = 0;
        assert!(validate_config(&config).is_err(), "Validation should fail with max_items_per_page = 0 for L0");
        config.time_hierarchy.levels[0].rollup_config.max_items_per_page = original_max_items; // Reset
    } else {
        panic!("Default config has no time hierarchy levels, cannot run test_rollup_validation effectively.");
    }
    
    // Test invalid max_page_age_seconds (for a specific level)
    // Simulating setting L0's max_page_age_seconds to 0
    if !config.time_hierarchy.levels.is_empty() {
        let original_max_age = config.time_hierarchy.levels[0].rollup_config.max_page_age_seconds;
        config.time_hierarchy.levels[0].rollup_config.max_page_age_seconds = 0;
        assert!(validate_config(&config).is_err(), "Validation should fail with max_page_age_seconds = 0 for L0");
        config.time_hierarchy.levels[0].rollup_config.max_page_age_seconds = original_max_age; // Reset
    } 
    // The rest of the original test is commented out as it relied on global config.rollup
    // /*
    // let mut config = create_test_config();

    // // Test invalid max_leaves_per_page
    // config.rollup.max_leaves_per_page = 0;
    // let result = validate_config(&config);
    // assert!(result.is_err());
    // assert!(result.unwrap_err().to_string().contains("max_leaves_per_page must be greater than 0"));

    // // Reset to valid and test invalid max_page_age_seconds
    // config = create_test_config(); // Reset to default valid config
    // config.rollup.max_page_age_seconds = 0;
    // let result2 = validate_config(&config);
    // assert!(result2.is_err(), "Validation should fail with max_page_age_seconds = 0");
    // assert!(result2.unwrap_err().to_string().contains("max_page_age_seconds must be greater than 0"));
    // */
}

#[test]
fn test_retention_validation() {
    let config = create_test_config();
    
    // Test with retention disabled - should always pass
    let mut temp_config = create_test_config(); // Use a mutable copy
    temp_config.retention.enabled = false;

    // Use unique paths for this test section to avoid interference
    let unique_log_dir = "./temp_test_logs_retention_disabled";
    let unique_storage_dir = "./temp_test_data_retention_disabled";
    temp_config.logging.file_path = format!("{}/test.log", unique_log_dir);
    temp_config.storage.base_path = unique_storage_dir.to_string();

    // Ensure these unique paths are created
    std::fs::create_dir_all(unique_log_dir).expect("Failed to create unique_log_dir for test_retention_validation");
    std::fs::create_dir_all(unique_storage_dir).expect("Failed to create unique_storage_dir for test_retention_validation");

    let result = validate_config(&temp_config);
    assert!(result.is_ok(), "Validation should pass with retention disabled. Error: {:?}", result.err());

    // Clean up unique created directories
    let _ = std::fs::remove_dir_all(unique_log_dir);
    let _ = std::fs::remove_dir_all(unique_storage_dir);
    
    // Scenario: Invalid cleanup interval (0)
    {
        let mut scenario_config = config.clone();
        let log_dir = "./temp_logs_ret_invalid_interval";
        let storage_dir = "./temp_data_ret_invalid_interval";
        scenario_config.logging.file_path = format!("{}/test.log", log_dir);
        scenario_config.storage.base_path = storage_dir.to_string();
        std::fs::create_dir_all(log_dir).expect("Failed to create log_dir for invalid_interval test");
        std::fs::create_dir_all(storage_dir).expect("Failed to create storage_dir for invalid_interval test");

        scenario_config.retention.enabled = true;
        scenario_config.retention.cleanup_interval_seconds = 0;
        let result = validate_config(&scenario_config);
        assert!(result.is_err(), "Validation should fail with zero cleanup interval. Actual: {:?}", result.as_ref().ok());
        assert!(result.unwrap_err().to_string().contains("Cleanup interval must be greater than 0"));

        let _ = std::fs::remove_dir_all(log_dir);
        let _ = std::fs::remove_dir_all(storage_dir);
    }
    
    // Scenario: Cleanup interval greater than retention period
    {
        let mut scenario_config = config.clone();
        let log_dir = "./temp_logs_ret_cleanup_gt_period";
        let storage_dir = "./temp_data_ret_cleanup_gt_period";
        scenario_config.logging.file_path = format!("{}/test.log", log_dir);
        scenario_config.storage.base_path = storage_dir.to_string();
        std::fs::create_dir_all(log_dir).expect("Failed to create log_dir for cleanup_gt_period test");
        std::fs::create_dir_all(storage_dir).expect("Failed to create storage_dir for cleanup_gt_period test");

        scenario_config.retention.enabled = true;
        scenario_config.retention.cleanup_interval_seconds = 3600; // 1 hour
        scenario_config.retention.period_seconds = 1800; // 30 minutes
        let result = validate_config(&scenario_config);
        assert!(result.is_err(), "Validation should fail when cleanup interval > retention period. Actual: {:?}", result.as_ref().ok());
        assert!(result.unwrap_err().to_string().contains("Retention period must be greater than or equal to cleanup interval"));

        let _ = std::fs::remove_dir_all(log_dir);
        let _ = std::fs::remove_dir_all(storage_dir);
    }
    
    // Scenario: Valid retention settings (retain forever)
    {
        let mut scenario_config = config.clone();
        let log_dir = "./temp_logs_ret_retain_forever";
        let storage_dir = "./temp_data_ret_retain_forever";
        scenario_config.logging.file_path = format!("{}/test.log", log_dir);
        scenario_config.storage.base_path = storage_dir.to_string();
        std::fs::create_dir_all(log_dir).expect("Failed to create log_dir for retain_forever test");
        std::fs::create_dir_all(storage_dir).expect("Failed to create storage_dir for retain_forever test");

        scenario_config.retention.enabled = true;
        scenario_config.retention.cleanup_interval_seconds = 3600; // Needs a valid cleanup interval
        scenario_config.retention.period_seconds = 0; // retain forever
        let result = validate_config(&scenario_config);
        assert!(result.is_ok(), "Validation should pass with valid retention settings (retain forever). Error: {:?}", result.err());

        let _ = std::fs::remove_dir_all(log_dir);
        let _ = std::fs::remove_dir_all(storage_dir);
    }
    
    // Scenario: Valid retention settings (specific period)
    {
        let mut scenario_config = config.clone();
        let log_dir = "./temp_logs_ret_specific_period";
        let storage_dir = "./temp_data_ret_specific_period";
        scenario_config.logging.file_path = format!("{}/test.log", log_dir);
        scenario_config.storage.base_path = storage_dir.to_string();
        std::fs::create_dir_all(log_dir).expect("Failed to create log_dir for specific_period test");
        std::fs::create_dir_all(storage_dir).expect("Failed to create storage_dir for specific_period test");

        scenario_config.retention.enabled = true;
        scenario_config.retention.period_seconds = 86400; // 1 day
        scenario_config.retention.cleanup_interval_seconds = 3600; // 1 hour
        let result = validate_config(&scenario_config);
        assert!(result.is_ok(), "Validation should pass with valid retention settings (specific period). Error: {:?}", result.err());

        let _ = std::fs::remove_dir_all(log_dir);
        let _ = std::fs::remove_dir_all(storage_dir);
    }
}
