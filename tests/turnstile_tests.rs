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
    let rc = ts.retry_next_pending(|_, _, _| 0).expect("retry");
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
fn test_confirm_ticket_not_found() {
    let mut ts = Turnstile::new("00".repeat(32), 1);
    let res = ts.confirm_ticket(&"ff".repeat(64), true, None);
    assert!(matches!(
        res.unwrap_err(),
        civicjournal_time::error::CJError::NotFound(_)
    ));
}

#[test]
fn test_retry_next_pending_success() {
    let mut ts = Turnstile::new("00".repeat(32), 1);
    let ticket = ts.append("{\"a\":1}", 0).unwrap();
    let rc = ts.retry_next_pending(|_, _, _| 1).unwrap();
    assert_eq!(rc, 0);
    assert_eq!(ts.pending_count(), 0);
    assert_eq!(ts.success_events().len(), 1);
    let ev = &ts.success_events()[0];
    assert_eq!(ev.orig_hash, ticket);
    assert_eq!(ts.latest_leaf_hash(), ev.leaf_hash);
    assert!(ts.leaf_exists(&ticket).unwrap());
}

#[test]
fn test_confirm_failure_then_retry_success_logs_event() {
    let mut ts = Turnstile::new("00".repeat(32), 2);
    let ticket = ts.append("{\"b\":1}", 1).unwrap();
    ts.confirm_ticket(&ticket, false, Some("db down")).unwrap();
    let rc = ts.retry_next_pending(|_, _, _| 1).unwrap();
    assert_eq!(rc, 0);
    assert_eq!(ts.success_events().len(), 1);
    assert_eq!(ts.success_events()[0].orig_hash, ticket);
}

#[test]
fn test_retry_next_pending_no_entries() {
    let mut ts = Turnstile::new("00".repeat(32), 1);
    let rc = ts.retry_next_pending(|_, _, _| 1).unwrap();
    assert_eq!(rc, -1);
}

#[test]
fn test_retry_next_pending_hash_mismatch() {
    use serde_json::Value;
    use std::fs;

    let dir = tempdir().unwrap();
    let path = dir.path().join("ts");
    let mut ts = Turnstile::new_with_storage("00".repeat(32), 1, Some(path.clone()), true);
    let ticket = ts.append("{\"a\":1}", 0).unwrap();

    // tamper with persisted payload
    let state_file = path.join("turnstile_state.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&state_file).unwrap()).unwrap();
    state["pending"][&ticket]["payload_json"] = Value::String("{\"a\":2}".into());
    fs::write(&state_file, serde_json::to_vec(&state).unwrap()).unwrap();

    // reload from disk so the modified payload is used
    let mut ts = Turnstile::new_with_storage("irrelevant".into(), 1, Some(path), true);

    let rc = ts.retry_next_pending(|_, _, _| 1).unwrap();
    assert_eq!(rc, -2);
    assert_eq!(ts.pending_count(), 0);
    assert!(ts.leaf_exists(&ticket).unwrap());
    assert!(ts.orphan_events().is_empty());
}

#[test]
fn test_list_pending_respects_max() {
    let mut ts = Turnstile::new("00".repeat(32), 3);
    let t1 = ts.append("{\"a\":1}", 0).unwrap();
    let t2 = ts.append("{\"b\":2}", 1).unwrap();
    let t3 = ts.append("{\"c\":3}", 2).unwrap();
    let list = ts.list_pending(2);
    assert_eq!(list.len(), 2);
    assert!(list.contains(&t1) || list.contains(&t2) || list.contains(&t3));
}

#[test]
fn test_retry_next_pending_failure_logs_orphan() {
    let mut ts = Turnstile::new("00".repeat(32), 2);
    let ticket = ts.append("{\"a\":1}", 42).unwrap();
    let prev = ts.latest_leaf_hash();
    let rc = ts.retry_next_pending(|_, _, _| 0).unwrap();
    assert_eq!(rc, 1);
    assert_eq!(ts.pending_count(), 1);
    assert_ne!(ts.latest_leaf_hash(), prev);
    assert_eq!(ts.orphan_events().len(), 1);
    let orphan = &ts.orphan_events()[0];
    assert_eq!(orphan.orig_hash, ticket);
    assert_eq!(orphan.error_msg, "retry failed");
    assert_eq!(orphan.timestamp, 42);
}

#[test]
fn test_retry_next_pending_exceeds_max_retries() {
    let mut ts = Turnstile::new("00".repeat(32), 1);
    let ticket = ts.append("{\"a\":1}", 5).unwrap();
    let rc = ts.retry_next_pending(|_, _, _| 0).unwrap();
    assert_eq!(rc, 2);
    assert_eq!(ts.pending_count(), 0);
    assert!(ts.leaf_exists(&ticket).unwrap());
    assert_eq!(ts.orphan_events().len(), 1);
    let orphan = &ts.orphan_events()[0];
    assert_eq!(orphan.orig_hash, ticket);
    assert_eq!(orphan.error_msg, "retry failed");
    assert_eq!(orphan.timestamp, 5);
}

#[test]
fn test_leaf_exists_checks_committed_and_pending() {
    let mut ts = Turnstile::new("00".repeat(32), 1);
    let ticket = ts.append("{\"x\":1}", 0).unwrap();
    assert!(ts.leaf_exists(&ticket).unwrap());
    ts.confirm_ticket(&ticket, true, None).unwrap();
    assert!(ts.leaf_exists(&ticket).unwrap());
    assert!(!ts.leaf_exists(&"aa".repeat(32)).unwrap());
}

#[test]
fn test_append_invalid_json_and_prev_hash() {
    // invalid JSON should produce an error
    let mut ts = Turnstile::new("00".repeat(32), 3);
    assert!(ts.append("{", 0).is_err());

    // invalid previous hash also results in error
    let mut ts = Turnstile::new("zz".into(), 3);
    assert!(ts.append("{\"a\":1}", 0).is_err());
}

#[test]
fn test_confirm_ticket_failure_marks_permanent_after_retries() {
    use serde_json::Value;
    use std::fs;

    let dir = tempdir().unwrap();
    let path = dir.path().join("ts");
    let mut ts = Turnstile::new_with_storage("00".repeat(32), 1, Some(path.clone()), true);
    let ticket = ts.append("{\"a\":1}", 99).unwrap();

    // first failure - still pending
    ts.confirm_ticket(&ticket, false, Some("e1")).unwrap();
    let state_file = path.join("turnstile_state.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&state_file).unwrap()).unwrap();
    let entry = &state["pending"][&ticket];
    assert_eq!(entry["retry_count"], 1);
    assert_eq!(entry["status"], "Pending");
    assert_eq!(entry["last_error"], "e1");
    assert_eq!(ts.orphan_events().len(), 1);
    let orphan1 = &ts.orphan_events()[0];
    assert_eq!(orphan1.orig_hash, ticket);
    assert_eq!(orphan1.error_msg, "e1");
    assert_eq!(orphan1.timestamp, 99);

    // second failure exceeds max_retries
    ts.confirm_ticket(&ticket, false, Some("e2")).unwrap();
    state = serde_json::from_slice(&fs::read(&state_file).unwrap()).unwrap();
    let entry = &state["pending"][&ticket];
    assert_eq!(entry["retry_count"], 2);
    assert_eq!(entry["status"], "FailedPermanent");
    assert_eq!(entry["last_error"], "e2");
    assert_eq!(ts.orphan_events().len(), 2);
    let orphan2 = &ts.orphan_events()[1];
    assert_eq!(orphan2.orig_hash, ticket);
    assert_eq!(orphan2.error_msg, "e2");
    assert_eq!(orphan2.timestamp, 99);
}

#[test]
fn test_append_thousand_events_under_minute() {
    use std::time::Instant;

    let mut ts = Turnstile::new("00".repeat(32), 1);
    let start = Instant::now();
    for i in 0..1000u32 {
        let payload = format!("{{\"n\":{}}}", i);
        let ticket = ts.append(&payload, i as u64).unwrap();
        ts.confirm_ticket(&ticket, true, None).unwrap();
    }
    let elapsed = start.elapsed();
    println!("processed 1000 events in {:?}", elapsed);
    assert!(elapsed.as_secs() < 60);
    assert_eq!(ts.pending_count(), 0);
}
