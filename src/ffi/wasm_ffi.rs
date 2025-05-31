// src/ffi/wasm_ffi.rs

// This file will contain WebAssembly bindings, likely using wasm-bindgen.

// Conditional compilation for wasm target
#[cfg(target_arch = "wasm32")]
mod wasm_bindings {
    use wasm_bindgen::prelude::*;

    // When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
    // allocator.
    #[cfg(feature = "wee_alloc")]
    #[global_allocator]
    static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

    /// Example WASM function: A simple alert.
    #[wasm_bindgen]
    extern "C" {
        fn alert(s: &str);
    }

    /// Example WASM function: Greet function exposed to JavaScript.
    #[wasm_bindgen]
    pub fn greet(name: &str) {
        alert(&format!("Hello, {}! From CivicJournal via WASM!", name));
    }

    // TODO: Expose actual CivicJournal API functions via wasm-bindgen
    // - Functions for appending deltas, querying, etc.
    // - Need to handle serialization/deserialization between Rust and JS types (e.g., Serde JSValue).

    /*
    use crate::api::sync_api::SyncApi; // Or a global/static journal instance
    use serde_json;

    #[wasm_bindgen]
    pub fn wasm_append_delta(container_id: &str, payload_json_str: &str) -> Result<String, JsValue> {
        // Parse payload_json_str to serde_json::Value
        // Call the underlying CivicJournal API
        // Serialize result (e.g., leaf_id or error) back to a String or JsValue
        // Example:
        // let payload: serde_json::Value = serde_json::from_str(payload_json_str)
        //     .map_err(|e| JsValue::from_str(&format!("Payload deserialization error: {}", e)))?;
        // let journal_api = get_global_journal_api(); // Placeholder for getting API instance
        // let leaf = journal_api.append_delta(container_id, &payload)
        //     .map_err(|e| JsValue::from_str(&format!("Append error: {}", e)))?;
        // Ok(serde_json::to_string(&leaf).unwrap_or_default()) // Return leaf as JSON string
        Ok(format!("WASM: Appended to {}: {}", container_id, payload_json_str))
    }
    */
}

#[cfg(test)]
mod tests {
    use super::*;

    // WASM tests typically run in a browser environment or using tools like wasm-pack test.
    // For now, this is a placeholder.
    #[test]
    fn it_works_wasm_ffi() {
        // Placeholder
    }
}
