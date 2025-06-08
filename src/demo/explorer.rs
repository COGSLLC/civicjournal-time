#![cfg(feature = "demo")]

use crate::CJResult;
use crate::api::async_api::Journal;
use chrono::{DateTime, Utc};
use serde_json::to_string_pretty;

/// Placeholder for CLI and HTTP exploration interfaces.
pub async fn serve_explorer(journal: &Journal, container_id: &str, at: DateTime<Utc>) -> CJResult<()> {
    let state = journal.reconstruct_container_state(container_id, at).await?;
    println!("{}", to_string_pretty(&state.state_data).unwrap_or_default());
    Ok(())
}
