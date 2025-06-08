#![cfg(feature = "demo")]

use civicjournal_time::demo::{DemoConfig, generator::LeafGenerator};
use civicjournal_time::api::async_api::Journal;
use civicjournal_time::test_utils::{get_test_config, SHARED_TEST_ID_MUTEX, reset_global_ids};

#[tokio::test]
async fn test_demo_config_load() {
    let mut cfg = DemoConfig::load("Journal.toml").expect("load config");
    cfg.database_url = None;
    assert!(cfg.users > 0);
}

#[tokio::test]
async fn test_leaf_generator_init() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let mut cfg = DemoConfig::load("Journal.toml").expect("load config");
    cfg.database_url = None;
    let journal = Journal::new(get_test_config()).await.unwrap();
    let mut gen = LeafGenerator::new(cfg, journal, true).await.unwrap();
    let (_container, payload) = gen.generate_payload();
    assert!(payload.get("user").is_some());
}
