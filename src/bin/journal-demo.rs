#![cfg(feature = "demo")]
use clap::{Parser, Subcommand, ValueEnum};
use civicjournal_time::demo::{DemoConfig, simulator::Simulator, explorer};
use chrono::{DateTime, Utc};
use civicjournal_time::api::async_api::Journal;
use civicjournal_time::init;
use civicjournal_time::CJResult;

#[derive(Parser)]
#[command(author, version, about="CivicJournal Demo Mode", long_about=None)]
struct Cli {
    /// Path to configuration file (Journal.toml)
    #[arg(long, default_value="Journal.toml")]
    config: String,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the simulator
    Run {
        #[arg(long, value_enum, default_value="batch")]
        mode: Mode,
        #[arg(long, default_value_t=false)]
        wipe: bool,
    },
    /// Explore reconstructed state
    Explore {
        #[arg(long)]
        container: String,
        #[arg(long)]
        at: String,
    },
}

#[derive(Clone, ValueEnum)]
enum Mode {
    Batch,
    Live,
}

#[tokio::main]
async fn main() -> CJResult<()> {
    let cli = Cli::parse();
    let config = init(Some(&cli.config))?;
    let demo_cfg = DemoConfig::load(&cli.config)?;
    match cli.command {
        Commands::Run { mode, wipe } => {
            let journal = Journal::new(config).await?;
            let live = matches!(mode, Mode::Live);
            let mut sim = Simulator::new(demo_cfg, journal, wipe, live).await?;
            sim.run().await?;
        }
        Commands::Explore { container, at } => {
            let journal = Journal::new(config).await?;
            let at_dt: DateTime<Utc> = at.parse::<DateTime<Utc>>().map_err(|e| civicjournal_time::CJError::new(e.to_string()))?;
            explorer::serve_explorer(&journal, &container, at_dt).await?;
        }
    }
    Ok(())
}
