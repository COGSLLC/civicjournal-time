#![cfg(feature = "demo")]

//! Scaffolding for the new `cj-demo` command line interface.
//!
//! Each subcommand is currently a stub. Implementation will follow
//! the specification in `DEMOMODE.md`.

use clap::{Parser, Subcommand};
use crate::{init, CJResult};
use crate::api::async_api::Journal;
use chrono::{DateTime, Utc, Duration};
use tokio_postgres::NoTls;
use rand::{SeedableRng, Rng};
use rand::rngs::StdRng;
use fake::{Fake, faker::lorem::en::Sentence};
use serde_json::{json, Value};
use hex;
use std::io::{self, Write};
use std::time::Duration as StdDuration;
use crossterm::{execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen, Clear, ClearType, SetSize},
    cursor,
    event::{self, Event, KeyCode},
    style::{Color, SetBackgroundColor, SetForegroundColor, ResetColor}
};

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
        Commands::Revert { container, as_of, db_url } => {
            revert_cmd(&journal, &container, &as_of, &db_url).await?;
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
        Commands::Nav { container } => {
            nav_cmd(&journal, &container).await?;
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

async fn revert_cmd(journal: &Journal, container: &str, as_of: &str, db_url: &str) -> CJResult<()> {
    let at = as_of.parse::<DateTime<Utc>>().map_err(|e| crate::CJError::new(e.to_string()))?;
    let state = journal.reconstruct_container_state(container, at).await?;
    let (client, connection) = tokio_postgres::connect(db_url, NoTls).await.map_err(|e| crate::CJError::new(e.to_string()))?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });
    let tx = client.transaction().await.map_err(|e| crate::CJError::new(e.to_string()))?;
    tx.execute(&format!("CREATE TABLE IF NOT EXISTS {} (field TEXT PRIMARY KEY, value JSONB)", container), &[]).await.map_err(|e| crate::CJError::new(e.to_string()))?;
    tx.execute(&format!("TRUNCATE {}", container), &[]).await.map_err(|e| crate::CJError::new(e.to_string()))?;
    if let Some(map) = state.state_data.as_object() {
        for (field, value) in map {
            let json_str = serde_json::to_string(value).map_err(|e| crate::CJError::new(e.to_string()))?;
            tx.execute(&format!("INSERT INTO {} (field, value) VALUES ($1, $2)", container), &[&field, &json_str]).await.map_err(|e| crate::CJError::new(e.to_string()))?;
        }
    }
    tx.commit().await.map_err(|e| crate::CJError::new(e.to_string()))?;
    journal.append_leaf(Utc::now(), None, container.to_string(), json!({"revert_to": as_of})).await?;
    println!("Reverted {} to {}", container, as_of);
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

async fn collect_leaves(journal: &Journal, container: &str) -> CJResult<Vec<crate::core::leaf::JournalLeaf>> {
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
    let mut leaves = Vec::new();
    for summary in pages {
        if let Some(page) = journal.query.storage().load_page(summary.level, summary.page_id).await? {
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

async fn find_page_for_ts(journal: &Journal, level: u8, ts: DateTime<Utc>) -> CJResult<Option<crate::core::page::JournalPage>> {
    let mut pages = journal.query.storage().list_finalized_pages_summary(level).await?;
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
            if let Some(page) = journal.query.storage().load_page(level, summary.page_id).await? {
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

fn show_help(stdout: &mut io::Stdout) -> crossterm::Result<()> {
    execute!(stdout, Clear(ClearType::All), cursor::MoveTo(0,0), SetForegroundColor(Color::White))?;
    writeln!(stdout, "Navigation Help")?;
    execute!(stdout, SetForegroundColor(Color::Grey))?;
    writeln!(stdout, "←/→ : previous/next leaf")?;
    writeln!(stdout, "↑/↓ : parent/child page")?;
    writeln!(stdout, "S : show state at timestamp")?;
    writeln!(stdout, "R : revert database")?;
    writeln!(stdout, "F : find leaf by id")?;
    writeln!(stdout, "D : dump current item")?;
    writeln!(stdout, "Q : quit")?;
    writeln!(stdout, "Press any key to continue...")?;
    execute!(stdout, SetForegroundColor(Color::White))?;
    stdout.flush()?;
    let _ = event::read();
    Ok(())
}

async fn show_state_prompt(journal: &Journal, container: &str) -> CJResult<()> {
    terminal::disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    let ts_str = read_line("Timestamp (YYYY-MM-DDTHH:MM:SSZ): ")?;
    let at = ts_str.parse::<DateTime<Utc>>().map_err(|e| crate::CJError::new(e.to_string()))?;
    let state = journal.reconstruct_container_state(container, at).await?;
    println!("{}", serde_json::to_string_pretty(&state.state_data).unwrap_or_default());
    let _ = read_line("Press Enter to continue...");
    execute!(io::stdout(), EnterAlternateScreen, SetBackgroundColor(Color::Blue), SetForegroundColor(Color::White), Clear(ClearType::All))?;
    terminal::enable_raw_mode()?;
    Ok(())
}

async fn revert_prompt(journal: &Journal, container: &str) -> CJResult<()> {
    terminal::disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    let ts_str = read_line("Revert to timestamp (YYYY-MM-DDTHH:MM:SSZ): ")?;
    let db_url = read_line("Postgres URL: ")?;
    execute!(io::stdout(), EnterAlternateScreen, SetBackgroundColor(Color::Blue), SetForegroundColor(Color::White), Clear(ClearType::All))?;
    terminal::enable_raw_mode()?;
    revert_cmd(journal, container, &ts_str, &db_url).await
}

async fn search_prompt(leaves: &[crate::core::leaf::JournalLeaf]) -> CJResult<Option<usize>> {
    terminal::disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    let id_str = read_line("Leaf id to jump to: ")?;
    execute!(io::stdout(), EnterAlternateScreen, SetBackgroundColor(Color::Blue), SetForegroundColor(Color::White), Clear(ClearType::All))?;
    terminal::enable_raw_mode()?;
    if let Ok(id) = id_str.parse::<u64>() {
        for (i, l) in leaves.iter().enumerate() {
            if l.leaf_id == id { return Ok(Some(i)); }
        }
    }
    Ok(None)
}

async fn dump_prompt(leaf: &crate::core::leaf::JournalLeaf) -> CJResult<()> {
    terminal::disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    let path = read_line("Dump file path: ")?;
    std::fs::write(&path, serde_json::to_vec_pretty(&leaf.delta_payload)?)?;
    println!("Saved to {}", path);
    let _ = read_line("Press Enter to continue...");
    execute!(io::stdout(), EnterAlternateScreen, SetBackgroundColor(Color::Blue), SetForegroundColor(Color::White), Clear(ClearType::All))?;
    terminal::enable_raw_mode()?;
    Ok(())
}

async fn render_nav(stdout: &mut io::Stdout, container: &str, level: u8, idx: usize, leaves: &[crate::core::leaf::JournalLeaf], journal: &Journal) -> CJResult<()> {
    execute!(stdout, cursor::MoveTo(0,0), Clear(ClearType::All), SetForegroundColor(Color::White))?;
    if level == 0 {
        let leaf = &leaves[idx];
        writeln!(stdout, "Container: {}", container)?;
        writeln!(stdout, "Leaf #{} @ {}", leaf.leaf_id, leaf.timestamp.to_rfc3339())?;
        execute!(stdout, SetForegroundColor(Color::Grey))?;
        writeln!(stdout, "[← prev] [→ next]  [↑ parent] [↓ child]")?;
        execute!(stdout, SetForegroundColor(Color::White))?;
        writeln!(stdout, "Payload: {}", serde_json::to_string(&leaf.delta_payload).unwrap_or_default())?;
        writeln!(stdout, "Hash: {}", hex::encode(leaf.leaf_hash))?;
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
            writeln!(stdout, "Hash: {}", hex::encode(page.page_hash))?;
        } else {
            writeln!(stdout, "No page at level {}", level)?;
        }
    }
    stdout.flush()?;
    Ok(())
}

async fn nav_cmd(journal: &Journal, container: &str) -> CJResult<()> {
    let leaves = collect_leaves(journal, container).await?;
    if leaves.is_empty() { println!("No leaves for {}", container); return Ok(()); }
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    const MIN_W: u16 = 80;
    const MIN_H: u16 = 20;
    let (w, h) = terminal::size()?;
    if w < MIN_W || h < MIN_H {
        execute!(io::stdout(), SetSize(MIN_W, MIN_H))?;
    }
    execute!(io::stdout(), SetBackgroundColor(Color::Blue), SetForegroundColor(Color::White), Clear(ClearType::All))?;
    let mut stdout = io::stdout();
    let mut idx: usize = 0;
    let mut level: u8 = 0;
    loop {
        render_nav(&mut stdout, container, level, idx, &leaves, journal).await?;
        if event::poll(StdDuration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Left => if idx > 0 { idx -= 1; },
                    KeyCode::Right => if idx + 1 < leaves.len() { idx += 1; },
                    KeyCode::Up => if level < 5 { if find_page_for_ts(journal, level+1, leaves[idx].timestamp).await?.is_some() { level += 1; } },
                    KeyCode::Down => if level > 0 { level -= 1; },
                    KeyCode::Char('q') | KeyCode::Char('Q') => break,
                    KeyCode::Char('h') | KeyCode::Char('H') => { show_help(&mut stdout)?; },
                    KeyCode::Char('s') | KeyCode::Char('S') => { show_state_prompt(journal, container).await?; },
                    KeyCode::Char('r') | KeyCode::Char('R') => { revert_prompt(journal, container).await?; },
                    KeyCode::Char('f') | KeyCode::Char('F') => { if let Some(n) = search_prompt(&leaves).await? { idx = n; } },
                    KeyCode::Char('d') | KeyCode::Char('D') => { dump_prompt(&leaves[idx]).await?; },
                    _ => {}
                }
            }
        }
    }
    execute!(io::stdout(), ResetColor, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

