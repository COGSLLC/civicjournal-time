// src/test_utils.rs

#![cfg(test)] // Ensure this module is only compiled for tests

use crate::config::Config;
use crate::{RollupConfig, StorageType, TimeLevel};
use std::sync::OnceLock;

/// Provides a common test configuration.
/// Initializes with in-memory storage and a single L0 time level.
pub fn get_test_config() -> &'static Config {
    static TEST_CONFIG: OnceLock<Config> = OnceLock::new();
    TEST_CONFIG.get_or_init(|| {
        let mut config = Config::default();
        config.storage.storage_type = StorageType::Memory;
        config.storage.base_path = "".to_string(); // In-memory, path not strictly needed but good for consistency
        config.time_hierarchy.levels = vec![TimeLevel {
            name: "L0_test_default".to_string(), // Generic name for test config
            duration_seconds: 60,
            rollup_config: RollupConfig {
                max_leaves_per_page: 10,
                max_page_age_seconds: 300,
                force_rollup_on_shutdown: false,
            },
            retention_policy: None,
        }];
        config
    })
}

// Add other common test utilities here as needed.
