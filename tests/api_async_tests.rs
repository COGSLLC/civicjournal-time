#![cfg(feature = "async_api")]

use civicjournal_time::api::async_api::{Journal, PageContentHash};
use civicjournal_time::config::{Config, TimeLevel, LevelRollupConfig};
use civicjournal_time::types::time::RollupContentType;
use civicjournal_time::{StorageType, CJError};
use civicjournal_time::storage::memory::MemoryStorage;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::time_manager::TimeHierarchyManager;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids, get_test_config};
use chrono::{Utc, Duration};
use serde_json::json;
use std::sync::Arc;

fn rollup_config() -> Config {
    let mut cfg = Config::default();
    cfg.storage.storage_type = StorageType::Memory;
    cfg.storage.base_path = "".to_string();
    cfg.time_hierarchy.levels = vec![
        TimeLevel {
            name: "L0".to_string(),
            duration_seconds: 60,
            rollup_config: LevelRollupConfig {
                max_items_per_page: 2,
                max_page_age_seconds: 300,
                content_type: RollupContentType::ChildHashes,
            },
            retention_policy: None,
        },
        TimeLevel {
            name: "L1".to_string(),
            duration_seconds: 300,
            rollup_config: LevelRollupConfig {
                max_items_per_page: 10,
                max_page_age_seconds: 86400,
                content_type: RollupContentType::ChildHashes,
            },
            retention_policy: None,
        },
    ];
    cfg
}

#[tokio::test]
async fn test_journal_new_success() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = get_test_config();
    let res = Journal::new(cfg).await;
    assert!(res.is_ok(), "Journal::new should succeed: {:?}", res.err());
}

#[tokio::test]
async fn test_journal_new_invalid_storage() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let mut cfg = Config::default();
    cfg.storage.storage_type = StorageType::File;
    cfg.storage.base_path = "".to_string();
    let cfg_static: &'static Config = Box::leak(Box::new(cfg));
    let res = Journal::new(cfg_static).await;
    assert!(matches!(res, Err(CJError::InvalidInput(_))));
}

#[tokio::test]
async fn test_append_single_leaf_returns_hash() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = get_test_config();
    let journal = Journal::new(cfg).await.expect("journal init");
    let ts = Utc::now();
    let res = journal.append_leaf(ts, None, "c1".into(), json!({"a":1})).await.unwrap();
    match res {
        PageContentHash::LeafHash(h) => assert_eq!(h.len(), 32),
        _ => panic!("Expected LeafHash"),
    }
}

#[tokio::test]
async fn test_append_multiple_leaves_unique_hashes() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = get_test_config();
    let journal = Journal::new(cfg).await.expect("journal init");
    let t0 = Utc::now();
    let h1 = journal.append_leaf(t0, None, "c1".into(), json!({"v":1})).await.unwrap();
    let h2 = journal.append_leaf(t0 + Duration::milliseconds(1), None, "c1".into(), json!({"v":2})).await.unwrap();
    let h3 = journal.append_leaf(t0 + Duration::milliseconds(2), None, "c1".into(), json!({"v":3})).await.unwrap();
    assert_ne!(h1, h2);
    assert_ne!(h2, h3);
    assert_ne!(h1, h3);
}

#[tokio::test]
async fn test_append_leaf_storage_error() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let mut cfg = get_test_config().clone();
    cfg.time_hierarchy.levels[0].rollup_config.max_items_per_page = 1;
    let mem = MemoryStorage::new();
    mem.set_fail_on_store(0, None);
    let storage: Arc<dyn StorageBackend> = Arc::new(mem);
    let manager = Arc::new(TimeHierarchyManager::new(Arc::new(cfg.clone()), storage.clone()));
    let query = civicjournal_time::query::QueryEngine::new(storage.clone(), manager.clone(), Arc::new(cfg));
    let journal = Journal { manager, query };
    let ts = Utc::now();
    let res = journal.append_leaf(ts, None, "err".into(), json!({"x":1})).await;
    assert!(matches!(res, Err(CJError::StorageError(_))));
}

#[tokio::test]
async fn test_rollup_trigger() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg: &'static Config = Box::leak(Box::new(rollup_config()));
    let journal = Journal::new(cfg).await.expect("init");
    let base = Utc::now();
    journal.append_leaf(base, None, "r1".into(), json!({"v":1})).await.unwrap();
    journal.append_leaf(base + Duration::milliseconds(1), None, "r1".into(), json!({"v":2})).await.unwrap();
    // Third append should roll up first two pages
    let res = journal.append_leaf(base + Duration::milliseconds(2), None, "r1".into(), json!({"v":3})).await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn test_get_page_not_found() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = get_test_config();
    let journal = Journal::new(cfg).await.expect("init");
    let res = journal.get_page(0, 42).await;
    assert!(matches!(res, Err(CJError::PageNotFound { level: 0, page_id: 42 })));
}

#[tokio::test]
async fn test_get_page_existing() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = get_test_config();
    let journal = Journal::new(cfg).await.expect("init");
    let ts = Utc::now();
    journal.append_leaf(ts, None, "p1".into(), json!({"v":1})).await.unwrap();
    let page = journal.get_page(0, 0).await.expect("page");
    assert_eq!(page.level, 0);
    assert_eq!(page.page_id, 0);
}

#[tokio::test]
async fn test_async_query_methods() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg: &'static Config = Box::leak(Box::new(rollup_config()));
    let journal = Journal::new(cfg).await.expect("init");
    let base = Utc::now();
    // append three leaves
    let h1 = match journal.append_leaf(base, None, "c1".into(), json!({"a":1})).await.unwrap() { PageContentHash::LeafHash(h) => h, _ => panic!() };
    let h2 = match journal.append_leaf(base + Duration::seconds(1), Some(PageContentHash::LeafHash(h1)), "c1".into(), json!({"b":2})).await.unwrap() { PageContentHash::LeafHash(h) => h, _ => panic!() };
    let _h3 = journal.append_leaf(base + Duration::seconds(2), Some(PageContentHash::LeafHash(h2)), "c1".into(), json!({"c":3})).await.unwrap();

    let proof = journal.get_leaf_inclusion_proof(&h2).await.unwrap();
    assert_eq!(proof.leaf.leaf_hash, h2);
    let state = journal.reconstruct_container_state("c1", base + Duration::seconds(1)).await.unwrap();
    assert_eq!(state.state_data["a"], 1);
    assert_eq!(state.state_data["b"], 2);

    let report = journal.get_delta_report("c1", base, base + Duration::seconds(3)).await.unwrap();
    assert_eq!(report.deltas.len(), 3);

    let integrity = journal.get_page_chain_integrity(0, Some(0), Some(1)).await.unwrap();
    assert_eq!(integrity.len(), 2);
    assert!(integrity.iter().all(|r| r.is_valid));
}
