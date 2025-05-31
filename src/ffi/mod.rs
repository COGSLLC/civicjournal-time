// src/ffi/mod.rs

/// C-compatible Foreign Function Interface for CivicJournal.
pub mod c_ffi;

// Conditionally compile wasm_ffi module only for wasm32 target
#[cfg(target_arch = "wasm32")]
pub mod wasm_ffi;

// FFI functions will typically be defined here or in sub-modules
// and marked with #[no_mangle]
