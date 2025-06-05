// src/ffi/c_ffi.rs

// This file will contain C-compatible FFI bindings.
// Functions exposed here should use C-compatible types (e.g., from libc crate)
// and be marked with #[no_mangle] and extern "C".

/*
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use libc;

use crate::api::sync_api::SyncApi; // Example: using the sync API
// Or a global/static instance of the journal API if appropriate

/// Example FFI function: Initializes the journal system.
/// Returns 0 on success, non-zero on error.
#[no_mangle]
pub extern "C" fn civicjournal_init() -> libc::c_int {
    // Initialize your journal instance here
    // This might involve setting up storage, etc.
    // For now, just a placeholder.
    println!("CivicJournal FFI: Initializing...");
    0 // Success
}

/// Example FFI function: Appends a delta.
/// Takes container_id and payload as C strings.
/// Returns a C string (e.g., leaf_id or error message) - caller must free.
#[no_mangle]
pub extern "C" fn civicjournal_append_delta(container_id_c: *const c_char, payload_c: *const c_char) -> *mut c_char {
    if container_id_c.is_null() || payload_c.is_null() {
        return CString::new("Error: Null input string").unwrap_or_default().into_raw();
    }

    let container_id = unsafe { CStr::from_ptr(container_id_c) }.to_str().unwrap_or_default();
    let payload_str = unsafe { CStr::from_ptr(payload_c) }.to_str().unwrap_or_default();

    // TODO: Parse payload_str into serde_json::Value
    // TODO: Call the actual journal API
    // let result = get_global_journal_api().append_delta(container_id, &payload_json_value);

    let response_message = format!("Appended to {}: {}", container_id, payload_str);
    CString::new(response_message).unwrap_or_default().into_raw()
}

/// Example FFI function: Frees a C string previously returned by an FFI function.
#[no_mangle]
pub extern "C" fn civicjournal_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(s));
    }
}
*/

use crate::turnstile::Turnstile;
use serde::Deserialize;
use std::path::PathBuf;
use libc::{c_char, c_int};
use std::ffi::{CStr, CString};

/// Opaque CJT client handle used across FFI calls.
#[repr(C)]
pub struct CJTClient {
    ts: Turnstile,
}

#[derive(Deserialize)]
struct InitConfig {
    storage_path: Option<String>,
    max_retries: Option<u32>,
    orphan_event_logging: Option<bool>,
}

/// Initialize a CJT client from JSON config.
#[no_mangle]
pub extern "C" fn cjt_init(config_json: *const c_char) -> *mut CJTClient {
    if config_json.is_null() {
        return std::ptr::null_mut();
    }
    let c_str = unsafe { CStr::from_ptr(config_json) };
    let config_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let cfg: InitConfig = serde_json::from_str(config_str).unwrap_or(InitConfig {
        storage_path: None,
        max_retries: None,
        orphan_event_logging: None,
    });
    let ts = Turnstile::new_with_storage(
        "00".repeat(32),
        cfg.max_retries.unwrap_or(5),
        cfg.storage_path.map(PathBuf::from),
        cfg.orphan_event_logging.unwrap_or(true),
    );
    Box::into_raw(Box::new(CJTClient { ts }))
}

/// Destroy a CJT client.
#[no_mangle]
pub extern "C" fn cjt_destroy(ptr: *mut CJTClient) {
    if ptr.is_null() {
        return;
    }
    unsafe { drop(Box::from_raw(ptr)); }
}

/// Append a payload and get back the ticket hash.
#[no_mangle]
pub extern "C" fn cjt_turnstile_append(
    ptr: *mut CJTClient,
    payload_json: *const c_char,
    timestamp: u64,
    out_ticket: *mut c_char,
) -> c_int {
    if ptr.is_null() || payload_json.is_null() || out_ticket.is_null() {
        return -1;
    }
    let ts = unsafe { &mut (*ptr).ts };
    let c_str = unsafe { CStr::from_ptr(payload_json) };
    match c_str.to_str() {
        Ok(payload) => match ts.append(payload, timestamp) {
            Ok(ticket) => {
                let bytes = ticket.as_bytes();
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ticket as *mut u8, bytes.len());
                    *out_ticket.add(64) = 0;
                }
                0
            }
            Err(_) => -2,
        },
        Err(_) => -3,
    }
}

/// Confirm or reject a pending ticket.
#[no_mangle]
pub extern "C" fn cjt_confirm_ticket(
    ptr: *mut CJTClient,
    leaf_hash: *const c_char,
    status: c_int,
    err_msg: *const c_char,
) -> c_int {
    if ptr.is_null() || leaf_hash.is_null() {
        return -1;
    }
    let ts = unsafe { &mut (*ptr).ts };
    let hash = unsafe { CStr::from_ptr(leaf_hash) }.to_string_lossy();
    let err = if !err_msg.is_null() {
        Some(unsafe { CStr::from_ptr(err_msg) }.to_string_lossy().to_string())
    } else {
        None
    };
    match ts.confirm_ticket(&hash, status != 0, err.as_deref()) {
        Ok(_) => 0,
        Err(_) => -2,
    }
}

/// Retry the next pending ticket using the provided callback.
#[no_mangle]
pub extern "C" fn cjt_retry_next_pending(
    ptr: *mut CJTClient,
    callback: Option<extern "C" fn(*const c_char, *const c_char, *const c_char) -> c_int>,
) -> c_int {
    if ptr.is_null() {
        return -1;
    }
    let ts = unsafe { &mut (*ptr).ts };
    let cb = match callback {
        Some(func) => func,
        None => return -1,
    };
    let rc = ts
        .retry_next_pending(|prev, payload, leaf| {
            let prev_c = CString::new(prev).unwrap();
            let payload_c = CString::new(payload).unwrap();
            let leaf_c = CString::new(leaf).unwrap();
            cb(prev_c.as_ptr(), payload_c.as_ptr(), leaf_c.as_ptr()) as i32
        })
        .unwrap_or(-3);
    rc
}

/// Check if a leaf exists.
#[no_mangle]
pub extern "C" fn cjt_leaf_exists(ptr: *mut CJTClient, leaf_hash: *const c_char) -> c_int {
    if ptr.is_null() || leaf_hash.is_null() {
        return -1;
    }
    let ts = unsafe { &mut (*ptr).ts };
    let hash = unsafe { CStr::from_ptr(leaf_hash) }.to_string_lossy();
    match ts.leaf_exists(&hash) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -2,
    }
}

/// Get latest committed leaf hash.
#[no_mangle]
pub extern "C" fn cjt_get_latest_leaf_hash(ptr: *mut CJTClient, out_hash: *mut c_char) -> c_int {
    if ptr.is_null() || out_hash.is_null() {
        return -1;
    }
    let ts = unsafe { &mut (*ptr).ts };
    let hash = ts.latest_leaf_hash();
    let bytes = hash.as_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_hash as *mut u8, bytes.len());
        *out_hash.add(64) = 0;
    }
    0
}

/// Get pending count.
#[no_mangle]
pub extern "C" fn cjt_pending_count(ptr: *mut CJTClient) -> c_int {
    if ptr.is_null() {
        return -1;
    }
    let ts = unsafe { &mut (*ptr).ts };
    ts.pending_count() as c_int
}

/// List pending hashes.
#[no_mangle]
pub extern "C" fn cjt_list_pending(
    ptr: *mut CJTClient,
    out_hashes: *mut [c_char; 65],
    max_entries: c_int,
) -> c_int {
    if ptr.is_null() || out_hashes.is_null() {
        return -1;
    }
    let ts = unsafe { &mut (*ptr).ts };
    let max = max_entries as usize;
    let hashes = ts.list_pending(max);
    let slice = unsafe { std::slice::from_raw_parts_mut(out_hashes, max) };
    for (i, h) in hashes.iter().enumerate() {
        let bytes = h.as_bytes();
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), slice[i].as_mut_ptr() as *mut u8, bytes.len());
            slice[i][64] = 0;
        }
    }
    hashes.len() as c_int
}

#[cfg(test)]
mod tests {
    

    #[test]
    fn it_works_c_ffi() {
        // FFI tests are more complex and often involve writing C code to call the Rust library.
        // For now, this is a placeholder.
    }
}
