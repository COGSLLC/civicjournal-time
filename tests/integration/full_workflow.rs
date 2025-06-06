#![cfg(feature = "async_api")]

use civicjournal_time::api::async_api::{Journal, PageContentHash};
use civicjournal_time::config::{Config, TimeLevel, LevelRollupConfig};
use civicjournal_time::types::time::RollupContentType;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};
use civicjournal_time::StorageType;
use chrono::{Utc, Duration};
use serde_json::json;

fn integration_config() -> Config {
    let mut cfg = Config::default();
    cfg.storage.storage_type = StorageType::Memory;
    cfg.storage.base_path = String::new();
    cfg.time_hierarchy.levels = vec![
        TimeLevel {
            name: "L0".to_string(),
            duration_seconds: 60,
            rollup_config: LevelRollupConfig {
                max_items_per_page: 2,
                max_page_age_seconds: 60,
                content_type: RollupContentType::ChildHashes,
            },
            retention_policy: None,
        },
        TimeLevel {
            name: "L1".to_string(),
            duration_seconds: 300,
            rollup_config: LevelRollupConfig {
                max_items_per_page: 2,
                max_page_age_seconds: 600,
                content_type: RollupContentType::ChildHashes,
            },
            retention_policy: None,
        },
    ];
    cfg
}

#[tokio::test]
async fn test_end_to_end_flow() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg: &'static Config = Box::leak(Box::new(integration_config()));
    let journal = Journal::new(cfg).await.expect("init journal");

    let base = Utc::now();
    journal
        .append_leaf(base, None, "c1".to_string(), json!({"a": 1}))
        .await
        .unwrap();
    let hash2 = match journal
        .append_leaf(base + Duration::seconds(1), None, "c1".to_string(), json!({"b": 2}))
        .await
        .unwrap()
    {
        PageContentHash::LeafHash(h) => h,
        _ => panic!("expected leaf hash"),
    };
    journal
        .append_leaf(base + Duration::seconds(2), None, "c1".to_string(), json!({"c": 3}))
        .await
        .unwrap();

    let proof = journal.get_leaf_inclusion_proof(&hash2).await.unwrap();
    assert_eq!(proof.leaf.delta_payload, json!({"b": 2}));

    let state = journal
        .reconstruct_container_state("c1", base + Duration::seconds(1))
        .await
        .unwrap();
    assert_eq!(state.state_data["a"], 1);

    let report = journal
        .get_delta_report("c1", base, base + Duration::seconds(3))
        .await
        .unwrap();
    assert_eq!(report.deltas.len(), 3);

    let integrity = journal.get_page_chain_integrity(0, None, None).await.unwrap();
    assert!(!integrity.is_empty());
    assert!(integrity.iter().all(|r| r.is_valid));

    let page = journal.get_page(0, 0).await.unwrap();
    assert_eq!(page.level, 0);
}

