use civicjournal_time::api::sync_api::Journal;
use civicjournal_time::config::Config;
use civicjournal_time::test_utils::get_test_config;
use civicjournal_time::error::CJError;
use chrono::Utc;

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

