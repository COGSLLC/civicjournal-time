⚠️ **BETA (v0.3.0)** — Core rollup, turnstile, and query features are stable.
# CivicJournal-Time

**An append-only, verifiable ledger for robust audit trails and time-series data management.**

CivicJournal-Time is a Rust-based system designed to create immutable, chronologically-ordered logs of events or data changes. It's particularly well-suited for tracking the history of external systems, providing a secure and verifiable audit trail that allows for state reconstruction, data integrity verification, and detailed auditing.



## Core Concepts

At its heart, CivicJournal-Time organizes data into a hierarchical structure, ensuring both efficiency and verifiability:

*   **`JournalLeaf`**: The fundamental unit of data. Each leaf represents a single event or delta, containing a timestamp, payload, and cryptographic hashes linking it to previous leaves. (See `src/core/leaf.rs`)
*   **`JournalPage`**: Collections of `JournalLeaf` objects (at Level 0) or hashes of child pages (at higher levels). Pages are the building blocks of the time hierarchy and are individually verifiable using Merkle trees. (See `src/core/page.rs`)
*   **Time Hierarchy**: A multi-level structure where pages from lower levels are summarized and rolled up into pages at higher levels. This allows for efficient querying and data management over long periods.
*   **Rollups**: The process of aggregating finalized child page data (either full leaves or their hashes/net-patches) into parent-level pages. Rollups are triggered based on configurable criteria:
    *   Maximum number of items per page (`max_items_per_page`).
    *   Maximum age of a page (`max_page_age_seconds`).
    *   The type of content rolled up (e.g., `ChildHashes`, `NetPatches`) is also configurable per level.
    (See `src/core/time_manager.rs` and `src/types/time.rs`)
*   **Merkle Trees**: Used within each `JournalPage` to calculate a single Merkle root hash. This root cryptographically represents all content within the page, allowing for efficient verification of data integrity and inclusion proofs. (See `src/core/merkle.rs`)

## Key Features

*   **Append-Only & Immutable**: Ensures data, once written, cannot be altered, providing a trustworthy record.
*   **Verifiable Data Integrity**: Cryptographic hashing and Merkle trees at every level guarantee data authenticity and detect tampering.
*   **Hierarchical Time-Based Organization**: Efficiently manages and queries large volumes of time-series data.
*   **Configurable Rollup Strategies**: Flexible rules for how and when data is aggregated into higher-level summaries.
*   **Pluggable Storage Backend**:
    *   Currently features `FileStorage` for disk-based persistence.
    *   Designed to support other backends in the future.
*   **Configurable Page Compression**: Supports Zstd, Lz4, Snappy, or no compression for stored page files to save space.
*   **Custom `.cjt` File Format**: Journal pages are stored in `.cjt` (CivicJournal Time Page) files, which include a 6-byte header specifying magic string, format version, and compression algorithm.
*   **Backup and Restore**:
    *   Built-in functionality to backup the journal to a zip archive.
    *   Ability to restore the journal from a backup.
    *   Includes a `backup_manifest.json` for tracking backup details.
*   **Synchronous & Asynchronous APIs**: Provides both blocking and non-blocking APIs for integration flexibility. (See `src/api/`)

### Query Engine and Turnstile Integration

The project includes a full query subsystem (`src/query/`) capable of generating
Merkle proofs, reconstructing container state, and reporting deltas over time.
These capabilities are exposed through the public Rust APIs as well as C and
WASM FFI bindings under `src/ffi/`. The optional Turnstile pattern uses these
bindings to coordinate writes with external databases while ensuring
append-only guarantees.

## Project Structure

A brief overview of the main directories:

*   `src/api/`: Contains the synchronous (`sync_api.rs`) and asynchronous (`async_api.rs`) public interfaces.
*   `src/config/`: Handles configuration loading, validation, and default values.
*   `src/core/`: Implements the core logic and data structures:
    *   `leaf.rs`: `JournalLeaf` definition.
    *   `page.rs`: `JournalPage` and `PageContent` definitions.
    *   `merkle.rs`: `MerkleTree` implementation.
    *   `time_manager.rs`: `TimeHierarchyManager` for managing pages and rollups.
*   `src/error.rs`: Defines custom error types for the project.
*   `src/storage/`: Contains storage backend implementations (e.g., `file.rs` for `FileStorage`).
*   `src/types/`: Common data types and enums used throughout the project (e.g., `TimeLevel`, `LevelRollupConfig`).

## Getting Started

### Prerequisites

*   Rust toolchain (latest stable recommended). Install from [rustup.rs](https://rustup.rs/).

### Building the Project

1.  Clone the repository:
    ```bash
    git clone <repository-url>
    cd civicjournal-time
    ```
2.  Build the project:
    ```bash
    cargo build
    ```
    For a release build:
    ```bash
    cargo build --release
    ```

### Running Tests

Ensure all components are functioning correctly:

```bash
cargo test
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

This project is licensed under the AGPL-3.0 for open-source use; contact about commercial licensing See the `LICENSE` file for details.
