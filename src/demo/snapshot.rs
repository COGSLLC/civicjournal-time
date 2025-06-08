#![cfg(feature = "demo")]

use crate::CJResult;
use crate::api::async_api::Journal;
use chrono::{DateTime, Utc};

/// Placeholder for snapshot helpers.
pub async fn create_snapshot(
    journal: &Journal,
    as_of: DateTime<Utc>,
    container_ids: Option<Vec<String>>,
) -> CJResult<[u8; 32]> {
    journal.create_snapshot(as_of, container_ids).await
}
