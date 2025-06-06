use civicjournal_time::{init};
use civicjournal_time::config::Config;
use civicjournal_time::StorageType;
use civicjournal_time::CJError;
use tempfile::tempdir;
use std::fs;

#[tokio::test]
async fn test_init_fails_when_logger_already_set() {
    let _guard = civicjournal_time::test_utils::SHARED_TEST_ID_MUTEX.lock().await;
    // Pre-initialize logger
    let _ = env_logger::builder().is_test(true).try_init();

    let dir = tempdir().unwrap();
    let config_path = dir.path().join("cfg.toml");
    let mut cfg = Config::default();
    cfg.storage.storage_type = StorageType::Memory;
    cfg.storage.base_path = "".into();
    fs::write(&config_path, toml::to_string(&cfg).unwrap()).unwrap();

    let result = init(Some(config_path.to_str().unwrap()));
    assert!(matches!(result, Err(CJError::InvalidInput(_))));
}
