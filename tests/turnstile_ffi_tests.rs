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
