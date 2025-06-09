#![cfg(feature = "demo")]
use civicjournal_time::demo_cli;
use civicjournal_time::CJResult;

#[tokio::main]
async fn main() -> CJResult<()> {
    demo_cli::run().await
}
