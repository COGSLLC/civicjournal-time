// src/ffi/c_ffi.rs

//! Minimal C-compatible FFI exposing query functionality for Turnstile.

use libc;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;

use crate::api::sync_api::Journal;
use crate::config::Config;

static GLOBAL_JOURNAL: OnceLock<Journal> = OnceLock::new();

/// Initialize a default in-memory journal for FFI calls.
#[no_mangle]
pub extern "C" fn civicjournal_init_default() -> libc::c_int {
    let cfg = Box::leak(Box::new(Config::default()));
    match Journal::new(cfg) {
        Ok(j) => {
            let _ = GLOBAL_JOURNAL.set(j);
            0
        }
        Err(_) => 1,
    }
}

fn get_journal() -> Result<&'static Journal, CString> {
    GLOBAL_JOURNAL
        .get()
        .ok_or_else(|| CString::new("not_initialized").unwrap())
}

/// Retrieve container state as JSON at the given timestamp (seconds since epoch).
#[no_mangle]
pub extern "C" fn civicjournal_get_container_state(container_id: *const c_char, ts_secs: i64) -> *mut c_char {
    if container_id.is_null() {
        return CString::new("null_ptr").unwrap().into_raw();
    }
    let cid = unsafe { CStr::from_ptr(container_id) }.to_string_lossy();
    let ts = chrono::NaiveDateTime::from_timestamp_opt(ts_secs, 0)
        .unwrap_or_else(|| chrono::NaiveDateTime::from_timestamp(0, 0))
        .and_utc();
    match get_journal() {
        Ok(j) => match j.reconstruct_container_state(&cid, ts) {
            Ok(state) => CString::new(state.state_data.to_string()).unwrap().into_raw(),
            Err(e) => CString::new(e.to_string()).unwrap().into_raw(),
        },
        Err(e) => e.into_raw(),
    }
}

/// Free a string returned by the FFI.
#[no_mangle]
pub extern "C" fn civicjournal_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    unsafe { drop(CString::from_raw(s)); }
}

