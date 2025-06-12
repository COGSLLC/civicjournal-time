#![cfg(feature = "demo")]

//! Scaffolding for the new `cj-demo` command line interface.
//!
//! Each subcommand is currently a stub. Implementation will follow
//! the specification in `DEMOMODE.md`.

use crate::api::async_api::{Journal, PageContentHash};
use crate::{init, CJResult};
use clap::{Parser, Subcommand};
#[cfg(feature = "demo")]
mod auto_db;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::{Color, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, SetSize},
};
use fake::{faker::lorem::en::Sentence, Fake};
use hex;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde_json::json;
use std::io::{self, Write};
use std::time::Duration as StdDuration;
use tokio_postgres::NoTls;

const DEMO_VERSION: &str = "1";

#[derive(Parser)]
#[command(author, version, about="CivicJournal Demo CLI", long_about=None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run default demo (generate data and launch navigator)
    Demo,
    /// Remove demo data directory
    Cleanup,
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
    match cli.command.unwrap_or(Commands::Demo) {
        Commands::Demo => run_demo(&config).await?,
        Commands::Cleanup => {
            cleanup_demo(&config)?;
            return Ok(());
        }
        Commands::Simulate {
            container,
            fields,
            duration,
            errors_parked: _,
            errors_malformed: _,
            start,
            seed,
        } => {
            let journal = Journal::new(config).await?;
            simulate(&journal, &container, fields, &duration, &start, seed).await?;
        }
        Commands::State { container, as_of } => {
            let journal = Journal::new(config).await?;
            state_cmd(&journal, &container, &as_of).await?;
        }
        Commands::Revert {
            container,
            as_of,
            db_url,
        } => {
            let journal = Journal::new(config).await?;
            revert_cmd(&journal, &container, &as_of, &db_url).await?;
        }
        Commands::Leaf { command } => {
            let journal = Journal::new(config).await?;
            match command {
                LeafCmd::List { container } => list_leaves(&journal, &container).await?,
                LeafCmd::Show {
                    container,
                    leaf_id,
                    pretty_json,
                } => show_leaf(&journal, &container, leaf_id, pretty_json).await?,
            }
        }
        Commands::Page { command } => {
            let journal = Journal::new(config).await?;
            match command {
                PageCmd::List {
                    container: _,
                    level,
                } => list_pages(&journal, level).await?,
                PageCmd::Show {
                    container: _,
                    page_id,
                    raw,
                } => show_page(&journal, page_id, raw).await?,
            }
        }
        Commands::Nav { container } => {
            let journal = Journal::new(config).await?;
            let mut idx = 0usize;
            let mut level = 0u8;
            nav_cmd(&journal, &container, &mut idx, &mut level).await?;
        }
    }
    Ok(())
}

fn parse_duration_spec(spec: &str) -> CJResult<Duration> {
    if spec.len() < 2 {
        return Err(crate::CJError::invalid_input("invalid duration"));
    }
    let (num_str, unit) = spec.split_at(spec.len() - 1);
    let n: i64 = num_str
        .parse()
        .map_err(|_| crate::CJError::invalid_input("invalid duration"))?;
    let dur = match unit {
        "y" => Duration::days(n * 365),
        "m" => Duration::days(n * 30),
        "d" => Duration::days(n),
        _ => return Err(crate::CJError::invalid_input("invalid duration")),
    };
    Ok(dur)
}

async fn simulate(
    journal: &Journal,
    container: &str,
    fields: u32,
    duration: &str,
    start: &str,
    seed: u64,
) -> CJResult<()> {
    let start_ts = start
        .parse::<DateTime<Utc>>()
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    let dur = parse_duration_spec(duration)?;
    let mut rng = StdRng::seed_from_u64(seed);
    let range_secs = dur.num_seconds();

    use crate::turnstile::Turnstile;
    let mut ts_queue = Turnstile::new("00".repeat(32), 1);

    let mut prev: Option<PageContentHash> = None;
    for i in 0..fields {
        let field_name = format!("field{}", i + 1);
        let offset = rng.gen_range(0..range_secs);
        let ts = start_ts + Duration::seconds(offset);
        let val: String = Sentence(1..2).fake();
        let payload = json!({ field_name: val });
        if i + 1 == 5 {
            // malformed packet
            if ts_queue.append("{", ts.timestamp() as u64).is_err() {
                journal
                    .append_leaf(
                        ts,
                        prev.clone(),
                        container.to_string(),
                        json!({"log":"malformed packet"}),
                    )
                    .await?;
            }
            let ticket = ts_queue.append(&payload.to_string(), ts.timestamp() as u64 + 1)?;
            ts_queue.confirm_ticket(&ticket, true, None)?;
            let res = journal
                .append_leaf(
                    ts + Duration::seconds(1),
                    prev.clone(),
                    container.to_string(),
                    payload,
                )
                .await?;
            if let PageContentHash::LeafHash(h) = res {
                prev = Some(PageContentHash::LeafHash(h));
            }
        } else if i + 1 == 10 {
            let ticket = ts_queue.append(&payload.to_string(), ts.timestamp() as u64)?;
            ts_queue.confirm_ticket(&ticket, false, Some("network error"))?;
            let res1 = journal
                .append_leaf(
                    ts,
                    prev.clone(),
                    container.to_string(),
                    json!({"log":"network error"}),
                )
                .await?;
            if let PageContentHash::LeafHash(h) = res1 {
                prev = Some(PageContentHash::LeafHash(h));
            }
            ts_queue.retry_next_pending(|_, _, _| 1)?;
            let res2 = journal
                .append_leaf(
                    ts + Duration::seconds(1),
                    prev.clone(),
                    container.to_string(),
                    payload,
                )
                .await?;
            if let PageContentHash::LeafHash(h) = res2 {
                prev = Some(PageContentHash::LeafHash(h));
            }
            journal
                .append_leaf(
                    ts + Duration::seconds(2),
                    prev.clone(),
                    container.to_string(),
                    json!({"log":"retry success"}),
                )
                .await?;
        } else {
            let ticket = ts_queue.append(&payload.to_string(), ts.timestamp() as u64)?;
            ts_queue.confirm_ticket(&ticket, true, None)?;
            let res = journal
                .append_leaf(ts, prev.clone(), container.to_string(), payload)
                .await?;
            if let PageContentHash::LeafHash(h) = res {
                prev = Some(PageContentHash::LeafHash(h));
            }
        }
    }
    println!("Simulated {} fields over {}", fields, duration);
    Ok(())
}

async fn state_cmd(journal: &Journal, container: &str, as_of: &str) -> CJResult<()> {
    let at = as_of
        .parse::<DateTime<Utc>>()
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    let state = journal.reconstruct_container_state(container, at).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&state.state_data).unwrap_or_default()
    );
    Ok(())
}

async fn revert_cmd(journal: &Journal, container: &str, as_of: &str, db_url: &str) -> CJResult<()> {
    let at = as_of
        .parse::<DateTime<Utc>>()
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    let state = journal.reconstruct_container_state(container, at).await?;
    let (mut client, connection) = tokio_postgres::connect(db_url, NoTls)
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });
    let tx = client
        .transaction()
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    tx.execute(
        &format!(
            "CREATE TABLE IF NOT EXISTS {} (field TEXT PRIMARY KEY, value JSONB)",
            container
        ),
        &[],
    )
    .await
    .map_err(|e| crate::CJError::new(e.to_string()))?;
    tx.execute(&format!("TRUNCATE {}", container), &[])
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    if let Some(map) = state.state_data.as_object() {
        for (field, value) in map {
            let json_str =
                serde_json::to_string(value).map_err(|e| crate::CJError::new(e.to_string()))?;
            tx.execute(
                &format!("INSERT INTO {} (field, value) VALUES ($1, $2)", container),
                &[&field, &json_str],
            )
            .await
            .map_err(|e| crate::CJError::new(e.to_string()))?;
        }
    }
    tx.commit()
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    journal
        .append_leaf(
            Utc::now(),
            None,
            container.to_string(),
            json!({"revert_to": as_of}),
        )
        .await?;
    println!("Reverted {} to {}", container, as_of);
    Ok(())
}

async fn list_leaves(journal: &Journal, container: &str) -> CJResult<()> {
    let mut pages = journal
        .query
        .storage()
        .list_finalized_pages_summary(0)
        .await?;
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
        if let Some(page) = journal
            .query
            .storage()
            .load_page(summary.level, summary.page_id)
            .await?
        {
            if let crate::core::page::PageContent::Leaves(leaves) = page.content {
                for leaf in leaves {
                    if leaf.container_id == container {
                        println!(
                            "{:>6} {} {}",
                            leaf.leaf_id,
                            leaf.timestamp.to_rfc3339(),
                            hex::encode(leaf.leaf_hash)
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

async fn show_leaf(journal: &Journal, container: &str, leaf_id: u64, pretty: bool) -> CJResult<()> {
    let mut pages = journal
        .query
        .storage()
        .list_finalized_pages_summary(0)
        .await?;
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
        if let Some(page) = journal
            .query
            .storage()
            .load_page(summary.level, summary.page_id)
            .await?
        {
            if let crate::core::page::PageContent::Leaves(leaves) = page.content {
                for leaf in leaves {
                    if leaf.container_id == container && leaf.leaf_id == leaf_id {
                        let output = if pretty {
                            serde_json::to_string_pretty(&leaf.delta_payload).unwrap_or_default()
                        } else {
                            serde_json::to_string(&leaf.delta_payload).unwrap_or_default()
                        };
                        let prev = leaf
                            .prev_hash
                            .map(|h| hex::encode(h))
                            .unwrap_or_else(|| "None".to_string());
                        println!("Leaf {} @ {}", leaf_id, leaf.timestamp.to_rfc3339(),);
                        println!("Prev: {}", prev);
                        println!("Payload: {}", output);
                        println!("Hash: {}", hex::encode(leaf.leaf_hash));
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
    let mut pages = journal
        .query
        .storage()
        .list_finalized_pages_summary(level as u8)
        .await?;
    pages.sort_by_key(|p| p.page_id);
    for p in pages {
        println!(
            "L{}P{} {}",
            p.level,
            p.page_id,
            p.creation_timestamp.to_rfc3339()
        );
    }
    Ok(())
}

async fn show_page(journal: &Journal, page_id: u64, raw: bool) -> CJResult<()> {
    for level in 0u8..=6u8 {
        if let Some(page) = journal.query.storage().load_page(level, page_id).await? {
            if raw {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&page).unwrap_or_default()
                );
            } else {
                println!("Page L{}P{}", page.level, page.page_id);
                println!("Start: {}", page.creation_timestamp.to_rfc3339());
                println!("End: {}", page.end_time.to_rfc3339());
                let prev = page
                    .prev_page_hash
                    .map(|h| hex::encode(h))
                    .unwrap_or_else(|| "None".to_string());
                println!("Prev: {}", prev);
                println!("Merkle: {}", hex::encode(page.merkle_root));
                println!("Hash: {}", hex::encode(page.page_hash));
                use crate::core::page::PageContent;
                match &page.content {
                    PageContent::Leaves(leaves) => {
                        println!("Leaves ({}):", leaves.len());
                        for leaf in leaves {
                            println!(
                                "  #{:>6} {} {}",
                                leaf.leaf_id,
                                leaf.timestamp.to_rfc3339(),
                                hex::encode(leaf.leaf_hash)
                            );
                        }
                    }
                    PageContent::ThrallHashes(hashes) => {
                        println!("Child Page Hashes ({}):", hashes.len());
                        for h in hashes {
                            println!("  {}", hex::encode(h));
                        }
                    }
                    PageContent::NetPatches(patches) => {
                        println!(
                            "NetPatch: {}",
                            serde_json::to_string_pretty(patches).unwrap_or_default()
                        );
                    }
                    PageContent::ThrallHashesWithNetPatches { hashes, patches } => {
                        println!("Child Page Hashes ({}):", hashes.len());
                        for h in hashes {
                            println!("  {}", hex::encode(h));
                        }
                        println!(
                            "NetPatch: {}",
                            serde_json::to_string_pretty(patches).unwrap_or_default()
                        );
                    }
                    PageContent::Snapshot(_) => {
                        println!("Snapshot page");
                    }
                }
            }
            return Ok(());
        }
    }
    println!("Page {} not found", page_id);
    Ok(())
}

async fn collect_leaves(
    journal: &Journal,
    container: &str,
) -> CJResult<Vec<crate::core::leaf::JournalLeaf>> {
    let mut pages = journal
        .query
        .storage()
        .list_finalized_pages_summary(0)
        .await?;
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
    let mut leaves = Vec::new();
    for summary in pages {
        if let Some(page) = journal
            .query
            .storage()
            .load_page(summary.level, summary.page_id)
            .await?
        {
            if let crate::core::page::PageContent::Leaves(ls) = page.content {
                for leaf in ls {
                    if leaf.container_id == container {
                        leaves.push(leaf);
                    }
                }
            }
        }
    }
    leaves.sort_by_key(|l| l.leaf_id);
    Ok(leaves)
}

async fn find_page_for_ts(
    journal: &Journal,
    level: u8,
    ts: DateTime<Utc>,
) -> CJResult<Option<crate::core::page::JournalPage>> {
    let mut pages = journal
        .query
        .storage()
        .list_finalized_pages_summary(level)
        .await?;
    if let Some(active) = journal.manager.get_active_page(level).await {
        pages.push(crate::core::page::JournalPageSummary {
            page_id: active.page_id,
            level: active.level,
            creation_timestamp: active.creation_timestamp,
            end_time: active.end_time,
            page_hash: active.page_hash,
        });
    }
    for summary in pages {
        if ts >= summary.creation_timestamp && ts < summary.end_time {
            if let Some(page) = journal
                .query
                .storage()
                .load_page(level, summary.page_id)
                .await?
            {
                return Ok(Some(page));
            }
        }
    }
    Ok(None)
}

fn read_line(prompt: &str) -> io::Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn parse_fuzzy_datetime(input: &str) -> Option<DateTime<Utc>> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let parts: Vec<&str> = input.split(|c| c == 'T' || c == ' ').collect();
    let date_part = parts.get(0)?;
    let time_part = parts.get(1).copied().unwrap_or("");

    let mut date_it = date_part.split('-');
    let year: i32 = date_it.next()?.parse().ok()?;
    let month: u32 = date_it.next().and_then(|m| m.parse().ok()).unwrap_or(1);
    let day: u32 = date_it.next().and_then(|d| d.parse().ok()).unwrap_or(1);

    let mut hour: u32 = 0;
    let mut minute: u32 = 1;
    let mut second: u32 = 0;
    if !time_part.is_empty() {
        let mut time_it = time_part.split(':');
        if let Some(h) = time_it.next() {
            hour = h.parse().unwrap_or(0);
        }
        if let Some(m) = time_it.next() {
            minute = m.parse().unwrap_or(1);
        }
        if let Some(s) = time_it.next() {
            second = s.parse().unwrap_or(0);
        }
    }

    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let dt = date.and_hms_opt(hour, minute, second)?;
    Some(DateTime::<Utc>::from_utc(dt, Utc))
}

fn show_help(stdout: &mut io::Stdout) -> io::Result<()> {
    execute!(
        stdout,
        Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        SetForegroundColor(Color::White)
    )?;
    writeln!(stdout, "Navigation Help")?;
    execute!(stdout, SetForegroundColor(Color::Grey))?;
    writeln!(stdout, "←/→ : previous/next leaf")?;
    writeln!(stdout, "↑/↓ : parent/child page")?;
    writeln!(stdout, "S : show state at timestamp")?;
    writeln!(stdout, "R : revert database")?;
    writeln!(stdout, "F : find leaf by id")?;
    writeln!(stdout, "D : display database state")?;
    writeln!(stdout, "L : view log entries")?;
    writeln!(stdout, "Q : quit")?;
    writeln!(stdout, "Press any key to continue...")?;
    execute!(stdout, SetForegroundColor(Color::White))?;
    stdout.flush()?;
    let _ = event::read();
    Ok(())
}

async fn show_state_prompt(journal: &Journal, container: &str) -> CJResult<()> {
    terminal::disable_raw_mode()?;
    let ts_str = read_line("Timestamp (YYYY[-MM[-DD[THH[:MM[:SS]]]]]): ")?;
    let Some(at) = parse_fuzzy_datetime(&ts_str) else {
        execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        println!("Invalid timestamp");
        let _ = read_line("Press Enter to continue...");
        terminal::enable_raw_mode()?;
        return Ok(());
    };
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    let state = journal.reconstruct_container_state(container, at).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&state.state_data).unwrap_or_default()
    );
    let _ = read_line("Press Enter to continue...");
    execute!(io::stdout(), Clear(ClearType::All))?;
    terminal::enable_raw_mode()?;
    Ok(())
}

async fn revert_prompt(journal: &Journal, container: &str) -> CJResult<()> {
    terminal::disable_raw_mode()?;
    let ts_str = read_line("Revert to timestamp (YYYY[-MM[-DD[THH[:MM[:SS]]]]]): ")?;
    let Some(at) = parse_fuzzy_datetime(&ts_str) else {
        execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        println!("Invalid timestamp");
        let _ = read_line("Press Enter to continue...");
        terminal::enable_raw_mode()?;
        return Ok(());
    };
    let db_url = read_line("Postgres URL: ")?;
    let confirm = read_line("Are you sure? [y/N] ")?;
    if confirm.eq_ignore_ascii_case("y") {
        execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        terminal::enable_raw_mode()?;
        revert_cmd(journal, container, &at.to_rfc3339(), &db_url).await?;
        terminal::disable_raw_mode()?;
        println!("Reverted {} to {}", container, at.to_rfc3339());
        let _ = read_line("Press Enter to continue...");
    } else {
        terminal::enable_raw_mode()?;
    }
    execute!(io::stdout(), Clear(ClearType::All))?;
    Ok(())
}

async fn search_prompt(leaves: &[crate::core::leaf::JournalLeaf]) -> CJResult<Option<usize>> {
    terminal::disable_raw_mode()?;
    let id_str = read_line("Leaf id to jump to: ")?;
    execute!(io::stdout(), Clear(ClearType::All))?;
    terminal::enable_raw_mode()?;
    if let Ok(id) = id_str.parse::<u64>() {
        for (i, l) in leaves.iter().enumerate() {
            if l.leaf_id == id {
                return Ok(Some(i));
            }
        }
    }
    Ok(None)
}

async fn dump_prompt(leaf: &crate::core::leaf::JournalLeaf) -> CJResult<()> {
    terminal::disable_raw_mode()?;
    let path = read_line("Dump file path: ")?;
    std::fs::write(&path, serde_json::to_vec_pretty(&leaf.delta_payload)?)?;
    println!("Saved to {}", path);
    let _ = read_line("Press Enter to continue...");
    execute!(io::stdout(), Clear(ClearType::All))?;
    terminal::enable_raw_mode()?;
    Ok(())
}

async fn display_db_prompt(journal: &Journal, container: &str) -> CJResult<()> {
    terminal::disable_raw_mode()?;
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    let state = journal
        .reconstruct_container_state(container, Utc::now())
        .await?;
    use serde_json::Value;
    use std::cmp::max;
    use std::collections::BTreeMap;
    let mut fields: Vec<(usize, String)> = Vec::new();
    let mut other: BTreeMap<String, String> = BTreeMap::new();
    if let Some(map) = state.state_data.as_object() {
        for (k, v) in map {
            if k.starts_with("field") {
                if let Ok(n) = k.trim_start_matches("field").parse::<usize>() {
                    let val = match v {
                        Value::String(s) => s.clone(),
                        _ => v.to_string(),
                    };
                    fields.push((n, val));
                } else {
                    other.insert(k.clone(), v.to_string());
                }
            } else {
                other.insert(k.clone(), v.to_string());
            }
        }
    }
    fields.sort_by_key(|(n, _)| *n);
    let formatted: Vec<String> = fields
        .iter()
        .map(|(n, v)| format!("field{}: {}", n, v))
        .collect();
    let (term_w, _) = terminal::size().unwrap_or((80, 20));
    let max_len: u16 = formatted.iter().map(|s| s.len() as u16).max().unwrap_or(0);
    let col_width = max(max_len + 2, 20);
    let cols = max(1, term_w / col_width);
    let rows = (formatted.len() as u16 + cols - 1) / cols;
    for r in 0..rows {
        let mut line = String::new();
        for c in 0..cols {
            let idx = (c * rows + r) as usize;
            if idx < formatted.len() {
                line.push_str(&format!(
                    "{:<width$}",
                    formatted[idx],
                    width = col_width as usize
                ));
            }
        }
        println!("{}", line.trim_end());
    }
    for (k, v) in other {
        println!("{}: {}", k, v);
    }
    let _ = read_line("Press Enter to continue...");
    execute!(io::stdout(), Clear(ClearType::All))?;
    terminal::enable_raw_mode()?;
    Ok(())
}

async fn log_viewer_prompt(journal: &Journal, container: &str) -> CJResult<()> {
    let logs_all = collect_leaves(journal, container).await?;
    let logs: Vec<_> = logs_all
        .into_iter()
        .filter(|l| l.delta_payload.get("log").is_some())
        .collect();
    if logs.is_empty() {
        execute!(io::stdout(), cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        println!("No log entries found.");
        let _ = read_line("Press Enter to continue...");
        return Ok(());
    }
    let mut idx: usize = 0;
    let mut stdout = io::stdout();
    loop {
        execute!(
            stdout,
            cursor::MoveTo(0, 0),
            Clear(ClearType::All),
            SetForegroundColor(Color::White)
        )?;
        let leaf = &logs[idx];
        let msg = leaf
            .delta_payload
            .get("log")
            .and_then(|v| v.as_str())
            .unwrap_or("log");
        writeln!(stdout, "Log entry {}/{}", idx + 1, logs.len())?;
        writeln!(
            stdout,
            "Leaf #{} @ {}",
            leaf.leaf_id,
            leaf.timestamp.to_rfc3339()
        )?;
        writeln!(stdout, "Message: {}", msg)?;
        execute!(stdout, SetForegroundColor(Color::Grey))?;
        writeln!(stdout, "[← prev] [→ next] [Q quit]")?;
        execute!(stdout, SetForegroundColor(Color::White))?;
        stdout.flush()?;
        if event::poll(StdDuration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    match key.code {
                        KeyCode::Left => {
                            if idx > 0 {
                                idx -= 1;
                            }
                        }
                        KeyCode::Right => {
                            if idx + 1 < logs.len() {
                                idx += 1;
                            }
                        }
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break,
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

async fn render_nav(
    stdout: &mut io::Stdout,
    container: &str,
    level: u8,
    idx: usize,
    leaves: &[crate::core::leaf::JournalLeaf],
    journal: &Journal,
) -> CJResult<()> {
    execute!(
        stdout,
        cursor::MoveTo(0, 0),
        Clear(ClearType::All),
        SetForegroundColor(Color::White)
    )?;
    if level == 0 {
        let leaf = &leaves[idx];
        writeln!(stdout, "Container: {}", container)?;
        writeln!(
            stdout,
            "Leaf #{} @ {}",
            leaf.leaf_id,
            leaf.timestamp.to_rfc3339()
        )?;
        execute!(stdout, SetForegroundColor(Color::Grey))?;
        writeln!(stdout, "[← prev] [→ next]  [↑ parent] [↓ child]")?;
        execute!(stdout, SetForegroundColor(Color::White))?;
        writeln!(
            stdout,
            "Payload: {}",
            serde_json::to_string(&leaf.delta_payload).unwrap_or_default()
        )?;
        let prev = leaf
            .prev_hash
            .map(|h| hex::encode(h))
            .unwrap_or_else(|| "None".to_string());
        writeln!(stdout, "Prev: {}", prev)?;
        writeln!(stdout, "Hash: {}", hex::encode(leaf.leaf_hash))?;
        execute!(stdout, SetForegroundColor(Color::Grey))?;
        writeln!(
            stdout,
            "[D display DB] [S state] [R revert] [F find] [L logs] [H help] [Q quit]"
        )?;
        execute!(stdout, SetForegroundColor(Color::White))?;
    } else {
        let leaf = &leaves[idx];
        if let Some(page) = find_page_for_ts(journal, level, leaf.timestamp).await? {
            writeln!(stdout, "Container: {}", container)?;
            writeln!(stdout, "Page L{}P{}", page.level, page.page_id)?;
            execute!(stdout, SetForegroundColor(Color::Grey))?;
            writeln!(stdout, "[← prev] [→ next]  [↑ parent] [↓ child]")?;
            execute!(stdout, SetForegroundColor(Color::White))?;
            writeln!(stdout, "Start: {}", page.creation_timestamp.to_rfc3339())?;
            writeln!(stdout, "End: {}", page.end_time.to_rfc3339())?;
            let prev = page
                .prev_page_hash
                .map(|h| hex::encode(h))
                .unwrap_or_else(|| "None".to_string());
            writeln!(stdout, "Prev: {}", prev)?;
            writeln!(stdout, "Hash: {}", hex::encode(page.page_hash))?;
            use crate::core::page::PageContent;
            match &page.content {
                PageContent::Leaves(leaves) => {
                    writeln!(stdout, "Leaves ({}):", leaves.len())?;
                    for l in leaves.iter().take(5) {
                        writeln!(stdout, "  #{:>6} {}", l.leaf_id, hex::encode(l.leaf_hash))?;
                    }
                    if leaves.len() > 5 {
                        writeln!(stdout, "  ... ({} total)", leaves.len())?;
                    }
                }
                PageContent::ThrallHashes(hashes) => {
                    writeln!(stdout, "Child Hashes ({}):", hashes.len())?;
                    for h in hashes.iter().take(5) {
                        writeln!(stdout, "  {}", hex::encode(h))?;
                    }
                    if hashes.len() > 5 {
                        writeln!(stdout, "  ... ({} total)", hashes.len())?;
                    }
                }
                PageContent::NetPatches(p) => {
                    writeln!(
                        stdout,
                        "NetPatch: {}",
                        serde_json::to_string_pretty(p).unwrap_or_default()
                    )?;
                }
                PageContent::ThrallHashesWithNetPatches { hashes, patches } => {
                    writeln!(stdout, "Child Hashes ({}):", hashes.len())?;
                    for h in hashes.iter().take(5) {
                        writeln!(stdout, "  {}", hex::encode(h))?;
                    }
                    if hashes.len() > 5 {
                        writeln!(stdout, "  ... ({} total)", hashes.len())?;
                    }
                    writeln!(
                        stdout,
                        "NetPatch: {}",
                        serde_json::to_string_pretty(patches).unwrap_or_default()
                    )?;
                }
                PageContent::Snapshot(_) => {}
            }
            execute!(stdout, SetForegroundColor(Color::Grey))?;
            writeln!(
                stdout,
                "[D display DB] [S state] [R revert] [F find] [L logs] [H help] [Q quit]"
            )?;
            execute!(stdout, SetForegroundColor(Color::White))?;
        } else {
            writeln!(stdout, "No page at level {}", level)?;
        }
    }
    stdout.flush()?;
    Ok(())
}

async fn nav_cmd(
    journal: &Journal,
    container: &str,
    idx: &mut usize,
    level: &mut u8,
) -> CJResult<()> {
    let leaves = collect_leaves(journal, container).await?;
    if leaves.is_empty() {
        println!("No leaves for {}", container);
        return Ok(());
    }
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    const MIN_W: u16 = 80;
    const MIN_H: u16 = 20;
    let (w, h) = terminal::size()?;
    if w < MIN_W || h < MIN_H {
        execute!(io::stdout(), SetSize(MIN_W, MIN_H))?;
    }
    execute!(
        io::stdout(),
        SetBackgroundColor(Color::Blue),
        SetForegroundColor(Color::White),
        Clear(ClearType::All)
    )?;
    let mut stdout = io::stdout();
    let mut needs_render = true;
    loop {
        if needs_render {
            render_nav(&mut stdout, container, *level, *idx, &leaves, journal).await?;
            needs_render = false;
        }
        if event::poll(StdDuration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    match key.code {
                        KeyCode::Left => {
                            if *idx > 0 {
                                *idx -= 1;
                                needs_render = true;
                            }
                        }
                        KeyCode::Right => {
                            if *idx + 1 < leaves.len() {
                                *idx += 1;
                                needs_render = true;
                            }
                        }
                        KeyCode::Up => {
                            if *level < 5 {
                                if find_page_for_ts(journal, *level + 1, leaves[*idx].timestamp)
                                    .await?
                                    .is_some()
                                {
                                    *level += 1;
                                    needs_render = true;
                                }
                            }
                        }
                        KeyCode::Down => {
                            if *level > 0 {
                                *level -= 1;
                                needs_render = true;
                            }
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => break,
                        KeyCode::Char('h') | KeyCode::Char('H') => {
                            show_help(&mut stdout)?;
                            needs_render = true;
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            show_state_prompt(journal, container).await?;
                            needs_render = true;
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            revert_prompt(journal, container).await?;
                            needs_render = true;
                        }
                        KeyCode::Char('f') | KeyCode::Char('F') => {
                            if let Some(n) = search_prompt(&leaves).await? {
                                *idx = n;
                                needs_render = true;
                            }
                        }
                        KeyCode::Char('d') | KeyCode::Char('D') => {
                            display_db_prompt(journal, container).await?;
                            needs_render = true;
                        }
                        KeyCode::Char('l') | KeyCode::Char('L') => {
                            log_viewer_prompt(journal, container).await?;
                            needs_render = true;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    execute!(io::stdout(), ResetColor, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

async fn run_demo(config: &'static crate::Config) -> CJResult<()> {
    use std::path::Path;
    let base = Path::new(&config.storage.base_path);
    let sentinel = base.join("demo_version");
    let need_gen = match config.storage.storage_type {
        crate::StorageType::File => {
            if !base.exists() {
                true
            } else {
                match std::fs::read_to_string(&sentinel) {
                    Ok(v) if v.trim() == DEMO_VERSION => false,
                    _ => {
                        cleanup_demo(config)?;
                        true
                    }
                }
            }
        }
        _ => false,
    };
    let (db_url, handle) = auto_db::launch_postgres().await?;
    println!("PostgreSQL running at {}", db_url);
    let journal = Journal::new(config).await?;
    if need_gen {
        println!("Generating demo data...");
        generate_demo_data(&journal, "demoDB").await?;
        std::fs::create_dir_all(base)?;
        std::fs::write(&sentinel, DEMO_VERSION)?;
    }
    let r = demo_app(&journal, "demoDB").await;
    drop(handle);
    r
}

async fn demo_app(journal: &Journal, container: &str) -> CJResult<()> {
    let mut idx: usize = 0;
    let mut level: u8 = 0;
    terminal::enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        SetBackgroundColor(Color::Blue),
        SetForegroundColor(Color::White),
        Clear(ClearType::All)
    )?;
    let mut stdout = io::stdout();
    let mut needs_render = true;
    loop {
        if needs_render {
            render_menu(&mut stdout)?;
            needs_render = false;
        }
        if event::poll(StdDuration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    match key.code {
                        KeyCode::Char('o') | KeyCode::Char('O') => {
                            execute!(io::stdout(), LeaveAlternateScreen)?;
                            terminal::disable_raw_mode()?;
                            nav_cmd(journal, container, &mut idx, &mut level).await?;
                            terminal::enable_raw_mode()?;
                            execute!(
                                io::stdout(),
                                EnterAlternateScreen,
                                SetBackgroundColor(Color::Blue),
                                SetForegroundColor(Color::White),
                                Clear(ClearType::All)
                            )?;
                            needs_render = true;
                        }
                        KeyCode::Char('d') | KeyCode::Char('D') => {
                            display_db_prompt(journal, container).await?;
                            needs_render = true;
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            show_state_prompt(journal, container).await?;
                            needs_render = true;
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            revert_prompt(journal, container).await?;
                            needs_render = true;
                        }
                        KeyCode::Char('h') | KeyCode::Char('H') => {
                            show_help(&mut stdout)?;
                            needs_render = true;
                        }
                        KeyCode::Char('l') | KeyCode::Char('L') => {
                            log_viewer_prompt(journal, container).await?;
                            needs_render = true;
                        }
                        KeyCode::Char('q') | KeyCode::Char('Q') => break,
                        _ => {}
                    }
                }
            }
        }
    }
    execute!(io::stdout(), ResetColor, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

fn render_menu(stdout: &mut io::Stdout) -> io::Result<()> {
    execute!(
        stdout,
        cursor::MoveTo(0, 0),
        Clear(ClearType::All),
        SetForegroundColor(Color::White)
    )?;
    writeln!(stdout, "CJ Demo Main Menu")?;
    execute!(stdout, SetForegroundColor(Color::Grey))?;
    writeln!(stdout, "O : open navigator")?;
    writeln!(stdout, "D : display database state")?;
    writeln!(stdout, "S : show state at timestamp")?;
    writeln!(stdout, "R : revert database")?;
    writeln!(stdout, "L : view log entries")?;
    writeln!(stdout, "H : help")?;
    writeln!(stdout, "Q : quit")?;
    execute!(stdout, SetForegroundColor(Color::White))?;
    stdout.flush()?;
    Ok(())
}

fn cleanup_demo(config: &crate::Config) -> CJResult<()> {
    if config.storage.storage_type == crate::StorageType::File {
        if std::path::Path::new(&config.storage.base_path).exists() {
            std::fs::remove_dir_all(&config.storage.base_path)?;
            println!("Removed {}", &config.storage.base_path);
        }
    }
    Ok(())
}

async fn generate_demo_data(journal: &Journal, container: &str) -> CJResult<()> {
    use crate::turnstile::Turnstile;
    use indicatif::{ProgressBar, ProgressStyle};
    let mut ts = Turnstile::new("00".repeat(32), 1);
    let mut rng = StdRng::seed_from_u64(42);

    #[derive(Clone)]
    struct DemoEvent {
        ts: DateTime<Utc>,
        field: String,
        value: String,
        error: bool,
    }

    let start = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();

    let mut events: Vec<DemoEvent> = Vec::new();
    let mut update_counts = vec![0u32; 10];

    fn field_message(idx: usize, count: u32) -> String {
        format!(
            "I am data in field {}. I have been updated {} times now.",
            idx + 1,
            count
        )
    }

    const UPDATES_PER_FIELD: usize = 100;

    for i in 0..10 {
        let field = format!("field{}", i + 1);
        for _ in 0..UPDATES_PER_FIELD {
            let day_offset = rng.gen_range(0..365) as i64;
            let day_seconds = rng.gen_range(8 * 3600..18 * 3600) as i64;
            let ts = start + Duration::days(day_offset) + Duration::seconds(day_seconds);
            update_counts[i] += 1;
            let value = field_message(i, update_counts[i]);
            events.push(DemoEvent {
                ts,
                field: field.clone(),
                value,
                error: false,
            });
        }
    }

    events.sort_by_key(|e| e.ts);

    let pb = ProgressBar::new(events.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
        )
        .unwrap(),
    );
    pb.enable_steady_tick(StdDuration::from_millis(100));

    let mut update_indices: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.value.contains("upd"))
        .map(|(i, _)| i)
        .collect();
    update_indices.shuffle(&mut rng);
    if update_indices.len() >= 3 {
        events[update_indices[0]].error = true;
        events[update_indices[1]].error = true;
        let mal_ts = start + Duration::seconds(rng.gen_range(0..(365 * 24 * 3600)) as i64);
        if ts.append("{", mal_ts.timestamp() as u64).is_err() {
            journal
                .append_leaf(
                    mal_ts + Duration::seconds(1),
                    None,
                    container.to_string(),
                    json!({"log":"malformed packet"}),
                )
                .await?;
        }
        // Simulate a network error by attempting to connect to an unreachable port
        use std::net::TcpStream;
        if let Err(e) = TcpStream::connect("127.0.0.1:9") {
            journal
                .append_leaf(
                    mal_ts + Duration::seconds(2),
                    None,
                    container.to_string(),
                    json!({"log": format!("network error: {}", e)}),
                )
                .await?;
        }
    }

    let mut prev: Option<PageContentHash> = None;
    for event in events {
        let payload = json!({ event.field.clone(): event.value });
        let ticket = ts.append(&payload.to_string(), event.ts.timestamp() as u64)?;
        if event.error {
            ts.confirm_ticket(&ticket, false, Some("db error"))?;
            let res1 = journal
                .append_leaf(
                    event.ts,
                    prev.clone(),
                    container.to_string(),
                    json!({"log":"db error"}),
                )
                .await?;
            if let PageContentHash::LeafHash(h) = res1 {
                prev = Some(PageContentHash::LeafHash(h));
            }
            let res2 = journal
                .append_leaf(
                    event.ts + Duration::seconds(1),
                    prev.clone(),
                    container.to_string(),
                    payload.clone(),
                )
                .await?;
            if let PageContentHash::LeafHash(h2) = res2 {
                prev = Some(PageContentHash::LeafHash(h2));
            }
            ts.retry_next_pending(|_, _, _| 1)?;
            journal
                .append_leaf(
                    event.ts + Duration::seconds(2),
                    prev.clone(),
                    container.to_string(),
                    json!({"log":"retry success"}),
                )
                .await?;
        } else {
            ts.confirm_ticket(&ticket, true, None)?;
            let res = journal
                .append_leaf(event.ts, prev.clone(), container.to_string(), payload)
                .await?;
            if let PageContentHash::LeafHash(h) = res {
                prev = Some(PageContentHash::LeafHash(h));
            }
        }
        pb.inc(1);
    }
    pb.finish_with_message("Data generation complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_duration_spec_valid() {
        assert_eq!(parse_duration_spec("5d").unwrap(), Duration::days(5));
        assert_eq!(parse_duration_spec("2m").unwrap(), Duration::days(60));
        assert_eq!(parse_duration_spec("1y").unwrap(), Duration::days(365));
    }

    #[test]
    fn test_parse_duration_spec_invalid() {
        assert!(parse_duration_spec("5x").is_err());
        assert!(parse_duration_spec("12").is_err());
        assert!(parse_duration_spec("x").is_err());
    }

    #[test]
    fn test_cleanup_demo_removes_directory() {
        use crate::config::Config;
        use crate::StorageType;
        let dir = tempdir().unwrap();
        let path = dir.path().join("demo");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("dummy"), b"test").unwrap();

        let mut cfg = Config::default();
        cfg.storage.storage_type = StorageType::File;
        cfg.storage.base_path = path.to_str().unwrap().to_string();

        cleanup_demo(&cfg).unwrap();
        assert!(!path.exists());
    }
}
