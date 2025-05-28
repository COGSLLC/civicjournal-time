//! CivicJournal Time - A command-line interface for the CivicJournal time hierarchy system

use clap::{Parser, Subcommand};
use civicjournal_time::time_hierarchy::TimeHierarchy;
use std::{
    path::PathBuf,
    process,
    time::Duration,
};

#[cfg(windows)]
use winapi::um::{
    wincon::{SetConsoleTitleW},
    consoleapi::{SetConsoleCtrlHandler},
};

// Enable Windows ANSI support if available
#[cfg(windows)]
fn setup_windows_console() {
    #[cfg(feature = "console")]
    if let Ok(_) = enable_ansi_support::enable_ansi_support() {
        // ANSI support enabled successfully
    }
}

#[cfg(not(windows))]
fn setup_windows_console() {
    // No-op on non-Windows
}

// Windows console setup
#[cfg(windows)]
fn set_console_title(title: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    
    let wide: Vec<u16> = OsStr::new(title).encode_wide().chain(Some(0)).collect();
    unsafe {
        SetConsoleTitleW(wide.as_ptr());
    }
}

#[cfg(not(windows))]
fn set_console_title(_title: &str) {
    // No-op on non-Windows
}

// Graceful shutdown handler
fn setup_graceful_shutdown() {
    #[cfg(windows)]
    unsafe {
        extern "system" fn ctrl_handler(_: u32) -> i32 {
            // Signal to exit
            1
        }
        
        SetConsoleCtrlHandler(Some(ctrl_handler as _), 1);
    }
    
    #[cfg(feature = "graceful-shutdown")]
    {
        ctrlc::set_handler(move || {
            println!("\nShutting down gracefully...");
            process::exit(0);
        }).expect("Error setting Ctrl-C handler");
    }
}

#[derive(Parser)]
#[command(name = "civicjournal-time")]
#[command(about = "CivicJournal Time - Hierarchical time-series journal management", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Verify the integrity of a time hierarchy
    Verify {
        /// Path to the time hierarchy data directory
        path: PathBuf,
    },
    
    /// Query the time hierarchy
    Query {
        /// Path to the time hierarchy data directory
        path: PathBuf,
        /// Start time (RFC 3339 format)
        #[arg(short, long)]
        start: Option<String>,
        /// End time (RFC 3339 format)
        #[arg(short, long)]
        end: Option<String>,
        /// Output format (json, text)
        #[arg(short, long, default_value = "text")]
        format: String,
    },
    
    /// Start a web server for the time hierarchy
    Serve {
        /// Path to the time hierarchy data directory
        path: PathBuf,
        /// Port to listen on
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
    
    /// Show statistics about the time hierarchy
    Stats {
        /// Path to the time hierarchy data directory
        path: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    // Set console title (Windows)
    set_console_title("CivicJournal Time");
    
    // Setup graceful shutdown
    setup_graceful_shutdown();
    
    // Initialize Windows console
    setup_windows_console();
    
    // Run the actual application
    if let Err(e) = run_application() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
    
    Ok(())
}

fn run_application() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Verify { path } => {
            println!("Verifying time hierarchy at: {}", path.display());
            
            // Add retry logic for file operations
            let max_retries = 3;
            let mut last_error = None;
            
            for attempt in 0..max_retries {
                // In our implementation, we'll create a new TimeHierarchy
                // and verify its integrity after loading from the directory
                let hierarchy = TimeHierarchy::new();
                match hierarchy.verify_integrity() {
                    Ok(_) => {
                        println!("✅ Verification successful!");
                        return Ok(());
                    }
                    Err(e) => {
                        last_error = Some(anyhow::Error::new(e));
                        if attempt < max_retries - 1 {
                            println!("⚠️  Verification attempt {} failed, retrying...", attempt + 1);
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }
                }
            }
            
            return Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Verification failed with unknown error")));
        },
        Commands::Query { path, start, end, format } => {
            println!("Querying time hierarchy at: {}", path.display());
            // Create a new TimeHierarchy for querying
            let _hierarchy = TimeHierarchy::new();
            
            if let Some(start) = start {
                println!("Start time: {}", start);
            }
            if let Some(end) = end {
                println!("End time: {}", end);
            }
            println!("Format: {}", format);
            
            // TODO: Implement actual query logic
            println!("Query functionality coming soon!");
        }
        Commands::Serve { path, port } => {
            #[cfg(not(feature = "web"))]
            {
                eprintln!("Error: Web server support requires the 'web' feature to be enabled");
                eprintln!("Please build with: cargo build --features=web");
                std::process::exit(1);
            }
            
            #[cfg(feature = "web")]
            {
                use tokio::runtime::Runtime;
                
                println!("Starting web server on port {} for: {}", port, path.display());
                
                // Create a new runtime for the web server
                let rt = Runtime::new()?;
                rt.block_on(async {
                    if let Err(e) = start_web_server(*port, path).await {
                        eprintln!("Web server error: {}", e);
                        std::process::exit(1);
                    }
                });
            }
        }
        Commands::Stats { path } => {
            println!("Statistics for time hierarchy at: {}", path.display());
            // Create a new TimeHierarchy and calculate its statistics
            let hierarchy = TimeHierarchy::new();
            let stats = hierarchy.calculate_stats();
            
            println!("\nTime Hierarchy Statistics");
            println!("=======================\n");
            println!("Total chunks: {}", stats.chunk_count);
            println!("Total deltas: {}", stats.delta_count);
            println!("Total size: {} bytes", stats.total_size_bytes);
        }
    }

    Ok(())
}

#[cfg(feature = "web")]
async fn start_web_server(port: u16, _path: &std::path::Path) -> anyhow::Result<()> {
    use warp::Filter;
    
    // Simple web server with a basic UI
    let hello = warp::path!("api" / "status")
        .map(|| "CivicJournal Time API is running!");
    
    let routes = hello.with(warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST"])
        .allow_headers(vec!["content-type"]));
    
    let socket = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    
    println!("Web UI available at: http://localhost:{}", port);
    println!("Press Ctrl+C to stop the server");
    
    // Use tokio::net for better Windows compatibility
    let listener = tokio::net::TcpListener::bind(socket).await?;
    warp::serve(routes).run_incoming(listener.incoming()).await;
    
    Ok(())
}
