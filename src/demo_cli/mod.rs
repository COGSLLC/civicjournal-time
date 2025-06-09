#![cfg(feature = "demo")]

//! Scaffolding for the new `cj-demo` command line interface.
//!
//! Each subcommand is currently a stub. Implementation will follow
//! the specification in `DEMOMODE.md`.

use clap::{Parser, Subcommand};
use crate::{init, CJResult};
use crate::api::async_api::Journal;
use chrono::{DateTime, Utc, Duration};
use rand::{SeedableRng, Rng};
use rand::rngs::StdRng;
use fake::{Fake, faker::lorem::en::Sentence};
use serde_json::{json, Value};
use hex;

#[derive(Parser)]
#[command(author, version, about="CivicJournal Demo CLI", long_about=None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Simulate journal history
    Simulate {
        #[arg(long)]
        container: String,
        #[arg(long, default_value_t = 50)]
        fields: u32,
        #[arg(long)]
        duration: String,
        #[arg(long, default_value_t = 0)]
        errors_parked: u32,
        #[arg(long, default_value_t = 0)]
        errors_malformed: u32,
        #[arg(long)]
        start: String,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    /// Reconstruct state as of a timestamp
    State {
        #[arg(long)]
        container: String,
        #[arg(long)]
        as_of: String,
    },
    /// Revert a database back to a timestamp
    Revert {
        #[arg(long)]
        container: String,
        #[arg(long)]
        as_of: String,
        #[arg(long)]
        db_url: String,
    },
    /// Leaf inspection commands
    Leaf {
        #[command(subcommand)]
        command: LeafCmd,
    },
    /// Page inspection commands
    Page {
        #[command(subcommand)]
        command: PageCmd,
    },
    /// Interactive navigator
    Nav {
        #[arg(long)]
        container: String,
    },
}

#[derive(Subcommand)]
pub enum LeafCmd {
    /// List leaves
    List {
        #[arg(long)]
        container: String,
    },
    /// Show a single leaf
    Show {
        #[arg(long)]
        container: String,
        #[arg(long)]
        leaf_id: u64,
        #[arg(long, default_value_t = false)]
        pretty_json: bool,
    },
}

#[derive(Subcommand)]
pub enum PageCmd {
    /// List pages at a level
    List {
        #[arg(long)]
        container: String,
        #[arg(long)]
        level: u32,
    },
    /// Show a single page
    Show {
        #[arg(long)]
        container: String,
        #[arg(long)]
        page_id: u64,
        #[arg(long, default_value_t = false)]
        raw: bool,
    },
}

pub async fn run() -> CJResult<()> {
    let cli = Cli::parse();
    let config = init(None)?;
    let journal = Journal::new(config).await?;
    match cli.command {
        Commands::Simulate { container, fields, duration, errors_parked: _, errors_malformed: _, start, seed } => {
            simulate(&journal, &container, fields, &duration, &start, seed).await?;
        }
        Commands::State { container, as_of } => {
            state_cmd(&journal, &container, &as_of).await?;
        }
        Commands::Revert { .. } => {
            println!("revert: not yet implemented");
        }
        Commands::Leaf { command } => {
            match command {
                LeafCmd::List { container } => list_leaves(&journal, &container).await?,
                LeafCmd::Show { container, leaf_id, pretty_json } => show_leaf(&journal, &container, leaf_id, pretty_json).await?,
            }
        }
        Commands::Page { command } => {
            match command {
                PageCmd::List { container: _, level } => list_pages(&journal, level).await?,
                PageCmd::Show { container: _, page_id, raw } => show_page(&journal, page_id, raw).await?,
            }
        }
        Commands::Nav { .. } => {
            println!("nav: not yet implemented");
        }
    }
    Ok(())
}

fn parse_duration_spec(spec: &str) -> CJResult<Duration> {
    if spec.len() < 2 { return Err(crate::CJError::invalid_input("invalid duration".into())); }
    let (num_str, unit) = spec.split_at(spec.len()-1);
    let n: i64 = num_str.parse().map_err(|_| crate::CJError::invalid_input("invalid duration".into()))?;
    let dur = match unit {
        "y" => Duration::days(n * 365),
        "m" => Duration::days(n * 30),
        "d" => Duration::days(n),
        _ => return Err(crate::CJError::invalid_input("invalid duration".into())),
    };
    Ok(dur)
}

async fn simulate(journal: &Journal, container: &str, fields: u32, duration: &str, start: &str, seed: u64) -> CJResult<()> {
    let start_ts = start.parse::<DateTime<Utc>>().map_err(|e| crate::CJError::new(e.to_string()))?;
    let dur = parse_duration_spec(duration)?;
    let mut rng = StdRng::seed_from_u64(seed);
    let range_secs = dur.num_seconds();
    for i in 0..fields {
        let field_name = format!("field{}", i + 1);
        let offset = rng.gen_range(0..range_secs);
        let ts = start_ts + Duration::seconds(offset);
        let val: String = Sentence(1..2).fake();
        let payload = json!({ field_name: val });
        journal.append_leaf(ts, None, container.to_string(), payload).await?;
    }
    println!("Simulated {} fields over {}", fields, duration);
    Ok(())
}

async fn state_cmd(journal: &Journal, container: &str, as_of: &str) -> CJResult<()> {
    let at = as_of.parse::<DateTime<Utc>>().map_err(|e| crate::CJError::new(e.to_string()))?;
    let state = journal.reconstruct_container_state(container, at).await?;
    println!("{}", serde_json::to_string_pretty(&state.state_data).unwrap_or_default());
    Ok(())
}

async fn list_leaves(journal: &Journal, container: &str) -> CJResult<()> {
    let mut pages = journal.query.storage().list_finalized_pages_summary(0).await?;
    if let Some(active) = journal.manager.get_active_page(0).await {
        pages.push(crate::core::page::JournalPageSummary {
            page_id: active.page_id,
            level: active.level,
            creation_timestamp: active.creation_timestamp,
            end_time: active.end_time,
            page_hash: active.page_hash,
        });
    }
    pages.sort_by_key(|p| p.page_id);
    for summary in pages {
        if let Some(page) = journal.query.storage().load_page(summary.level, summary.page_id).await? {
            if let crate::core::page::PageContent::Leaves(leaves) = page.content {
                for leaf in leaves {
                    if leaf.container_id == container {
                        println!("{:>6} {} {}", leaf.leaf_id, leaf.timestamp.to_rfc3339(), hex::encode(leaf.leaf_hash));
                    }
                }
            }
        }
    }
    Ok(())
}

async fn show_leaf(journal: &Journal, container: &str, leaf_id: u64, pretty: bool) -> CJResult<()> {
    let mut pages = journal.query.storage().list_finalized_pages_summary(0).await?;
    if let Some(active) = journal.manager.get_active_page(0).await {
        pages.push(crate::core::page::JournalPageSummary {
            page_id: active.page_id,
            level: active.level,
            creation_timestamp: active.creation_timestamp,
            end_time: active.end_time,
            page_hash: active.page_hash,
        });
    }
    pages.sort_by_key(|p| p.page_id);
    for summary in pages {
        if let Some(page) = journal.query.storage().load_page(summary.level, summary.page_id).await? {
            if let crate::core::page::PageContent::Leaves(leaves) = page.content {
                for leaf in leaves {
                    if leaf.container_id == container && leaf.leaf_id == leaf_id {
                        let output = if pretty {
                            serde_json::to_string_pretty(&leaf.delta_payload).unwrap_or_default()
                        } else {
                            serde_json::to_string(&leaf.delta_payload).unwrap_or_default()
                        };
                        println!("Leaf {} @ {}\n{}", leaf_id, leaf.timestamp.to_rfc3339(), output);
                        return Ok(());
                    }
                }
            }
        }
    }
    println!("Leaf {} not found", leaf_id);
    Ok(())
}

async fn list_pages(journal: &Journal, level: u32) -> CJResult<()> {
    let mut pages = journal.query.storage().list_finalized_pages_summary(level as u8).await?;
    pages.sort_by_key(|p| p.page_id);
    for p in pages {
        println!("L{}P{} {}", p.level, p.page_id, p.creation_timestamp.to_rfc3339());
    }
    Ok(())
}

async fn show_page(journal: &Journal, page_id: u64, raw: bool) -> CJResult<()> {
    if let Some(page) = journal.query.storage().load_page(0, page_id).await? {
        if raw {
            println!("page_hash {}", hex::encode(page.page_hash));
        } else {
            println!("{}", serde_json::to_string_pretty(&page).unwrap_or_default());
        }
    } else {
        println!("Page {} not found", page_id);
    }
    Ok(())
}

