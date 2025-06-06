use civicjournal_time::core::page::{JournalPage, PageContent};
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::config::{Config, TimeHierarchyConfig, TimeLevel, LevelRollupConfig};
use civicjournal_time::types::time::RollupContentType;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};
use chrono::{Utc, Duration};
use serde_json::json;
use std::collections::HashMap;

fn net_patch_config() -> Config {
    Config {
        time_hierarchy: TimeHierarchyConfig {
            levels: vec![
                TimeLevel {
                    name: "L0".to_string(),
                    duration_seconds: 60,
                    rollup_config: LevelRollupConfig {
                        max_items_per_page: 10,
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
                        max_page_age_seconds: 600,
                        content_type: RollupContentType::NetPatches,
                    },
                    retention_policy: None,
                },
            ],
        },
        force_rollup_on_shutdown: false,
        ..Default::default()
    }
}

#[tokio::test]
async fn test_net_patch_canonical_hashing() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = net_patch_config();
    let now = Utc::now();

    let mut page1 = JournalPage::new_with_id(0, 1, None, now, &cfg);
    page1.content = PageContent::NetPatches(HashMap::new());
    let mut p1_obj2 = HashMap::new();
    p1_obj2.insert("b".to_string(), json!(2));
    let mut p1_obj1 = HashMap::new();
    p1_obj1.insert("a".to_string(), json!(1));
    if let PageContent::NetPatches(ref mut map) = page1.content {
        map.insert("obj2".to_string(), p1_obj2);
        map.insert("obj1".to_string(), p1_obj1);
    }
    page1.recalculate_merkle_root_and_page_hash();

    let mut page2 = JournalPage::new_with_id(0, 1, None, now, &cfg);
    page2.content = PageContent::NetPatches(HashMap::new());
    let mut p2_obj1 = HashMap::new();
    p2_obj1.insert("a".to_string(), json!(1));
    let mut p2_obj2 = HashMap::new();
    p2_obj2.insert("b".to_string(), json!(2));
    if let PageContent::NetPatches(ref mut map) = page2.content {
        map.insert("obj1".to_string(), p2_obj1);
        map.insert("obj2".to_string(), p2_obj2);
    }
    page2.recalculate_merkle_root_and_page_hash();

    assert_eq!(page1.page_hash, page2.page_hash, "hashing should be order independent");
}

#[tokio::test]
async fn test_should_finalize_conditions() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = net_patch_config();
    let t0 = Utc::now();
    let mut page = JournalPage::new(0, None, t0, &cfg);
    let leaf = JournalLeaf::new(t0, None, "c".into(), json!({"a":1})).unwrap();
    if let PageContent::Leaves(ref mut v) = page.content { v.push(leaf); }
    page.recalculate_merkle_root_and_page_hash();

    let res = page.should_finalize(t0 + Duration::seconds(10), t0 + Duration::seconds(10), 2, Some(Duration::seconds(300)));
    assert_eq!(res, (false, false));

    let res = page.should_finalize(t0 + Duration::seconds(10), t0 + Duration::seconds(10), 1, Some(Duration::seconds(300)));
    assert_eq!(res, (true, false));

    let res = page.should_finalize(t0 + Duration::seconds(10), t0 + Duration::seconds(301), 2, Some(Duration::seconds(300)));
    assert_eq!(res, (true, false));

    let res = page.should_finalize(page.end_time + Duration::seconds(1), page.end_time + Duration::seconds(1), 2, Some(Duration::seconds(300)));
    assert_eq!(res, (false, true));
}

