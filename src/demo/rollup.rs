#![cfg(feature = "demo")]

use crate::CJResult;
use crate::api::async_api::Journal;

/// Placeholder for demo rollup helpers.
pub async fn perform_rollup(journal: &Journal) -> CJResult<()> {
    journal.apply_retention_policies().await
}
