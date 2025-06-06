#![cfg(feature = "async_api")]

use civicjournal_time::api::async_api::{Journal, PageContentHash};
use civicjournal_time::config::{Config, TimeLevel, LevelRollupConfig};
use civicjournal_time::types::time::RollupContentType;
use civicjournal_time::{StorageType};
use civicjournal_time::error::CJError;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};
use chrono::{Utc, Duration};
use serde_json::json;

fn single_leaf_config() -> Config {
    let mut cfg = Config::default();
    cfg.storage.storage_type = StorageType::Memory;
    cfg.storage.base_path = "".to_string();
    cfg.time_hierarchy.levels = vec![TimeLevel {
        name: "L0_test".to_string(),
        duration_seconds: 60,
        rollup_config: LevelRollupConfig {
            max_items_per_page: 1,
            max_page_age_seconds: 300,
            content_type: RollupContentType::ChildHashes,
        },
        retention_policy: None,
    }];
    cfg
}

#[tokio::test]
async fn test_api_leaf_inclusion_proof() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");

    let ts = Utc::now();
    let res = journal
        .append_leaf(ts, None, "container1".to_string(), json!({"v":1}))
        .await
        .expect("append");
    let hash = match res { PageContentHash::LeafHash(h) => h, _ => panic!("expected LeafHash") };

    let proof = journal.get_leaf_inclusion_proof(&hash).await.expect("proof");
    assert_eq!(proof.leaf.leaf_hash, hash);
    assert_eq!(proof.level, 0);
}

#[tokio::test]
async fn test_api_leaf_inclusion_proof_not_found() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");
    let fake_hash = [0u8; 32];
    let result = journal.get_leaf_inclusion_proof(&fake_hash).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_api_reconstruct_and_delta() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");
    let t0 = Utc::now();
    journal.append_leaf(t0, None, "c1".into(), json!({"a":1})).await.unwrap();
    journal.append_leaf(t0 + Duration::seconds(1), None, "c1".into(), json!({"b":2})).await.unwrap();
    journal.append_leaf(t0 + Duration::seconds(2), None, "c1".into(), json!({"a":3})).await.unwrap();

    let state = journal.reconstruct_container_state("c1", t0 + Duration::seconds(1)).await.unwrap();
    assert_eq!(state.state_data["a"], 1);
    assert_eq!(state.state_data["b"], 2);

    let report = journal.get_delta_report("c1", t0, t0 + Duration::seconds(3)).await.unwrap();
    assert_eq!(report.deltas.len(), 3);
}

#[tokio::test]
async fn test_api_page_chain_integrity() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");
    let ts = Utc::now();
    journal.append_leaf(ts, None, "pc1".into(), json!({"x":1})).await.unwrap();

    let reports = journal.get_page_chain_integrity(0, None, None).await.unwrap();
    assert!(!reports.is_empty());
    assert!(reports.iter().all(|r| r.is_valid));
}

#[tokio::test]
async fn test_api_get_page_success() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");

    let ts = Utc::now();
    journal.append_leaf(ts, None, "gp1".into(), json!({"v":1})).await.unwrap();

    let page = journal.get_page(0, 0).await.expect("page");
    assert_eq!(page.level, 0);
    assert_eq!(page.page_id, 0);
}

#[tokio::test]
async fn test_api_get_page_not_found() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");

    let result = journal.get_page(0, 42).await;
    assert!(matches!(result, Err(CJError::PageNotFound { level: 0, page_id: 42 })));
}

#[tokio::test]
async fn test_api_invalid_delta_range() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");

    let t0 = Utc::now();
    let res = journal.get_delta_report("c", t0 + Duration::seconds(1), t0).await;
    assert!(matches!(res, Err(CJError::InvalidInput(_))));
}
#[tokio::test]
async fn test_api_reconstruct_state_not_found() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");
    let now = Utc::now();
    let res = journal.reconstruct_container_state("missing", now).await;
    assert!(matches!(res, Err(CJError::InvalidInput(_))));
}

#[tokio::test]
async fn test_api_delta_report_container_not_found() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");
    let now = Utc::now();
    let res = journal.get_delta_report("missing", now, now + Duration::seconds(1)).await;
    assert!(matches!(res, Err(CJError::InvalidInput(_))));
}

#[tokio::test]
async fn test_api_page_chain_integrity_invalid_range() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = single_leaf_config();
    let journal = Journal::new(Box::leak(Box::new(cfg))).await.expect("journal init");
    let res = journal.get_page_chain_integrity(0, Some(3), Some(1)).await;
    assert!(matches!(res, Err(CJError::InvalidInput(_))));
}
