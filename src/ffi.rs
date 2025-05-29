use std::os::raw::c_char;
use std::ffi::{CStr, CString};
use std::ptr;
use libc::c_void;

use crate::time_hierarchy::TimeHierarchy;

/// Opaque handle to a TimeHierarchy instance
#[repr(C)]
pub struct CJTimeHierarchy(*mut c_void);

/// Error codes for C API functions
#[repr(C)]
pub enum CJErrorCode {
    /// No error
    Success = 0,
    /// Generic error
    Error = 1,
    /// I/O error
    IoError = 2,
    /// Serialization error
    SerializationError = 3,
    /// Verification error
    VerificationError = 4,
}

/// Creates a new TimeHierarchy instance
///
/// # Safety
/// The returned pointer must be freed with `civicjournal_time_hierarchy_free`
#[no_mangle]
pub unsafe extern "C" fn civicjournal_time_hierarchy_new() -> *mut CJTimeHierarchy {
    let hierarchy = Box::new(TimeHierarchy::default());
    Box::into_raw(Box::new(CJTimeHierarchy(Box::into_raw(Box::new(hierarchy)) as *mut _)))
}

/// Frees a TimeHierarchy instance
///
/// # Safety
/// The pointer must be valid and not used after this call
#[no_mangle]
pub unsafe extern "C" fn civicjournal_time_hierarchy_free(ptr: *mut CJTimeHierarchy) {
    if ptr.is_null() {
        return;
    }
    let _ = Box::from_raw((*ptr).0 as *mut TimeHierarchy);
    let _ = Box::from_raw(ptr);
}

/// Gets the current time hierarchy statistics as a JSON string
///
/// Returns a newly allocated string that must be freed with `civicjournal_free_string`
#[no_mangle]
pub unsafe extern "C" fn civicjournal_time_hierarchy_stats(
    hierarchy: *const CJTimeHierarchy,
) -> *mut c_char {
    if hierarchy.is_null() {
        return ptr::null_mut();
    }

    let hierarchy = &*((*hierarchy).0 as *const TimeHierarchy);
    match serde_json::to_string(&hierarchy.calculate_stats()) {
        Ok(s) => CString::new(s).unwrap().into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Frees a string that was allocated by the library
///
/// # Safety
/// The string must have been allocated by this library and not already freed
#[no_mangle]
pub unsafe extern "C" fn civicjournal_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    let _ = CString::from_raw(s);
}

/// Gets an error message for an error code
///
/// # Safety
/// The string must be freed with `civicjournal_free_string`
#[no_mangle]
pub unsafe extern "C" fn civicjournal_get_error_message(error_code: CJErrorCode) -> *mut c_char {
    let message = match error_code {
        CJErrorCode::Success => "No error",
        CJErrorCode::Error => "Generic error",
        CJErrorCode::IoError => "I/O error occurred",
        CJErrorCode::SerializationError => "Serialization error occurred",
        CJErrorCode::VerificationError => "Verification error occurred",
    };
    
    CString::new(message)
        .unwrap()
        .into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn test_ffi_basics() {
        unsafe {
            let hierarchy = civicjournal_time_hierarchy_new();
            assert!(!hierarchy.is_null());
            
            // Test getting stats
            let stats_json = civicjournal_time_hierarchy_stats(hierarchy);
            assert!(!stats_json.is_null());
            let stats_str = CStr::from_ptr(stats_json).to_str().unwrap();
            assert!(stats_str.contains("chunk_count"));
            
            // Clean up
            civicjournal_free_string(stats_json);
            civicjournal_time_hierarchy_free(hierarchy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn test_ffi_basics() {
        unsafe {
            let hierarchy = civicjournal_time_hierarchy_new();
            assert!(!hierarchy.is_null());
            
            // Test getting stats
            let stats_json = civicjournal_time_hierarchy_stats(hierarchy);
            assert!(!stats_json.is_null());
            let stats_str = CStr::from_ptr(stats_json).to_str().unwrap();
            assert!(stats_str.contains("chunk_count"));
            
            // Clean up
            civicjournal_free_string(stats_json);
            civicjournal_time_hierarchy_free(hierarchy);
        }
    }
}
