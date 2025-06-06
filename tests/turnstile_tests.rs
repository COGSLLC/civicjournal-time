use civicjournal_time::turnstile::Turnstile;
use tempfile::tempdir;

#[test]
fn test_turnstile_append_confirm_flow() {
    let mut ts = Turnstile::new("00".repeat(32), 3);
    let ticket = ts.append("{\"foo\":\"bar\"}", 0).unwrap();
    assert_eq!(
        ticket,
        "671fde0a1b5143bb2f28c9f08f5db13fed479b6313f56f8145260615d8cb05b4"
    );
    assert_eq!(ts.pending_count(), 1);
    ts.confirm_ticket(&ticket, true, None).unwrap();
    assert!(ts.leaf_exists(&ticket).unwrap());
    assert_eq!(ts.pending_count(), 0);
}

#[test]
fn test_turnstile_list_pending() {
    let mut ts = Turnstile::new("00".repeat(32), 3);
    let t1 = ts.append("{\"a\":1}", 1).unwrap();
    let t2 = ts.append("{\"b\":2}", 2).unwrap();
    let pending = ts.list_pending(10);
    assert_eq!(pending.len(), 2);
    assert!(pending.contains(&t1));
    assert!(pending.contains(&t2));
}

#[test]
fn test_turnstile_persistence() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ts");
    let mut ts = Turnstile::new_with_storage("00".repeat(32), 3, Some(path.clone()), true);
    let t1 = ts.append("{\"a\":1}", 1).unwrap();
    drop(ts);
    let ts2 = Turnstile::new_with_storage("ignored".to_string(), 3, Some(path), true);
    assert!(ts2.leaf_exists(&t1).unwrap());
    assert_eq!(ts2.pending_count(), 1);
}

#[test]
fn test_turnstile_retry_logic() {
    let mut ts = Turnstile::new("00".repeat(32), 1);
    ts.append("{\"x\":1}", 1).unwrap();
    let rc = ts
        .retry_next_pending(|_, _, _| 0)
        .expect("retry");
    assert_eq!(rc, 2); // exceeded retries
    assert_eq!(ts.pending_count(), 0);
}

#[test]
fn test_retry_succeeds_after_temporary_failure() {
    let mut ts = Turnstile::new("00".repeat(32), 2);
    ts.append("{\"x\":1}", 1).unwrap();

    // first retry fails, leaving the entry pending
    let rc = ts.retry_next_pending(|_, _, _| 0).expect("retry");
    assert_eq!(rc, 1);
    assert_eq!(ts.pending_count(), 1);

    // second retry succeeds and commits the entry
    let rc = ts.retry_next_pending(|_, _, _| 1).expect("retry");
    assert_eq!(rc, 0);
    assert_eq!(ts.pending_count(), 0);
}

#[test]
fn test_orphan_event_logged() {
    let mut ts = Turnstile::new("00".repeat(32), 3);
    let t = ts.append("{\"z\":1}", 10).unwrap();
    ts.confirm_ticket(&t, false, Some("db error")).unwrap();
    assert_eq!(ts.orphan_events().len(), 1);
    assert_eq!(ts.orphan_events()[0].orig_hash, t);
    assert_eq!(ts.latest_leaf_hash(), ts.orphan_events()[0].leaf_hash);
}

#[test]
fn test_orphan_logging_disabled() {
    let mut ts = Turnstile::new_with_storage("00".repeat(32), 3, None, false);
    let t = ts.append("{\"a\":1}", 0).unwrap();
    ts.confirm_ticket(&t, false, Some("err")).unwrap();
    assert_eq!(ts.orphan_events().len(), 0);
    let prev = ts.latest_leaf_hash();
    ts.append("{\"b\":2}", 1).unwrap();
    ts.retry_next_pending(|_, _, _| 0).unwrap();
    assert_eq!(ts.orphan_events().len(), 0);
    assert_eq!(ts.latest_leaf_hash(), prev);
}

#[test]
fn test_append_invalid_json_and_prev_hash() {
    // invalid JSON should produce an error
    let mut ts = Turnstile::new("00".repeat(32), 3);
    assert!(ts.append("{" , 0).is_err());

    // invalid previous hash also results in error
    let mut ts = Turnstile::new("zz".into(), 3);
    assert!(ts.append("{\"a\":1}", 0).is_err());
}
