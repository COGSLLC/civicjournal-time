#![cfg(feature = "async_api")]

use civicjournal_time::ffi::c_ffi::*;
use std::ffi::{CString, CStr};
use libc::{c_char, c_int};

#[test]
fn test_ffi_append_confirm_get_latest() {
    let cfg = CString::new("{}").unwrap();
    let client = unsafe { cjt_init(cfg.as_ptr()) };
    assert!(!client.is_null());

    let payload = CString::new("{\"foo\":\"bar\"}").unwrap();
    let mut ticket_buf = [0 as c_char; 65];
    let rc = unsafe { cjt_turnstile_append(client, payload.as_ptr(), 0, ticket_buf.as_mut_ptr()) };
    assert_eq!(rc, 0);
    let ticket = unsafe { CStr::from_ptr(ticket_buf.as_ptr()) }.to_str().unwrap().to_string();

    let ticket_c = CString::new(ticket.clone()).unwrap();
    let rc = unsafe { cjt_confirm_ticket(client, ticket_c.as_ptr(), 1, std::ptr::null()) };
    assert_eq!(rc, 0);

    let mut latest_buf = [0 as c_char; 65];
    let rc = unsafe { cjt_get_latest_leaf_hash(client, latest_buf.as_mut_ptr()) };
    assert_eq!(rc, 0);
    let latest = unsafe { CStr::from_ptr(latest_buf.as_ptr()) }.to_str().unwrap();
    assert_eq!(latest, ticket);

    unsafe { cjt_destroy(client) };
}

extern "C" fn fail_callback(_prev: *const c_char, _payload: *const c_char, _hash: *const c_char) -> c_int {
    0
}

#[test]
fn test_ffi_retry_flow() {
    let cfg = CString::new("{\"max_retries\":1}").unwrap();
    let client = unsafe { cjt_init(cfg.as_ptr()) };
    assert!(!client.is_null());

    let payload = CString::new("{\"x\":1}").unwrap();
    let mut ticket_buf = [0 as c_char; 65];
    let rc = unsafe { cjt_turnstile_append(client, payload.as_ptr(), 0, ticket_buf.as_mut_ptr()) };
    assert_eq!(rc, 0);

    let rc = unsafe { cjt_retry_next_pending(client, Some(fail_callback)) };
    assert_eq!(rc, 2);

    unsafe { cjt_destroy(client) };
}

#[test]
fn test_ffi_orphan_logging_disabled() {
    let cfg = CString::new("{\"orphan_event_logging\":false}").unwrap();
    let client = unsafe { cjt_init(cfg.as_ptr()) };
    assert!(!client.is_null());

    let payload = CString::new("{\"x\":1}").unwrap();
    let mut ticket_buf = [0 as c_char; 65];
    let rc = unsafe { cjt_turnstile_append(client, payload.as_ptr(), 0, ticket_buf.as_mut_ptr()) };
    assert_eq!(rc, 0);
    let ticket = unsafe { CStr::from_ptr(ticket_buf.as_ptr()) }.to_str().unwrap().to_string();

    let err = CString::new("fail").unwrap();
    let t_c = CString::new(ticket.clone()).unwrap();
    let rc = unsafe { cjt_confirm_ticket(client, t_c.as_ptr(), 0, err.as_ptr()) };
    assert_eq!(rc, 0);

    let mut latest_buf = [0 as c_char; 65];
    unsafe { cjt_get_latest_leaf_hash(client, latest_buf.as_mut_ptr()) };
    let latest = unsafe { CStr::from_ptr(latest_buf.as_ptr()) }.to_str().unwrap();
    assert_eq!(latest, "0000000000000000000000000000000000000000000000000000000000000000");

    unsafe { cjt_destroy(client) };
}

#[test]
fn test_ffi_leaf_exists() {
    let cfg = CString::new("{}").unwrap();
    let client = unsafe { cjt_init(cfg.as_ptr()) };
    assert!(!client.is_null());

    let payload = CString::new("{\"a\":1}").unwrap();
    let mut ticket_buf = [0 as c_char; 65];
    let rc = unsafe { cjt_turnstile_append(client, payload.as_ptr(), 0, ticket_buf.as_mut_ptr()) };
    assert_eq!(rc, 0);
    let ticket = unsafe { CStr::from_ptr(ticket_buf.as_ptr()) }.to_str().unwrap().to_string();

    let t_c = CString::new(ticket.clone()).unwrap();
    let rc = unsafe { cjt_confirm_ticket(client, t_c.as_ptr(), 1, std::ptr::null()) };
    assert_eq!(rc, 0);

    let t_c = CString::new(ticket.clone()).unwrap();
    let rc = unsafe { cjt_leaf_exists(client, t_c.as_ptr()) };
    assert_eq!(rc, 1);

    let fake = CString::new("f".repeat(64)).unwrap();
    let rc = unsafe { cjt_leaf_exists(client, fake.as_ptr()) };
    assert_eq!(rc, 0);

    unsafe { cjt_destroy(client) };
}

#[test]
fn test_ffi_pending_count() {
    let cfg = CString::new("{}").unwrap();
    let client = unsafe { cjt_init(cfg.as_ptr()) };
    assert!(!client.is_null());

    let p1 = CString::new("{\"a\":1}").unwrap();
    let p2 = CString::new("{\"b\":2}").unwrap();
    let mut buf1 = [0 as c_char; 65];
    let mut buf2 = [0 as c_char; 65];
    unsafe { cjt_turnstile_append(client, p1.as_ptr(), 0, buf1.as_mut_ptr()) };
    unsafe { cjt_turnstile_append(client, p2.as_ptr(), 0, buf2.as_mut_ptr()) };

    let count = unsafe { cjt_pending_count(client) };
    assert_eq!(count, 2);

    let t1 = unsafe { CStr::from_ptr(buf1.as_ptr()) }.to_str().unwrap();
    let t2 = unsafe { CStr::from_ptr(buf2.as_ptr()) }.to_str().unwrap();
    let t1_c = CString::new(t1).unwrap();
    let t2_c = CString::new(t2).unwrap();
    unsafe { cjt_confirm_ticket(client, t1_c.as_ptr(), 1, std::ptr::null()) };
    let count = unsafe { cjt_pending_count(client) };
    assert_eq!(count, 1);
    unsafe { cjt_confirm_ticket(client, t2_c.as_ptr(), 1, std::ptr::null()) };
    let count = unsafe { cjt_pending_count(client) };
    assert_eq!(count, 0);

    unsafe { cjt_destroy(client) };
}

#[test]
fn test_ffi_list_pending() {
    let cfg = CString::new("{}").unwrap();
    let client = unsafe { cjt_init(cfg.as_ptr()) };
    assert!(!client.is_null());

    let p1 = CString::new("{\"a\":1}").unwrap();
    let p2 = CString::new("{\"b\":2}").unwrap();
    let p3 = CString::new("{\"c\":3}").unwrap();
    let mut b1 = [0 as c_char; 65];
    let mut b2 = [0 as c_char; 65];
    let mut b3 = [0 as c_char; 65];
    unsafe { cjt_turnstile_append(client, p1.as_ptr(), 0, b1.as_mut_ptr()) };
    unsafe { cjt_turnstile_append(client, p2.as_ptr(), 0, b2.as_mut_ptr()) };
    unsafe { cjt_turnstile_append(client, p3.as_ptr(), 0, b3.as_mut_ptr()) };

    let t1 = unsafe { CStr::from_ptr(b1.as_ptr()) }.to_str().unwrap().to_string();
    let t2 = unsafe { CStr::from_ptr(b2.as_ptr()) }.to_str().unwrap().to_string();
    let t3 = unsafe { CStr::from_ptr(b3.as_ptr()) }.to_str().unwrap().to_string();

    let mut out: [[c_char; 65]; 3] = [[0; 65]; 3];
    let rc = unsafe { cjt_list_pending(client, out.as_mut_ptr(), 2) };
    assert_eq!(rc, 2);
    let res1 = unsafe { CStr::from_ptr(out[0].as_ptr()) }.to_str().unwrap().to_string();
    let res2 = unsafe { CStr::from_ptr(out[1].as_ptr()) }.to_str().unwrap().to_string();
    let results = vec![res1, res2];
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|h| h == &t1 || h == &t2 || h == &t3));

    let t1_c = CString::new(t1).unwrap();
    unsafe { cjt_confirm_ticket(client, t1_c.as_ptr(), 1, std::ptr::null()) };

    let mut out2: [[c_char; 65]; 3] = [[0; 65]; 3];
    let rc = unsafe { cjt_list_pending(client, out2.as_mut_ptr(), 3) };
    assert_eq!(rc, 2);
    let r1 = unsafe { CStr::from_ptr(out2[0].as_ptr()) }.to_str().unwrap().to_string();
    let r2 = unsafe { CStr::from_ptr(out2[1].as_ptr()) }.to_str().unwrap().to_string();
    let remaining = vec![r1, r2];
    assert_eq!(remaining.len(), 2);
    assert!(remaining.contains(&t2));
    assert!(remaining.contains(&t3));

    unsafe { cjt_destroy(client) };
}
