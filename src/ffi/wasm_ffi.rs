// src/ffi/wasm_ffi.rs

// This file will contain WebAssembly bindings, likely using wasm-bindgen.

// Conditional compilation for wasm target
#[cfg(target_arch = "wasm32")]
mod wasm_bindings {
    use crate::api::async_api::{Journal as AsyncJournal, PageContentHash};
    use crate::config::Config;
    use chrono::{DateTime, NaiveDateTime, Utc};
    use hex;
    use serde_json::Value;
    use toml;
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

    /// Return the crate version as a convenience for JavaScript callers.
    #[wasm_bindgen]
    pub fn journal_version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Obtain the default configuration as a TOML string.
    #[wasm_bindgen]
    pub fn default_config() -> String {
        let cfg = crate::config::Config::default();
        toml::to_string(&cfg).unwrap_or_default()
    }

    /// WASM wrapper around the asynchronous `Journal` API.
    #[wasm_bindgen]
    pub struct WasmJournal {
        inner: AsyncJournal,
    }

    #[wasm_bindgen]
    impl WasmJournal {
        /// Create a new journal from an optional TOML configuration string.
        #[wasm_bindgen(constructor)]
        pub async fn new(config_toml: Option<String>) -> Result<WasmJournal, JsValue> {
            let cfg = if let Some(cfg_str) = config_toml {
                let mut cfg: Config = toml::from_str(&cfg_str)
                    .map_err(|e| JsValue::from_str(&format!("Config parse error: {}", e)))?;
                cfg.apply_env_vars()
                    .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
                Box::leak(Box::new(cfg))
            } else {
                Box::leak(Box::new(Config::default()))
            };
            let journal = AsyncJournal::new(cfg)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            Ok(WasmJournal { inner: journal })
        }

        /// Append a JSON payload to the journal. Timestamp is milliseconds since epoch.
        #[wasm_bindgen]
        pub async fn append_leaf(
            &self,
            timestamp_ms: i64,
            container_id: String,
            payload_json: String,
        ) -> Result<String, JsValue> {
            let naive = NaiveDateTime::from_timestamp_millis(timestamp_ms)
                .ok_or_else(|| JsValue::from_str("invalid timestamp"))?;
            let ts = DateTime::<Utc>::from_utc(naive, Utc);
            let data: Value = serde_json::from_str(&payload_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid JSON: {}", e)))?;
            let res = self
                .inner
                .append_leaf(ts, None, container_id, data)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            let hash = match res {
                PageContentHash::LeafHash(h) => h,
                PageContentHash::ThrallPageHash(h) => h,
            };
            Ok(hex::encode(hash))
        }

        /// Retrieve a page and return it as a serialized JSON value.
        #[wasm_bindgen]
        pub async fn get_page(&self, level: u8, page_id: u64) -> Result<JsValue, JsValue> {
            let page = self
                .inner
                .get_page(level, page_id)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            JsValue::from_serde(&page)
                .map_err(|e| JsValue::from_str(&format!("Serialize error: {}", e)))
        }

        /// Reconstruct container state at a timestamp (milliseconds since epoch).
        #[wasm_bindgen]
        pub async fn reconstruct_container_state(
            &self,
            container_id: String,
            at_ms: i64,
        ) -> Result<JsValue, JsValue> {
            let naive = NaiveDateTime::from_timestamp_millis(at_ms)
                .ok_or_else(|| JsValue::from_str("invalid timestamp"))?;
            let at = DateTime::<Utc>::from_utc(naive, Utc);
            let state = self
                .inner
                .reconstruct_container_state(&container_id, at)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            JsValue::from_serde(&state)
                .map_err(|e| JsValue::from_str(&format!("Serialize error: {}", e)))
        }

        /// Get an inclusion proof for a leaf hash (hex encoded).
        #[wasm_bindgen]
        pub async fn get_leaf_inclusion_proof(
            &self,
            leaf_hash_hex: String,
        ) -> Result<JsValue, JsValue> {
            let bytes = hex::decode(&leaf_hash_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid hex: {}", e)))?;
            let hash: [u8; 32] = bytes
                .try_into()
                .map_err(|_| JsValue::from_str("hash must be 32 bytes"))?;
            let proof = self
                .inner
                .get_leaf_inclusion_proof(&hash)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            JsValue::from_serde(&proof)
                .map_err(|e| JsValue::from_str(&format!("Serialize error: {}", e)))
        }

        /// Inclusion proof with a page hint (level,page_id) to speed up lookup.
        #[wasm_bindgen]
        pub async fn get_leaf_inclusion_proof_with_hint(
            &self,
            leaf_hash_hex: String,
            hint_level: Option<u8>,
            hint_page: Option<u64>,
        ) -> Result<JsValue, JsValue> {
            let bytes = hex::decode(&leaf_hash_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid hex: {}", e)))?;
            let hash: [u8; 32] = bytes
                .try_into()
                .map_err(|_| JsValue::from_str("hash must be 32 bytes"))?;
            let hint = match (hint_level, hint_page) {
                (Some(l), Some(p)) => Some((l, p)),
                _ => None,
            };
            let proof = self
                .inner
                .get_leaf_inclusion_proof_with_hint(&hash, hint)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            JsValue::from_serde(&proof)
                .map_err(|e| JsValue::from_str(&format!("Serialize error: {}", e)))
        }

        /// Get changes for a container in a time range (milliseconds since epoch).
        #[wasm_bindgen]
        pub async fn get_delta_report(
            &self,
            container_id: String,
            from_ms: i64,
            to_ms: i64,
        ) -> Result<JsValue, JsValue> {
            let from = NaiveDateTime::from_timestamp_millis(from_ms)
                .ok_or_else(|| JsValue::from_str("invalid from timestamp"))?;
            let to = NaiveDateTime::from_timestamp_millis(to_ms)
                .ok_or_else(|| JsValue::from_str("invalid to timestamp"))?;
            let from = DateTime::<Utc>::from_utc(from, Utc);
            let to = DateTime::<Utc>::from_utc(to, Utc);
            let report = self
                .inner
                .get_delta_report(&container_id, from, to)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            JsValue::from_serde(&report)
                .map_err(|e| JsValue::from_str(&format!("Serialize error: {}", e)))
        }

        /// Paginated delta report for large ranges.
        #[wasm_bindgen]
        pub async fn get_delta_report_paginated(
            &self,
            container_id: String,
            from_ms: i64,
            to_ms: i64,
            offset: usize,
            limit: usize,
        ) -> Result<JsValue, JsValue> {
            let from = NaiveDateTime::from_timestamp_millis(from_ms)
                .ok_or_else(|| JsValue::from_str("invalid from timestamp"))?;
            let to = NaiveDateTime::from_timestamp_millis(to_ms)
                .ok_or_else(|| JsValue::from_str("invalid to timestamp"))?;
            let from = DateTime::<Utc>::from_utc(from, Utc);
            let to = DateTime::<Utc>::from_utc(to, Utc);
            let report = self
                .inner
                .get_delta_report_paginated(&container_id, from, to, offset, limit)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            JsValue::from_serde(&report)
                .map_err(|e| JsValue::from_str(&format!("Serialize error: {}", e)))
        }

        /// Verify integrity of a range of pages.
        #[wasm_bindgen]
        pub async fn get_page_chain_integrity(
            &self,
            level: u8,
            from: Option<u64>,
            to: Option<u64>,
        ) -> Result<JsValue, JsValue> {
            let reports = self
                .inner
                .get_page_chain_integrity(level, from, to)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            JsValue::from_serde(&reports)
                .map_err(|e| JsValue::from_str(&format!("Serialize error: {}", e)))
        }

        /// Force application of retention policies (useful in tests/demo).
        #[wasm_bindgen]
        pub async fn apply_retention_policies(&self) -> Result<(), JsValue> {
            self.inner
                .apply_retention_policies()
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))
        }

        /// Create a snapshot and return its page hash as hex.
        #[wasm_bindgen]
        pub async fn create_snapshot(
            &self,
            as_of_ms: i64,
            container_ids: Option<js_sys::Array>,
        ) -> Result<String, JsValue> {
            let as_of = NaiveDateTime::from_timestamp_millis(as_of_ms)
                .ok_or_else(|| JsValue::from_str("invalid timestamp"))?;
            let as_of = DateTime::<Utc>::from_utc(as_of, Utc);
            let ids = container_ids.map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_string())
                    .collect::<Vec<String>>()
            });
            let hash = self
                .inner
                .create_snapshot(as_of, ids)
                .await
                .map_err(|e| JsValue::from_str(&format!("{}", e)))?;
            Ok(hex::encode(hash))
        }
    }
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

#[cfg(target_arch = "wasm32")]
pub use wasm_bindings::*;
