use civicjournal_time::api::async_api::Journal;
use civicjournal_time::config::{Config, TimeLevel, LevelRollupConfig, CompressionConfig};
use civicjournal_time::types::time::RollupContentType;
use civicjournal_time::StorageType;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};
use civicjournal_time::storage::file::FileStorage;
use civicjournal_time::core::page::PageContent;
use chrono::{Utc, Duration};
use serde_json::json;
use tempfile::tempdir;

fn config_with_file_storage(dir: &std::path::Path) -> Config {
    let mut cfg = Config::default();
    cfg.storage.storage_type = StorageType::File;
    cfg.storage.base_path = dir.to_str().unwrap().to_string();
    cfg.compression = CompressionConfig { enabled: false, algorithm: civicjournal_time::CompressionAlgorithm::None, level: 0 };
    cfg.time_hierarchy.levels = vec![
        TimeLevel {
            name: "L0".into(),
            duration_seconds: 60,
            rollup_config: LevelRollupConfig { max_items_per_page: 2, max_page_age_seconds: 60, content_type: RollupContentType::ChildHashes },
            retention_policy: None,
        },
        TimeLevel {
            name: "L1".into(),
            duration_seconds: 300,
            rollup_config: LevelRollupConfig { max_items_per_page: 10, max_page_age_seconds: 600, content_type: RollupContentType::ChildHashes },
            retention_policy: None,
        },
    ];
    cfg
}

#[tokio::test]
async fn test_file_storage_rollup_integration() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let tmp = tempdir().unwrap();
    let cfg: &'static Config = Box::leak(Box::new(config_with_file_storage(tmp.path())));
    let journal = Journal::new(cfg).await.expect("journal init");

    let t0 = Utc::now();
    journal.append_leaf(t0, None, "c".into(), json!({"id":1})).await.unwrap();
    journal.append_leaf(t0 + Duration::seconds(1), None, "c".into(), json!({"id":2})).await.unwrap();
    journal.append_leaf(t0 + Duration::seconds(2), None, "c".into(), json!({"id":3})).await.unwrap();

    // Open storage separately to verify pages
    let storage = FileStorage::new(tmp.path().to_path_buf(), cfg.compression.clone()).await.unwrap();
    // L1 page 0 should exist with thrall hash of first L0 page
    let l1_page = storage.load_page(1, 0).await.unwrap().expect("L1 page missing");
    let l0_page0 = storage.load_page(0, 0).await.unwrap().unwrap();
    match l1_page.content {
        PageContent::ThrallHashes(h) => {
            assert_eq!(h.len(), 1);
            assert_eq!(h[0], l0_page0.page_hash);
        }
        _ => panic!("expected thrall hashes"),
    }

    // Query API should return all leaves
    let report = journal.get_delta_report("c", t0, t0 + Duration::seconds(3)).await.unwrap();
    assert_eq!(report.deltas.len(), 3);
}
