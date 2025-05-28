# CivicJournal Specification

## Overview

CivicJournal is a hierarchical time-series journal system that provides efficient historical state management through the use of merkle-chains and pages. Unlike blockchain-based systems that focus on blocks, CivicJournal uses a journal-based approach with merkle-chains to establish provenance and integrity of historical data.

## Core Concepts

### Journal Pages vs. Blocks

Traditional blockchain systems organize data into "blocks" that are linked together in a chain. CivicJournal takes a different approach by organizing data into "journal pages" which are more flexible and better suited for time-series data:

1. **Journal Pages**: Self-contained units of time-series data with clear start and end timestamps
2. **Delta Entries**: Individual state changes recorded within pages
3. **Hierarchical Organization**: Pages are organized in a fractal-like hierarchy based on time spans

Unlike blocks that are primarily focused on consensus mechanisms, journal pages are optimized for efficient storage, retrieval, and verification of historical state.

### Merkle-Chains vs. Blockchains

CivicJournal uses merkle-chains rather than blockchains:

1. **Merkle-Chains**: A linked sequence of cryptographic hashes forming a chain of provenance
2. **Hash Linking**: Each delta entry contains a hash of its data and a reference to the previous entry's hash
3. **Verifiable History**: The chain of hashes provides cryptographic proof of the sequence and integrity of entries

The key difference is that merkle-chains in CivicJournal are focused on provability of history rather than distributed consensus, making them more efficient for centralized or semi-centralized applications.

## Architecture

CivicJournal is structured around several key components:

### 1. Time Hierarchy

The Time Hierarchy is a fractal-like structure that organizes journal pages at different time granularities:

- **Level 0**: Minute-level pages (60 seconds)
- **Level 1**: Hour-level pages (3600 seconds)
- **Level 2**: Day-level pages (86400 seconds)
- **Level 3**: Month-level pages (2592000 seconds)
- **Level 4**: Year-level pages (31536000 seconds)
- **Level 5**: Decade-level pages (315360000 seconds)
- **Level 6**: Century-level pages (3153600000 seconds)

Each level contains pages that cover a specific time window, with higher levels aggregating data from lower levels.

### 2. Journal Pages

Journal pages (implemented as `TimeChunk` in the code) have the following properties:

- **Time Window**: Each page covers a specific time window (start_time to end_time)
- **Delta Entries**: Collection of individual state changes within the time window
- **Summary**: A rolled-up representation of the page's content
- **Hash**: Cryptographic hash of the page's content
- **Thralls**: References to child pages (forming a hierarchical structure)

### 3. Delta Entries

Delta entries (implemented as `Delta` in the code) represent individual state changes:

- **Timestamp**: The exact time the delta was created
- **Data**: The actual change data (serialized)
- **Hash**: Cryptographic hash of this delta
- **Previous Hash**: Hash of the previous delta (forming a merkle-chain)

## Integrity Verification

CivicJournal provides strong guarantees for data integrity through:

1. **Hash Chaining**: Each delta entry references the hash of the previous entry
2. **Page Hashing**: Each journal page has a hash derived from its content and child pages
3. **Hierarchy Verification**: The entire hierarchy can be verified for integrity

This provides cryptographic proof that the history has not been tampered with.

## Journal Operations

### Adding Entries

When a new delta entry is added:

1. The appropriate journal page for its timestamp is located or created
2. The entry is added to the page, with a reference to the previous entry's hash
3. The page's hash is updated
4. Parent pages in the hierarchy are updated as needed

### Querying History

CivicJournal supports efficient queries:

1. **Time-Based Queries**: Find entries within a specific time window
2. **Level-Based Queries**: Query at different levels of granularity
3. **Summary Queries**: Get summarized data without retrieving all individual entries

### Verification

The entire journal or specific pages can be verified for integrity:

1. **Delta Chain Verification**: Checks that the hash chain of deltas is intact
2. **Page Verification**: Verifies that each page's hash matches its content
3. **Hierarchy Verification**: Verifies the integrity of the entire hierarchy

## File Format and Storage

CivicJournal pages are stored on disk with the following characteristics:

1. **Individual Files**: Each journal page is stored in a separate JSON file
2. **Directory Structure**: Files are organized in directories based on their level in the hierarchy
3. **Marker File**: A `.civicjournal-time` file marks the root directory of a CivicJournal

## Windows Compatibility

CivicJournal is designed to be fully compatible with Windows systems:

1. **File Locking Handling**: Implements retry logic for file operations to handle Windows-specific file locking
2. **Console Support**: Windows console integration with title setting and ANSI color support
3. **Graceful Shutdown**: Proper handling of termination signals on Windows

## Implementation Details

### Core Components

- **TimeHierarchy**: The main struct that manages the hierarchy of journal pages
- **TimeChunk**: Represents a journal page with a specific time window
- **Delta**: Represents an individual state change entry
- **HierarchyConfig**: Configuration for the time hierarchy levels

### Error Handling

CivicJournal includes robust error handling for:

- **Verification Errors**: When integrity verification fails
- **I/O Errors**: When file operations fail
- **Serialization Errors**: When data serialization/deserialization fails
- **Invalid Time Ranges**: When time parameters are invalid

## Command-Line Interface

CivicJournal provides a CLI with several commands:

- **verify**: Verify the integrity of a journal
- **query**: Query the journal for entries within a time range
- **serve**: Start a web server for interacting with the journal
- **stats**: Display statistics about the journal

## Web API

When the web feature is enabled, CivicJournal provides a REST API for:

- Querying journal entries
- Adding new entries
- Verifying journal integrity
- Retrieving journal statistics

## Use Cases

CivicJournal is well-suited for:

1. **Audit Trails**: Recording and verifying sequences of events
2. **Historical State Management**: Efficiently storing and querying historical states
3. **Data Provenance**: Establishing the origin and history of data
4. **Time-Series Analysis**: Analyzing data patterns over time at different granularities
5. **Compliance Records**: Maintaining tamper-evident records for regulatory purposes

## Future Directions

Planned enhancements to CivicJournal include:

1. **Distributed Journal**: Support for distributed journal pages across multiple nodes
2. **Compression**: Advanced compression techniques for journal pages
3. **Pruning**: Selective pruning of historical data while maintaining integrity
4. **Streaming**: Real-time streaming of journal entries
5. **Advanced Queries**: More sophisticated query capabilities including filtering and aggregation

## Conclusion

CivicJournal provides a flexible, efficient, and secure approach to historical state management through its journal-based architecture using merkle-chains and hierarchical pages. By focusing on journal pages rather than blocks, it offers advantages in terms of scalability, query efficiency, and integration with existing systems.
