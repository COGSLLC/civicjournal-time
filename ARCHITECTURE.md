# CivicJournal Time - Architecture Document

## Current Architecture (As of May 2024)

### Core Components

1. **Time Hierarchy System**
   - Manages time-based organization of data chunks
   - Handles chunk creation, sealing, and retention
   - Implements a hierarchical time-based storage model

2. **Chunk Management**
   - Handles storage and retrieval of data chunks
   - Manages chunk persistence and caching
   - Implements chunk eviction policies

3. **Configuration System**
   - Comprehensive configuration via TOML files
   - Supports multiple chunking strategies:
     - Time-based
     - Event count-based
     - Signal-based
     - Size-based
   - Flexible compression and persistence options

4. **Data Model**
   - Delta-based change tracking
   - Merkle tree for data integrity
   - Snapshot support for efficient state reconstruction

### Current Module Structure

```
src/
├── config.rs         # Configuration parsing and validation
├── error.rs          # Error types and conversions
├── ffi.rs            # Foreign Function Interface bindings
├── fork_detection.rs # Fork detection logic
├── health_monitor.rs # System health monitoring
├── journal_api.rs    # Main public API
├── lib.rs            # Library entry point
├── merkle_tree.rs    # Merkle tree implementation
├── recovery.rs       # Data recovery utilities
├── schema.rs         # Core data structures
├── snapshot.rs       # Snapshot management
└── time_hierarchy.rs # Time-based data organization
```

### Key Data Structures

1. **TimeHierarchy**
   - Manages the hierarchical organization of time-based chunks
   - Handles chunk creation, sealing, and retrieval
   - Implements retention policies

2. **TimeChunk**
   - Represents a chunk of time with associated deltas
   - Manages delta storage and retrieval
   - Handles serialization/deserialization

3. **Delta**
   - Represents a single change to the system state
   - Contains metadata about the change
   - Supports cryptographic verification

4. **MerkleTree**
   - Provides cryptographic integrity verification
   - Supports efficient delta validation
   - Enables secure state reconstruction

## Architectural Issues and Limitations

1. **Complexity in Time Hierarchy**
   - Overly complex time management logic
   - Tight coupling between time management and storage
   - Difficult to extend with new time-based behaviors

2. **Configuration Complexity**
   - Overly flexible configuration options
   - Complex validation logic
   - Difficult to reason about configuration interactions

3. **Error Handling**
   - Inconsistent error types and handling
   - Missing error contexts in many places
   - Complex error conversion logic

4. **Performance**
   - Potential bottlenecks in chunk management
   - Inefficient memory usage patterns
   - Suboptimal serialization/deserialization

## Proposed Architecture

### Core Principles

1. **Simplicity First**
   - Reduce unnecessary abstractions
   - Favor composition over inheritance
   - Make common cases easy, complex cases possible

2. **Type Safety**
   - Leverage Rust's type system
   - Minimize use of `unwrap()` and `expect()`
   - Use newtypes for domain concepts

3. **Modularity**
   - Clear module boundaries
   - Well-defined interfaces
   - Minimal cross-module dependencies

4. **Testability**
   - Design for testability
   - Support dependency injection
   - Include comprehensive test coverage

### Proposed Module Structure

```
src/
├── types/                # Centralized type definitions (single source of truth)
│   ├── mod.rs          # Main types module exports
│   ├── delta.rs        # Delta type definitions
│   ├── error.rs        # Error types and conversions
│   ├── ffi.rs          # FFI type definitions
│   ├── metrics.rs      # Metrics type definitions
│   ├── storage.rs      # Storage-related type definitions
│   └── time/           # Time-related types
│       ├── mod.rs      # Time module exports
│       ├── chunk.rs    # Time chunk definitions
│       ├── duration.rs # Duration handling
│       ├── level.rs    # Time level definitions
│       └── unit.rs     # Time unit definitions
│
├── api/               # Public APIs
│   ├── mod.rs         # Public API traits
│   ├── sync.rs        # Synchronous API implementation
│   └── async.rs       # Asynchronous API (feature-gated)
│
├── storage/           # Storage backends
│   ├── mod.rs         # Storage trait and implementations
│   ├── memory.rs      # In-memory storage backend
│   └── disk.rs        # Persistent storage backend
│
└── ffi/              # Foreign function interface
    ├── mod.rs        # Main FFI module
    ├── async_ffi.rs  # Async FFI bindings
    ├── c/           # C-specific FFI code
    └── wasm/        # WebAssembly-specific FFI code
    ├── mod.rs          # FFI utilities
    ├── c/             # C bindings
    └── wasm/          # WebAssembly bindings
```

### Key Architectural Changes

1. **Simplified Time Management**
   - Separate time windowing from storage
   - Use a more straightforward chunking strategy
   - Improve retention policy implementation

2. **Improved Configuration**
   - Simplify configuration options
   - Better validation with clearer error messages
   - Support environment variable overrides

3. **Enhanced Error Handling**
   - Unified error type with proper error contexts
   - Better error conversion between modules
   - More descriptive error messages

4. **Performance Optimizations**
   - Better memory management
   - Improved serialization/deserialization
   - More efficient chunk storage and retrieval

## Migration Strategy

1. **Phase 1: Foundation**
   - Set up new module structure
   - Implement core data structures
   - Create basic storage backend

2. **Phase 2: Core Functionality**
   - Implement time windowing
   - Add delta management
   - Implement basic API surface

3. **Phase 3: Advanced Features**
   - Add Merkle tree support
   - Implement snapshots
   - Add health monitoring

4. **Phase 4: Integration**
   - Implement FFI bindings
   - Add documentation and examples
   - Create migration guide

## Performance Considerations

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
