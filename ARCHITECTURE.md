# CivicJournal-Time Architecture

**Last Updated**: 2025-06-05
**Version**: 0.4.0

## Overview

CivicJournal-Time is a high-performance, append-only ledger system designed for verifiable audit trails and time-series data management. This document provides a comprehensive overview of the system's architecture, components, and design decisions.

## Core Design Principles

1. **Immutable by Design**: All data is append-only, ensuring a tamper-evident audit trail.
2. **Verifiability**: Cryptographic hashing and Merkle trees enable efficient verification of data integrity.
3. **Scalability**: Hierarchical time-based organization allows efficient querying of large datasets.
4. **Flexibility**: Configurable rollup and retention policies adapt to various use cases.
5. **Performance**: Optimized for high-throughput write operations with configurable read performance trade-offs.

## Core Architecture

### Overview

CivicJournal Time is a hierarchical Merkle-chained delta-log system designed for tamper-evident time-series data. It organizes data into a time-based hierarchy where each level represents a different time granularity, from minutes up to centuries.

### Core Components

1. **Time Hierarchy System**
   - Manages time-based organization of journal entries across 7 levels (minutes to centuries)
   - Handles page creation, sealing, and roll-up operations
   - Implements configurable time windows for each level
   - Enforces data retention policies and cleanup

2. **Journal Management**
   - Manages the lifecycle of journal entries and pages
   - Handles both synchronous and asynchronous operations
   - Implements rollup policies based on size and age thresholds
   - Provides backup and restore functionality

3. **Storage System**
   - Abstract `StorageBackend` trait for pluggable storage implementations
   - Built-in implementations:
     - `FileStorage`: Disk-based storage with configurable compression
     - `MemoryStorage`: In-memory storage for testing and development
   - Supports multiple compression algorithms (Zstd, Lz4, Snappy, None)
   - Implements efficient page serialization/deserialization

4. **Query Engine**
   - Provides interfaces for data retrieval and verification
   - Implements Merkle proofs for data integrity
   - Supports state reconstruction and time-range queries
   - Enables verification of page chain integrity

5. **Configuration System**
   - Comprehensive TOML-based configuration
   - Environment variable overrides support
   - Runtime configuration validation
   - Sensible defaults with customization options

### Module Structure

```
src/
├── config/           # Configuration management
│   ├── mod.rs        # Configuration loading and validation
│   ├── error.rs      # Configuration error handling
│   ├── validation.rs # Configuration validation logic
│   └── config.toml   # Default configuration
├── types/            # Core type definitions
│   ├── mod.rs        # Type exports
│   ├── time.rs       # Time-related types and utilities
│   └── journal.rs    # Journal entry and page types
├── core/             # Core functionality
│   ├── mod.rs        # Core module exports
│   ├── hash.rs       # Cryptographic hashing utilities
│   ├── leaf.rs       # JournalLeaf implementation
│   ├── page.rs       # JournalPage implementation
│   └── merkle.rs     # Merkle tree implementation
├── storage/          # Storage backends
│   ├── mod.rs        # StorageBackend trait and implementations
│   ├── memory.rs     # In-memory storage backend
│   ├── file.rs       # File system storage backend
│   └── error.rs      # Storage-related errors
├── time/             # Time hierarchy implementation
│   ├── mod.rs        # Time hierarchy management
│   ├── level.rs      # Time level definitions and utilities
│   ├── rollup.rs     # Roll-up logic and policies
│   └── manager.rs    # TimeHierarchyManager implementation
├── query/            # Query engine
│   ├── mod.rs        # Query module exports
│   ├── engine.rs     # QueryEngine implementation
│   └── types.rs      # Query-related types and results
├── api/              # Public API
│   ├── mod.rs        # Public interface
│   ├── sync.rs       # Synchronous API wrapper
│   └── error.rs      # API error types
└── utils/            # Utility modules
    ├── compression/  # Compression algorithms
    └── serialization/ # Serialization utilities
```

### Key Data Structures

1. **JournalLeaf**
   - Represents a single delta/change
   - Contains cryptographic links to previous entries
   - Includes metadata and payload

2. **JournalPage**
   - Represents a page in the time hierarchy, holding aggregated data for a specific time window and level.
   - Its primary content is stored in the `content` field, which is of type `PageContent` (an enum):
     - `PageContent::Leaves(Vec<JournalLeaf>)`: Typically used for Level 0 pages. Stores the actual `JournalLeaf` objects recorded within the page's time window.
     - `PageContent::ThrallHashes(Vec<[u8; 32]>)`: Typically used for Level 1+ pages. Stores the page hashes of its finalized child pages. The nature of what these thrall hashes represent (e.g., direct aggregation of child Merkle roots, or roots of net-patch summaries) is determined by the `RollupContentType` defined in the `LevelRollupConfig` for the parent level.
   - Contains `merkle_root`: The root hash of a Merkle tree constructed over its `PageContent` (e.g., over leaf hashes for L0, or over thrall page hashes for L1+).
   - Is cryptographically linked to the previous page at the same hierarchical level via `prev_page_hash`.
   - Has its own unique `page_hash`, derived from its key fields, ensuring its integrity.
   - Manages its lifecycle including finalization (sealing) and participates in the roll-up process, where its summary (e.g., its `page_hash`) is incorporated into a parent page at the next higher level.
   - **Persistence**: `JournalPage` instances are serialized using a format like Bincode (potentially with compression like Zstd, Lz4, or Snappy based on configuration) and stored as individual files. These files typically use a `.cjt` extension and begin with a 6-byte header:
     - Bytes 0-3: Magic string `CJTP` (CivicJournal Time Page).
     - Byte 4: Format version (e.g., `1`).
     - Byte 5: Compression algorithm identifier (e.g., `0` for None, `1` for Zstd, corresponding to the `CompressionAlgorithm` enum).

3. **TimeHierarchy**
   - Manages the 7-level time hierarchy
   - Handles page creation and roll-up
   - Implements retention policies

4. **MerkleTree**
   - Provides cryptographic verification.
   - Supports efficient inclusion proofs. The `QueryEngine`'s `get_leaf_inclusion_proof` method dynamically constructs a `MerkleTree`. The tree is built using hashes derived from the `JournalPage`'s `PageContent` (i.e., from `leaf.leaf_hash` for `PageContent::Leaves`, or directly from the `[u8; 32]` hashes for `PageContent::ThrallHashes`).
   - For L0 pages, if the page containing the leaf is loaded, the full `JournalLeaf` object is directly available from `PageContent::Leaves`. The `get_leaf_inclusion_proof` method can then return this full leaf along with the proof.
   - The primary challenge for `load_leaf_by_hash` and `get_leaf_inclusion_proof` is efficiently locating the specific L0 page containing the target leaf, especially if it's not in an active page.
   - Enables secure state reconstruction.

## Configuration System

The configuration system provides a flexible and type-safe way to configure all aspects of the CivicJournal Time system. It's implemented in the `src/config/` directory with the following structure:

```
src/config/
├── mod.rs         # Main configuration module and public API
├── config.toml    # Default configuration values
├── error.rs       # Error types and handling
├── validation.rs  # Configuration validation logic
└── types.rs       # Type definitions and enums
```

### Configuration Structure

The configuration is defined in TOML format with the following main sections:

```toml
[time_hierarchy]
# Time levels configuration
levels = [
    { name = "minute", duration_seconds = 60 },
    { name = "hour", duration_seconds = 3600 },
    { name = "day", duration_seconds = 86400 },
    { name = "month", duration_seconds = 2592000 },
    { name = "year", duration_seconds = 31536000 },
    { name = "decade", duration_seconds = 315360000 },
    { name = "century", duration_seconds = 3153600000 }
]

[rollup]
# When to trigger roll-up operations
max_leaves_per_page = 1000
max_page_age_seconds = 3600  # 1 hour
force_rollup_on_shutdown = true

[storage]
type = "file"  # or "memory"
base_path = "./data"
max_open_files = 100

[compression]
enabled = true
algorithm = "zstd"  # or "lz4", "snappy", "none"
level = 3          # Compression level (1-22 for zstd)

[logging]
level = "info"     # "trace", "debug", "info", "warn", "error"
console = true
file = false
file_path = "./civicjournal.log"

[metrics]
enabled = true
push_interval_seconds = 60
endpoint = ""

[retention]
period_seconds = 0  # 0 = keep forever
enabled = false
cleanup_interval_seconds = 3600
```

### Core Components

1. **Main Configuration (`mod.rs`)**
   - Defines the `Config` struct with all configuration options
   - Implements loading from files, environment variables, and programmatic overrides
   - Provides validation of configuration values
   - Includes sensible defaults for all settings

2. **Error Handling (`error.rs`)**
   - Defines `ConfigError` enum with variants for different error cases
   - Implements `std::error::Error` for all error types
   - Provides helpful error messages and context

3. **Validation (`validation.rs`)**
   - Validates configuration values and relationships
   - Ensures time hierarchy levels are properly ordered
   - Validates storage and compression settings
   - Checks retention policies for consistency

4. **Type Definitions (`types.rs`)**
   - Defines enums for configuration options:
     - `StorageType`: Memory or file-based storage
     - `CompressionAlgorithm`: Supported compression algorithms
     - `LogLevel`: Logging verbosity levels
   - Implements parsing and display traits for all enums

### Configuration Loading

The configuration system supports multiple ways to load and override settings:

1. **Default Configuration**
   - Built-in defaults are compiled into the binary
   - Ensures the application always has valid configuration
   - Defined in `src/config/config.toml`

2. **File-based Configuration**
   - Loads from `config.toml` in the application directory
   - Can specify custom path via `Config::load("path/to/config.toml")`
   - Supports environment variable overrides (e.g., `CJ_STORAGE_TYPE=memory`)

3. **Programmatic Overrides**
   - All settings can be modified at runtime
   - Supports dynamic reconfiguration
   - Thread-safe for concurrent access

### Validation Rules

The configuration system enforces the following validation rules:

1. **Time Hierarchy**
   - Must have at least one time level
   - Level names must be unique
   - Durations must be in ascending order
   - Durations must be greater than zero

2. **Storage**
   - Valid storage types: "memory" or "file"
   - File storage requires a valid base path
   - Maximum open files must be greater than zero

3. **Compression**
   - Valid algorithms: "zstd", "lz4", "snappy", "none"
   - Zstd levels must be between 1-22
   - LZ4 levels must be between 0-16

4. **Retention**
   - If enabled, retention period must be greater than zero
   - Cleanup interval must be greater than zero

### Usage Example

```rust
use civicjournal_time::{Config, CJResult};

fn main() -> CJResult<()> {
    // Load configuration from default location
    let config = Config::load("config.toml")?;
    
    // Or create a custom configuration programmatically
    let custom_config = Config {
        storage: StorageConfig {
            r#type: "memory".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    
    Ok(())
}
```

### Error Handling

All configuration operations return `Result<T, ConfigError>` with detailed error information:

- File I/O errors
- TOML parsing errors
- Validation failures
- Type conversion errors

### Testing

The configuration system includes comprehensive unit tests for:
- Loading and parsing configuration files
- Validating configuration values
- Testing edge cases and error conditions
- Verifying default values

## Performance Characteristics

### Write Performance

- **Throughput**: Up to 10,000 writes/second (depends on hardware)
- **Latency**: <10ms for 95th percentile (SSD)
- **Batching**: Significant improvements with batch operations

### Read Performance

- **Point Queries**: <1ms for cached data
- **Range Queries**: Depends on data size and time range
- **State Reconstruction**: Linear with number of deltas

### Memory Usage

- **Page Cache**: Configurable maximum
- **Working Set**: Depends on access patterns
- **Leak Detection**: Built-in diagnostics

## Security Model

### Threat Model

1. **Data Tampering**:
   - Prevented by cryptographic hashing
   - Detected via Merkle proofs

2. **Data Leakage**:
   - File system permissions
   - Encryption at rest (future)

3. **Denial of Service**:
   - Rate limiting
   - Resource quotas

### Audit Logging

- Security-relevant events
- Configuration changes
- Access patterns

## Future Work

### Short-term

1. **Enhanced Query Language**
2. **Improved Documentation**
3. **Additional Storage Backends**

### Medium-term

1. **Distributed Deployment**
2. **Access Control**
3. **Encryption**

### Long-term

1. **Global Scale**
2. **Advanced Analytics**
3. **Blockchain Integration**

## Conclusion

CivicJournal-Time provides a robust foundation for building verifiable audit trails and time-series data management systems. Its modular design, strong focus on data integrity, and flexible configuration make it suitable for a wide range of applications.

For more details, see the [API Documentation](https://docs.rs/civicjournal-time) and [Developer Guide](DEVELOPER_GUIDE.md).

### Memory Management

1. **Page Caching**
   - LRU cache for frequently accessed pages
   - Configurable cache size limits

2. **Batch Processing**
   - Process leaves in batches
   - Reduce I/O overhead

3. **Lazy Loading**
   - Load pages on demand
   - Release unused resources

### Storage Optimization

1. **Compression**
   - Configurable compression algorithms
   - Balance between speed and size

2. **Serialization**
   - Efficient binary format
   - Support for schema evolution

3. **Indexing**
   - Fast lookups by time range
   - Efficient page location

## Security Considerations

1. **Data Integrity**
   - Cryptographic hashing of all entries
   - Merkle proofs for verification

2. **Access Control**
   - Configurable read/write permissions
   - Support for authentication

3. **Audit Trail**
   - Immutable record of all changes
   - Cryptographic signatures

## Extensibility

The architecture is designed to be extended in several ways:

1. **Custom Storage Backends**
   - Implement the `Storage` trait
   - Support for databases, cloud storage, etc.

2. **Custom Roll-up Logic**
   - Implement custom roll-up strategies
   - Support for domain-specific optimizations

3. **Plugins**
   - Extend functionality without modifying core
   - Support for custom metrics, exporters, etc.

4. **Application-Defined Triggers (Special Signals)**
    *   **Concept**: CivicJournal-Time allows consuming applications to define "special signals" or triggers based on application-specific events. These triggers can programmatically initiate core journaling operations like rollups or snapshots.
    *   **Purpose**:
        *   **Enhanced Integrity for Critical Events**: For significant application events (e.g., finalization of a vote, completion of a major transaction), an immediate rollup can be triggered. This ensures the leaf data representing the event is quickly incorporated into higher-level, aggregated, and cryptographically secured pages, making the record more robust and its evidence more broadly distributed.
        *   **Semantic Alignment**: Allows the journal's lifecycle events (rollups, snapshots) to align with meaningful milestones in the application's domain, rather than being driven solely by generic time intervals or data volume thresholds. For example, a specific date (e.g., fiscal year-end) could trigger a multi-level rollup to a "Yearly" page.
    *   **Mechanism**:
        *   Applications can "wire up" these triggers when integrating with CivicJournal-Time. This typically involves an API or configuration mechanism where the application registers specific events or conditions that, when met, will instruct the CivicJournal-Time instance to perform a designated action (e.g., `force_rollup(levels)`, `create_snapshot(params)`).
        *   This provides a powerful way to customize the journaling behavior to closely match the application's operational logic and data significance.

1. **Memory Usage**
   - Implement efficient memory pooling
   - Use streaming where possible
   - Minimize allocations

2. **I/O Optimization**
   - Batch I/O operations
   - Use async I/O where beneficial
   - Implement efficient serialization

3. **Concurrency**
   - Use appropriate synchronization primitives
   - Minimize lock contention
   - Support concurrent operations

## Security Considerations

1. **Data Integrity**
   - Cryptographic verification of all data
   - Secure storage of sensitive information
   - Protection against tampering

2. **Access Control**
   - Fine-grained permissions
   - Secure default configurations
   - Audit logging

3. **Cryptography**
   - Use modern, well-audited crypto primitives
   - Secure key management
   - Protection against side-channel attacks

## Time Hierarchy and Rollup Mechanism

The time hierarchy is implemented in `src/core/time_manager.rs` and manages:

1. **Page Creation and Management**:
   - Pages are created on-demand based on timestamp and level
   - Each page has a time window defined by its level's duration
   - Pages track their creation time, end time, and content hashes

2. **Rollup Process**:
   - Triggered when a page is finalized (either by size or age)
   - Aggregates hashes from child pages into parent pages
   - Recursively rolls up through the hierarchy as needed
   - Stops at the highest configured time level

3. **Active Page Management**:
   - Each level maintains at most one active page
   - Active pages accept new leaves until finalized
   - Page finalization occurs when:
     - Maximum leaves per page is reached, or
     - Page age exceeds maximum configured age

## Test Architecture

The test suite follows these patterns:

1. **Global State Management**:
   - Uses `SHARED_TEST_ID_MUTEX` for test isolation
   - `reset_global_ids()` ensures clean state between tests
   - All tests that modify global state must acquire the mutex

2. **Test Organization**:
   - Unit tests are co-located with their modules
   - Integration tests verify cross-component behavior
   - Test helpers create consistent test data

3. **Test Reliability**:
   - Tests are deterministic and independent
   - Time-based tests use controlled timestamps
   - All tests clean up after themselves

## Future Extensions

1. **Pluggable Storage**
   - Support for different storage backends
   - Cloud storage integration
   - Distributed storage support

2. **Advanced Querying**
   - Time-based queries
   - Content-based filtering
   - Aggregation support

3. **Replication**
   - Multi-master replication
   - Conflict resolution
   - Geo-distribution support

## Glossary

- **Chunk**: A collection of deltas within a specific time window
- **Delta**: A single change to the system state
- **Merkle Tree**: A tree of hashes used for data integrity verification
- **Time Window**: A specific period of time for organizing chunks
- **Snapshot**: A complete representation of the system state at a specific point in time
