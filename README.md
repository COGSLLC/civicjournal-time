⚠️ **BETA (v0.4.0)** — Core rollup, turnstile, snapshot, and query features are stable. Production-ready for typical use cases.
# CivicJournal-Time

**An append-only, verifiable ledger for robust audit trails and time-series data management.**

CivicJournal-Time is a Rust-based system designed to create immutable, chronologically-ordered logs of events or data changes. It's particularly well-suited for tracking the history of external systems, providing a secure and verifiable audit trail that allows for state reconstruction, data integrity verification, and detailed auditing.

⚠︎ **BETA (v0.4.0)** — Full hierarchical rollups, snapshots, and query engine operational.

## Core Concepts

CivicJournal-Time organizes data into a hierarchical structure that ensures both efficiency and verifiability:

### Key Components

* **`JournalLeaf`**: The fundamental unit of data representing a single event or delta.
  * Contains timestamp, payload, and cryptographic hashes linking to previous leaves
  * Defined in `src/core/leaf.rs`
  
* **`JournalPage`**: Container for organizing data at each level of the time hierarchy.
  * Level 0: Stores actual `JournalLeaf` objects
  * Level 1+: Stores hashes of child pages or net patches
  * Implements Merkle tree for cryptographic verification
  * Defined in `src/core/page.rs`

* **Time Hierarchy**: Multi-level structure for efficient data management:
  - Level 0: Raw leaves (most granular)
  - Level 1: Aggregates Level 0 pages
  - Level 2-6: Progressively coarser time windows
  - Each level has configurable time windows and rollup policies

### Rollup Mechanism

The rollup process aggregates data from lower to higher levels based on:

1. **Size-based Triggers**:
   - `max_items_per_page`: Maximum number of items before rollup
   - Configured per level in `config.toml`

2. **Time-based Triggers**:
   - `max_page_age_seconds`: Maximum age before rollup
   - Ensures data is rolled up even with low volume

3. **Event-based Triggers**:
   - External events can trigger immediate rollup
   - Useful for ensuring data persistence after critical operations

### Storage Format

Journal pages are stored in `.cjt` files with a 6-byte header:
- Bytes 0-3: Magic string `CJTP`
- Byte 4: Format version (currently `1`)
- Byte 5: Compression algorithm (0=None, 1=Zstd, 2=Lz4, 3=Snappy)

### Data Integrity

* Each page includes a Merkle root of its contents
* Parent pages contain hashes of child pages
* Cryptographic chaining ensures tamper evidence
* Built-in verification tools validate page chains

## Key Features

### Core Functionality
* **Append-Only & Immutable**: Data cannot be modified or deleted, ensuring a trustworthy audit trail
* **Verifiable Data Integrity**: Cryptographic hashing and Merkle trees guarantee data authenticity
* **Time-Based Hierarchy**: Efficient organization of data across multiple time granularities
* **Configurable Rollup Strategies**: Flexible rules for data aggregation and summarization

### Storage & Performance
* **Pluggable Storage Backends**:
  - `FileStorage`: Disk-based persistence with compression
  - `MemoryStorage`: For testing and development
* **Compression Support**:
  - Zstd, Lz4, and Snappy compression algorithms
  - Configurable compression levels
* **Efficient File Format**:
  - Custom `.cjt` format with 6-byte header
  - Supports random access to page data

### Data Management
* **Flexible Retention Policies**:
  - `KeepIndefinitely`: Retain all data
  - `DeleteAfterSecs`: Automatic cleanup after specified period
  - `KeepNPages`: Maintain fixed-size rolling window
* **Backup & Restore**:
  - Full journal backup to zip archives
  - Atomic restore operations
  - Backup manifest with integrity information
  - Asynchronous API for non-blocking operations
* **Snapshots**:
  - Dedicated level (default `250`) stores full system state snapshots without requiring placeholder levels in between
  - Enables fast restoration and archival of older journal data

### Query Capabilities
* **Merkle Proofs**: Verify inclusion of specific leaves
* **State Reconstruction**: Rebuild container state at any point in time
* **Delta Reports**: Track changes over specified time ranges
* **Chain Verification**: Validate integrity of page hierarchies

### Integration
* **Dual API Support**:
  - Asynchronous API for high-throughput applications
  - Synchronous wrapper for simpler integration
* **FFI Bindings**:
  - C-compatible interface
  - WebAssembly (WASM) support
  - Turnstile pattern for coordination with external systems

### Query Engine and Turnstile Integration

The project includes a full query subsystem (`src/query/`) capable of generating
Merkle proofs, reconstructing container state, and reporting deltas over time.
These capabilities are exposed through the public Rust APIs as well as C and
WASM FFI bindings under `src/ffi/`. The optional Turnstile pattern uses these
bindings to coordinate writes with external databases while ensuring
append-only guarantees.

## Project Structure

```
src/
├── api/                  # Public API interfaces
│   ├── async_api.rs      # Asynchronous API implementation
│   ├── sync_api.rs       # Synchronous API wrapper
│   └── mod.rs            # Module exports and documentation
├── config/               # Configuration management
│   ├── config.toml       # Default configuration
│   ├── error.rs          # Configuration error types
│   ├── mod.rs            # Configuration loading and validation
│   └── validation.rs     # Configuration validation logic
├── core/                 # Core data structures and logic
│   ├── hash.rs           # Cryptographic hashing utilities
│   ├── leaf.rs           # JournalLeaf implementation
│   ├── merkle.rs         # Merkle tree implementation
│   ├── page.rs           # JournalPage and PageContent
│   └── time_manager.rs   # TimeHierarchyManager implementation
├── error.rs              # Error types and handling
├── ffi/                  # Foreign Function Interface
│   ├── c_ffi.rs          # C-compatible bindings
│   ├── mod.rs            # Module exports
│   └── wasm_ffi.rs       # WebAssembly bindings
├── query/                # Query engine
│   ├── engine.rs         # Query implementation
│   ├── mod.rs            # Module exports
│   └── types.rs          # Query result types
├── storage/              # Storage backends
│   ├── file.rs           # File-based storage
│   ├── memory.rs         # In-memory storage
│   └── mod.rs            # StorageBackend trait
├── test_utils.rs         # Testing utilities
├── time/                 # Time-related functionality
│   ├── hierarchy.rs      # Time hierarchy management
│   ├── level.rs          # Time level definitions
│   ├── mod.rs            # Module exports
│   └── unit.rs           # Time unit constants
└── types/                # Common types
    ├── compression.rs    # Compression types
    ├── error.rs          # Error type definitions
    ├── log_level.rs      # Logging level types
    ├── mod.rs            # Module exports
    ├── storage.rs        # Storage-related types
    └── time.rs           # Time-related types
```

### Key Modules

* **API Layer** (`api/`):
  - `async_api.rs`: Core async API for high-performance applications
  - `sync_api.rs`: Blocking wrapper around async API
  - Comprehensive error handling and documentation

* **Core Logic** (`core/`):
  - `time_manager.rs`: Orchestrates page lifecycle and rollups
  - `page.rs`: Page data structures and operations
  - `leaf.rs`: Leaf data structure and validation
  - `merkle.rs`: Cryptographic verification

* **Storage** (`storage/`):
  - `file.rs`: Persistent storage with compression
  - `memory.rs`: Volatile storage for testing
  - Common `StorageBackend` trait for extensibility

* **Query Engine** (`query/`):
  - State reconstruction
  - Merkle proof generation
  - Time-based queries
  - Delta analysis

## Getting Started

### Prerequisites

* Rust 1.70+ (latest stable recommended)
* Cargo (Rust's package manager)
* For development: Git, Clippy, Rustfmt

### Installation

1. Install Rust (if not already installed):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. Clone the repository:
   ```bash
   git clone https://github.com/COGSLLC/civicjournal-time.git
   cd civicjournal-time
   ```

3. Build the project:
   ```bash
   # Debug build
   cargo build
   
   # Release build with optimizations
   cargo build --release
   
   # Run tests
   cargo test
   
   # Run benchmarks
   cargo bench
   ```

### Basic Usage

```rust
use civicjournal_time::{
    api::{AsyncJournal, Config, TimeLevel},
    types::time::{LevelRollupConfig, RollupContentType, RollupRetentionPolicy},
};
use chrono::Utc;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a basic configuration
    let config = Config {
        time_hierarchy: TimeLevel::default_levels(),
        rollup: vec![
            LevelRollupConfig {
                max_items_per_page: 1000,
                max_page_age_seconds: 3600, // 1 hour
                content_type: RollupContentType::ChildHashes,
            },
            // Add more levels as needed
        ],
        ..Default::default()
    };

    // Create a new journal instance
    let journal = AsyncJournal::new(Arc::new(config)).await?;
    
    // Example: Add a leaf
    let leaf = journal.create_leaf(
        "container-123",
        serde_json::json!({ "action": "create", "user": "alice" }),
        None, // prev_hash
    )?;
    
    let page_id = journal.append_leaf(leaf).await?;
    println!("Leaf added to page: {}", page_id);
    
    // Query the journal
    let state = journal.reconstruct_state("container-123", Utc::now()).await?;
    println!("Current state: {:?}", state);
    
    Ok(())
}
```

### Configuration

CivicJournal-Time is highly configurable through a TOML file. Here's an example `config.toml`:

```toml
[time_hierarchy]
levels = [
    { name = "minute", duration_seconds = 60 },
    { name = "hour", duration_seconds = 3600 },
    { name = "day", duration_seconds = 86400 },
    { name = "month", duration_seconds = 2592000 },
    { name = "year", duration_seconds = 31536000 },
    { name = "decade", duration_seconds = 315360000 },
    { name = "century", duration_seconds = 3153600000 },
]

[rollup]
# Configuration for each level (index matches time_hierarchy.levels - 1)
[[rollup.levels]]
max_items_per_page = 1000
max_page_age_seconds = 3600  # 1 hour
content_type = "ChildHashes"  # or "NetPatches"

[[rollup.levels]]
max_items_per_page = 500
max_page_age_seconds = 86400  # 1 day
content_type = "ChildHashes"

[storage]
type = "file"  # or "memory"
base_path = "./data"
max_open_files = 1000

[compression]
enabled = true
algorithm = "zstd"  # or "lz4", "snappy", "none"
level = 3  # Compression level (1-22 for zstd)

[retention]
enabled = true
period_seconds = 2592000  # 30 days
cleanup_interval_seconds = 86400  # 1 day

[logging]
level = "info"  # "trace", "debug", "info", "warn", "error"
console = true
file = false
file_path = "./civicjournal.log"
```

You can also override configuration values using environment variables. Prefix
the upper‑case path with `CJ_`, for example `CJ_LOGGING_LEVEL=debug`.

### Backup and Restore

```rust
// Create a backup
journal.backup("./backups/journal_backup.zip").await?;

// Restore from backup
journal.restore("./backups/journal_backup.zip").await?;
```

### Querying Data

```rust
// Get inclusion proof for a leaf (optional page hint for efficiency)
let proof = journal
    .get_leaf_inclusion_proof_with_hint(leaf_hash, Some((0, 42)))
    .await?;

// Reconstruct container state at a specific time
let state = journal.reconstruct_state(
    "container-123",
    Utc.ymd(2023, 1, 1).and_hms(12, 0, 0)
).await?;

// Get changes within a time range
let changes = journal.get_delta_report(
    "container-123",
    Utc.ymd(2023, 1, 1).and_hms(0, 0, 0),
    Utc.ymd(2023, 1, 2).and_hms(0, 0, 0)
).await?;
// or fetch results in pages
let first_page = journal
    .get_delta_report_paginated(
        "container-123",
        Utc.ymd(2023, 1, 1).and_hms(0, 0, 0),
        Utc.ymd(2023, 1, 2).and_hms(0, 0, 0),
        0,
        100,
    )
    .await?;

// Verify page chain integrity
let is_valid = journal.verify_page_chain(0, Some(1), Some(100)).await?;
```
    ```bash
    cargo build --release
    ```

### Running Tests

Ensure all components are functioning correctly:

```bash
cargo test
```
This runs all unit and integration tests, including the expanded memory
storage suite located in `tests/memory_storage_tests.rs`. These tests now
cover failure simulation, page and leaf lookups, existence checks and
clearing logic, concurrency, targeted and level-wide failure simulation for
the in-memory backend.
Additional unit tests in `tests/basic_types_tests.rs` verify enum conversions
for common types and constructors for core error helpers.
Turnstile tests in `tests/turnstile_tests.rs` cover persistence, retry logic,
orphan handling, and invalid input including corruption detection.
Page behavior tests in `tests/page_behavior_tests.rs` validate net patch hashing
order independence and the page finalization logic.

For running tests that manipulate time for age-based rollup testing:

```bash
cargo test -- --include-ignored
```

*(Note for Windows users: File locking issues with the test executable can sometimes occur. If `cargo clean` or `cargo test` fails unexpectedly, a system reboot or checking for locking processes might be necessary.)*

## Configuration

CivicJournal-Time is configured via a central configuration structure, typically initialized when creating a `Journal` instance. Key configurable aspects include:

*   **Time Hierarchy Levels**: Definition of each time level, including its duration (`epoch_seconds`).
*   **Rollup Parameters**: Per-level settings for `max_items_per_page`, `max_page_age_seconds`, and `RollupContentType`.
*   **Storage Configuration**: Base path for `FileStorage`, compression algorithm and level.
*   **Global Settings**: Such as `force_rollup_on_shutdown`.

Refer to `src/config/mod.rs` and `src/types/time.rs` for details on configuration structures.

## File Format (`.cjt`)

Journal pages managed by `FileStorage` are stored as `.cjt` files. Each file begins with a 6-byte header:

1.  **Magic String (4 bytes)**: `CJTP` (CivicJournal Time Page)
2.  **Format Version (1 byte)**: e.g., `1`
3.  **Compression Algorithm (1 byte)**: `0` (None), `1` (Zstd), `2` (Lz4), `3` (Snappy)

The rest of the file contains the (potentially) compressed and serialized `JournalPage` data.

## Backup and Restore

The `FileStorage` backend provides methods for backing up the entire journal to a zip archive and restoring from it. A `backup_manifest.json` file is generated during backup, containing metadata about the backup process and the files included.

## Demo Mode

Use the optional `demo` feature to generate sample data and explore rollups and snapshots. Build and run the simulator with:

```bash
cargo run --features demo --bin journal-demo -- run --mode batch
```

See [DEMOMODE.md](DEMOMODE.md) for full configuration and PostgreSQL setup instructions.

## WebAssembly Bindings

The crate exposes a `WasmJournal` type for WebAssembly environments when built
with `wasm-bindgen`. Use `wasm-pack` to compile the project:

```bash
wasm-pack build --target web
```

In JavaScript, initialize the module and interact with the journal asynchronously:

```javascript
import init, { WasmJournal, default_config, journal_version } from './pkg/civicjournal_time.js';

async function run() {
  await init();
  console.log('Version:', journal_version());
  const cfg = default_config();
  const journal = await WasmJournal.new(cfg);
  const hash = await journal.append_leaf(Date.now(), 'web', JSON.stringify({ msg: 'hello' }));
  console.log('Appended leaf', hash);
}

run();
```

## C FFI

The library can also be compiled as a `cdylib` exposing a C-compatible API used
by the Turnstile integration. Build with:

```bash
cargo build --release --features c-ffi
```

Link your C application against the resulting library and include the generated
`cjt_ffi.h` header. See [TURNSTILE.md](TURNSTILE.md) for a complete list of
available functions and an extended usage example.

## Additional Features

The codebase includes several capabilities that are not required for basic usage but can be enabled through configuration:

* **Automatic Snapshot Scheduling** – periodic snapshots can be created by enabling the `[snapshot.automatic_creation]` section and supplying a cron expression or interval.
* **Snapshot Retention Policies** – `[snapshot.retention]` allows pruning old snapshots based on count or maximum age.
* **Metrics Collection** – the `[metrics]` section pushes internal metrics to a monitoring endpoint at a configurable interval.
* **Environment Variable Overrides** – any configuration option can be overridden by environment variables prefixed with `CJ_` (e.g. `CJ_LOGGING_LEVEL=debug`).
* **Memory Storage & Demo Mode** – a `MemoryStorage` backend and the `demo` feature provide an in-memory simulator useful for tests and demonstrations.

## Further Documentation

For more in-depth information, please refer to the following documents in the repository:

*   `ARCHITECTURE.md`: Describes the overall system architecture.
*   `ROLLUP.MD`: Details the rollup mechanism.
*   `CivicJournalSpec.txt`: (Note: This document may contain some outdated information regarding page structure and file formats. Prioritize information from `README.md`, `ARCHITECTURE.MD`, and `ROLLUP.MD`.)
*   `plan.md`: Tracks development progress and future plans.

## Contributing

Contributions are welcome! Please follow these general guidelines:

1.  Fork the repository.
2.  Create a new branch for your feature or bug fix.
3.  Make your changes, ensuring to add or update tests as appropriate.
4.  Ensure all tests pass (`cargo test`).
5.  Format your code (`cargo fmt`).
6.  Submit a pull request with a clear description of your changes.

## License

### Open Source License

CivicJournal-Time is available under the **GNU Affero General Public License v3.0 only** (AGPL-3.0-only) for open-source use.

[![AGPL-3.0-only](https://img.shields.io/badge/License-AGPL_v3_only-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)

This is a strong copyleft license that requires any derivative works to be distributed under the same license.

### Commercial Licensing

For organizations that prefer not to be bound by the AGPL-3.0-only terms, commercial licenses are available. Commercial licenses provide:

- **No AGPL Copyleft Requirements**: Use CivicJournal-Time in proprietary applications without the requirement to release your source code.

Also available:
- **Custom Development**: Tailored features and customizations to meet your specific needs.


**Contact us** at `licensing@cogs.llc` to discuss commercial licensing options tailored to your organization's needs.

### License Comparison

| Feature | AGPL-3.0-only | Commercial License |
|---------|---------------|-------------------|
| Source Code Access Required | Yes | No |
| Copyleft Requirements | Yes | No |
| Priority Support | Community-based | Included |
| Custom Development | Not included | Available |
| Legal Indemnification | No | Included |

See the [LICENSE](LICENSE) file for full AGPL-3.0-only license text and terms.
