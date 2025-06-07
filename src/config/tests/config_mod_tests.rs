use super::*;
use tempfile::tempdir;
use std::fs;
use std::path::Path;

#[test]
fn test_config_default_values() {
    let cfg = Config::default();
    assert_eq!(cfg.time_hierarchy.levels.len(), 4);
    assert!(cfg.force_rollup_on_shutdown);
}

#[test]
fn test_load_existing_file_and_missing_file() {
    let dir = tempdir().expect("create temp dir");
    // Prepare a config with a temp storage path to satisfy validation
    let mut cfg = Config::default();
    let storage_path = dir.path().join("data");
    fs::create_dir_all(&storage_path).unwrap();
    cfg.storage.base_path = storage_path.to_string_lossy().into();
    cfg.logging.level = LogLevel::Debug;
    cfg.force_rollup_on_shutdown = false;

    let toml_string = toml::to_string(&cfg).expect("serialize config");
    let config_path = dir.path().join("cfg.toml");
    fs::write(&config_path, toml_string).unwrap();

    let loaded = Config::load(&config_path).expect("load existing config");
    assert_eq!(loaded.logging.level, LogLevel::Debug);
    assert!(!loaded.force_rollup_on_shutdown);

    // Nonexistent file should fall back to defaults
    let missing_path = dir.path().join("missing.toml");
    let default_loaded = Config::load(&missing_path).expect("load missing");
    assert_eq!(default_loaded.time_hierarchy.levels.len(), 4);
    assert!(default_loaded.force_rollup_on_shutdown);
}

#[test]
fn test_load_invalid_toml_fails() {
    let dir = tempdir().expect("create temp dir");
    let invalid_path = dir.path().join("bad.toml");
    fs::write(&invalid_path, "not = [valid\n").unwrap();

    let err = Config::load(&invalid_path);
    assert!(err.is_err());
    let msg = format!("{}", err.unwrap_err());
    assert!(msg.contains("Failed to parse config file"));
}

#[test]
fn test_apply_env_vars() {
    std::env::set_var("CJ_LOGGING_LEVEL", "debug");
    let mut cfg = Config::default();
    cfg.apply_env_vars().unwrap();
    assert_eq!(cfg.logging.level, LogLevel::Debug);
    std::env::remove_var("CJ_LOGGING_LEVEL");

    std::env::set_var("CJ_LOGGING_LEVEL", "bogus");
    let mut cfg = Config::default();
    let err = cfg.apply_env_vars();
    assert!(err.is_err());
    std::env::remove_var("CJ_LOGGING_LEVEL");
}

#[test]
fn test_validate_invalid_config() {
    let mut cfg = Config::default();
    cfg.time_hierarchy.levels[0].rollup_config.max_items_per_page = 0;
    assert!(cfg.validate().is_err());
}

#[test]
fn test_config_dir_returns_valid_path() {
    if let Some(path) = Config::config_dir() {
        // Path should end with the application directory name
        assert!(Path::new(&path).ends_with("civicjournal-time"));
    } else {
        panic!("config_dir returned None");
    }
}

