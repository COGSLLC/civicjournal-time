// src/test_utils.rs

use crate::config::Config;
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use std::sync::atomic::Ordering;
use crate::{StorageType, TimeLevel, LevelRollupConfig};
use crate::types::time::RollupContentType;
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
            rollup_config: LevelRollupConfig {
                max_items_per_page: 10,
                max_page_age_seconds: 300,
                content_type: RollupContentType::ChildHashes, // Default for tests
            },
            retention_policy: None,
        }];
        config
    })
}

// Add other common test utilities here as needed.

/// Test identifier for debug logs
static mut TEST_COUNTER: u32 = 0;

/// Get a unique identifier for the current test
pub fn get_test_id() -> u32 {
    unsafe {
        TEST_COUNTER += 1;
        TEST_COUNTER
    }
}

/// Helper to log mutex acquisition with test name
pub async fn acquire_test_mutex(test_name: &str) -> tokio::sync::MutexGuard<'static, ()> {
    let test_id = get_test_id();
    println!("[TEST-DEBUG] [{}] [ID:{}] Attempting to acquire mutex", test_name, test_id);
    let guard = SHARED_TEST_ID_MUTEX.lock().await;
    println!("[TEST-DEBUG] [{}] [ID:{}] Successfully acquired mutex", test_name, test_id);
    guard
}

lazy_static! {
    pub static ref SHARED_TEST_ID_MUTEX: Mutex<()> = {
        println!("[TEST-DEBUG] Initializing SHARED_TEST_ID_MUTEX");
        Mutex::new(())
    };
}

pub fn reset_global_ids() {
    // These paths assume NEXT_PAGE_ID is pub(crate) in crate::core::page
    // and NEXT_LEAF_ID is pub(crate) in crate::core::leaf
    // If they were made fully public, the path might be simpler, but crate::core::... should work.
    println!("[TEST-DEBUG] Resetting global IDs");
    crate::core::page::NEXT_PAGE_ID.store(0, Ordering::SeqCst);
    crate::core::leaf::NEXT_LEAF_ID.store(0, Ordering::SeqCst);
    println!("[TEST-DEBUG] Global IDs reset complete");
}
