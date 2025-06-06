use civicjournal_time::api::sync_api::Journal;
use civicjournal_time::config::Config;
use civicjournal_time::test_utils::get_test_config;
use civicjournal_time::error::CJError;
use civicjournal_time::StorageType;
use tempfile::tempdir;
use chrono::Utc;
use serde_json::json;
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::core::page::{JournalPage, PageContent};
use civicjournal_time::storage::{file::FileStorage, StorageBackend};
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};

#[test]
fn test_sync_leaf_inclusion_proof_not_found() {
    let cfg: &'static Config = get_test_config();
    let journal = Journal::new(cfg).expect("journal init");
    let fake = [0u8; 32];
    let res = journal.get_leaf_inclusion_proof(&fake);
    assert!(res.is_err());
}

#[test]
fn test_sync_reconstruct_state_not_found() {
    let cfg = get_test_config();
    let journal = Journal::new(cfg).expect("journal init");
    let now = Utc::now();
    let res = journal.reconstruct_container_state("missing", now);
    assert!(matches!(res, Err(CJError::InvalidInput(_))));
}

#[test]
fn test_sync_invalid_delta_range() {
    let cfg = get_test_config();
    let journal = Journal::new(cfg).expect("journal init");
    let now = Utc::now();
    let res = journal.get_delta_report("c", now + chrono::Duration::seconds(1), now);
    assert!(matches!(res, Err(CJError::InvalidInput(_))));
}

#[test]
fn test_sync_page_chain_integrity_invalid_range() {
    let cfg = get_test_config();
    let journal = Journal::new(cfg).expect("journal init");
    let res = journal.get_page_chain_integrity(0, Some(2), Some(1));
    assert!(matches!(res, Err(CJError::InvalidInput(_))));
}

#[test]
fn test_sync_get_page_not_found() {
    let cfg: &'static Config = get_test_config();
    let journal = Journal::new(cfg).expect("journal init");
    let res = journal.get_page(0, 99);
    assert!(matches!(res, Err(CJError::PageNotFound { level: 0, page_id: 99 })));
}

#[test]
fn test_sync_delta_report_container_not_found() {
    let cfg = get_test_config();
    let journal = Journal::new(cfg).expect("journal init");
    let now = Utc::now();
    let res = journal.get_delta_report("missing", now, now + chrono::Duration::seconds(1));
    assert!(matches!(res, Err(CJError::InvalidInput(_))));
}

#[test]
fn test_journal_new_invalid_storage() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.block_on(SHARED_TEST_ID_MUTEX.lock());
    reset_global_ids();

    let mut cfg = Config::default();
    cfg.storage.storage_type = StorageType::File;
    cfg.storage.base_path = "".to_string();
    let cfg_static: &'static Config = Box::leak(Box::new(cfg));
    let result = Journal::new(cfg_static);
    assert!(matches!(result, Err(CJError::InvalidInput(_))));
}

#[test]
fn test_sync_get_page_existing() {
    let temp_dir = tempdir().expect("tempdir");
    let mut cfg = Config::default();
    cfg.storage.storage_type = StorageType::File;
    cfg.storage.base_path = temp_dir.path().to_str().unwrap().to_string();
    cfg.compression.enabled = false;
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let _guard = rt.block_on(SHARED_TEST_ID_MUTEX.lock());
    reset_global_ids();
    let storage = rt
        .block_on(FileStorage::new(
            &cfg.storage.base_path,
            cfg.compression.clone(),
        ))
        .expect("fs new");
    let page = civicjournal_time::core::page::JournalPage::new_with_id(
        0,
        0,
        None,
        Utc::now(),
        &cfg,
    );
    rt.block_on(storage.store_page(&page)).expect("store page");

    let journal = Journal::new(Box::leak(Box::new(cfg))).expect("journal init");
    let loaded = journal.get_page(0, 0).expect("get page");
    assert_eq!(loaded.page_id, page.page_id);
    assert_eq!(loaded.level, page.level);
}

#[test]
fn test_sync_leaf_inclusion_proof_success() {
    let temp_dir = tempdir().expect("tempdir");
    let mut cfg = Config::default();
    cfg.storage.storage_type = StorageType::File;
    cfg.storage.base_path = temp_dir.path().to_str().unwrap().to_string();
    cfg.compression.enabled = false;
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let _guard = rt.block_on(SHARED_TEST_ID_MUTEX.lock());
    reset_global_ids();

    let storage = rt
        .block_on(FileStorage::new(&cfg.storage.base_path, cfg.compression.clone()))
        .expect("fs new");

    let t0 = Utc::now();
    let leaf1 = JournalLeaf::new(t0, None, "c1".into(), json!({"a":1})).unwrap();
    let leaf2 = JournalLeaf::new(t0, Some(leaf1.leaf_hash), "c1".into(), json!({"b":2})).unwrap();
    let mut page = JournalPage::new(0, None, t0, &cfg);
    if let PageContent::Leaves(ref mut v) = page.content { v.push(leaf1.clone()); v.push(leaf2.clone()); }
    page.recalculate_merkle_root_and_page_hash();
    rt.block_on(storage.store_page(&page)).expect("store page");

    let journal = Journal::new(Box::leak(Box::new(cfg))).expect("journal init");
    let proof = journal.get_leaf_inclusion_proof(&leaf2.leaf_hash).expect("proof");
    assert_eq!(proof.leaf.leaf_hash, leaf2.leaf_hash);
    assert_eq!(proof.page_id, page.page_id);
    assert_eq!(proof.level, page.level);
}

