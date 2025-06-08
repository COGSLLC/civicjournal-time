# Query System

The query system provides read access and verification utilities for data stored in the CivicJournal time-series ledger.  It is centred around the `QueryEngine` type which wraps a `StorageBackend` and a `TimeHierarchyManager`.

The following sections summarise the features available to users when querying a journal instance.

## Components

- **`QueryEngine`** (`src/query/engine.rs`)
  - `get_leaf_inclusion_proof_with_hint`: returns a Merkle proof that a particular `JournalLeaf` exists in a page, optionally using a `(level, page_id)` hint.
  - `reconstruct_container_state`: rebuilds a container's JSON state at a specific timestamp.
  - `get_delta_report`: lists all `JournalLeaf` changes for a container over a time window.
  - `get_delta_report_paginated`: like `get_delta_report` but returns a slice of results using `offset` and `limit`.
  - `get_page_chain_integrity`: checks that a sequence of pages link correctly and have valid Merkle roots.
- **Query types** (`src/query/types.rs`)
  - `QueryError`, `QueryPoint`, `LeafInclusionProof`, `ReconstructedState`, `DeltaReport`, `PageIntegrityReport`.
- **API integration**
  - The async (`src/api/async_api.rs`) and sync (`src/api/sync_api.rs`) `Journal` wrappers expose the above methods.
  - `src/ffi/c_ffi.rs` exposes a minimal C ABI (`civicjournal_get_container_state`) for Turnstile.

## Usage

### Rust (Async)
```rust
use civicjournal_time::api::async_api::Journal;
use chrono::Utc;

let cfg = civicjournal_time::Config::default();
let journal = Journal::new(&cfg).await?;
let proof = journal
    .get_leaf_inclusion_proof_with_hint(&leaf_hash, None)
    .await?;
let state = journal.reconstruct_container_state("container", Utc::now()).await?;
```

### Rust (Sync)
```rust
use civicjournal_time::api::sync_api::Journal;
use chrono::Utc;

let cfg = civicjournal_time::Config::default();
let journal = Journal::new(&cfg)?;
let report = journal.get_delta_report("container", start, end)?;
```

### C FFI
```c
// after calling civicjournal_init_default()
char *json = civicjournal_get_container_state("container", 1700000000);
// use json string ...
civicjournal_free_string(json);
```

## How It Works

`QueryEngine` uses the storage backend to load pages and leaves. It consults `TimeHierarchyManager` to enumerate active and finalized pages. For inclusion proofs it scans level‑0 pages to find the target leaf and constructs a `MerkleTree` on demand. State reconstruction and delta reports iterate pages chronologically applying leaf deltas. Page integrity checks recompute each page's Merkle root and verify linkage via `prev_page_hash`.

The async and sync APIs simply delegate to these methods. The FFI layer holds a global `Journal` instance and exposes a limited wrapper for container state queries which Turnstile can call.

## Current Limitations / Future Work

- `get_leaf_inclusion_proof_with_hint` still performs a linear scan of level‑0 pages when the hint does not match. Full indexing would improve performance.
- The FFI only exposes container state retrieval; additional query functions may be exported as needed.

The overall query interface is functional and can reconstruct state, produce delta reports, verify page integrity, and generate inclusion proofs, but efficiency improvements and broader FFI coverage remain open tasks.
