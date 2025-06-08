# Demo Mode Specification

This document outlines the **Demo Mode** for the Journal system. It provides instructions for generating, exploring, and validating years of simulated journal data.

## 1. Overview

* **Purpose**: Showcase rollup, snapshot, and exploration features in a safe, isolated environment.
* **Goals**:

  * Rapidly generate realistic journal activity over a 20‑year span.
  * Trigger rollups and snapshots at configurable intervals.
  * Offer interactive or CLI‑based exploration of history and state.

## 2. Prerequisites

* Rust 1.68+ (or your chosen language runtime)
* PostgreSQL 12+ instance (dedicated schema/database)
* Optional: Docker Compose setup for isolation
* Dependencies:

  * CLI parser (e.g., Clap for Rust)
  * Async/runtime (e.g., Tokio)
  * DB client (e.g., SQLx or Diesel)
  * Faker or lorem ipsum crate for fake content

## 3. Quick Start

Build and run the simulator with the optional `demo` feature:

```bash
cargo run --features demo --bin journal-demo -- run --mode batch
```

This generates demo data according to the `[demo]` section in `Journal.toml`.
Base application settings are loaded from `config.toml` if it exists; otherwise
default values are used.

## 4. Configuration

Add a `[demo]` section to your `Journal.toml`:

```toml
[demo]
start_date = "2005-01-01"
end_date   = "2025-01-01"
rate       = { real_seconds_per_month = 0.5 }
seed       = 42
users      = 10
containers = 20
leaf_rate  = { per_real_second = 10 }
rollup     = { l0_size=1000, l1_time="1h", l2_time="1d" }
snapshot   = { every="P1Y" }
```

* **start\_date/end\_date**: demo timeline bounds.
* **rate**: acceleration factor.
* **leaf\_rate**: leaves generated per tick.
* **rollup** thresholds\*\*: override production defaults for demo.
* **snapshot.every**: ISO‑8601 interval for snapshots.

## 5. Time Simulator

* **Mode**: Batch vs. Live

  * **Batch**: generate entire dataset in one run, then exit.
  * **Live**: pace generation by sleeping `rate.real_seconds_per_month` per simulated month.
* **Implementation**:

  * Compute total months.
  * Loop from `start_date` to `end_date`, advancing by one month per iteration.
  * Within each month, spawn `leaf_rate.per_real_second * rate` leaves.

## 6. Leaf Generator

* For each tick:

  1. Pick random user ID (`1..users`).
  2. Pick random container ID (`1..containers`).
  3. Generate content via lorem ipsum library.
  4. Random delta type: CREATE / EDIT / COMMENT.
  5. Call `create_leaf()` API.

* **Traceability**: embed tags like `[demo:<month>:<seq>]` in content to aid tracing.

## 7. Rollup Triggering

* After each leaf insertion, check all configured levels:

  * Size threshold for L0
  * Time threshold for L1/L2
* Invoke `seal_page(level)` when conditions are met.
* Support **burstiness** by injecting random spikes in `leaf_rate`.

## 8. Snapshot Generation

* At each interval defined by `snapshot.every` (e.g., yearly):

  * Call `create_snapshot(as_of_timestamp)`.
  * Log snapshot hashes.
  * Invoke **snap\_off** to archive lower rollup pages fully covered by this snapshot.

## 9. Explorer Interface

Provide two options:

### 9.1 CLI Tool

```
journal-demo explore \
  --container 5 \
  --as-of 2013-06-01 \
  --show-leafs \
  --show-rollup-chain
```

* Subcommands: `list-users`, `list-containers`, `get-state`, `trace-leaf <id>`.

### 9.2 Minimal Web UI

* Simple HTML + JS app served on `localhost:4000`
* Endpoints:

  * `/api/leafs?container=X&from=...&to=...`
  * `/api/rollups?level=0&...`
  * `/api/snapshots`
* Visualize via D3 or plain tables.

## 10. PostgreSQL Integration

Demo Mode can run entirely using the file storage backend, but to better mimic a
production deployment it should also insert each generated payload into a real
PostgreSQL table. This allows the turnstile trigger in
[`TURNSTILE.md`](TURNSTILE.md) to validate that the ledger and the database stay
in sync.

* Launch a dedicated Postgres instance (e.g. `journal_demo`). If `database_url`
  is omitted the demo first tries Docker and falls back to an embedded server
  downloaded at runtime. If both fail you'll be asked to install PostgreSQL
  locally. To run manually use a small Docker Compose file:

  ```yaml
  services:
    db:
      image: postgres:15
      environment:
        POSTGRES_DB: journal_demo
        POSTGRES_USER: demo
        POSTGRES_PASSWORD: demo
      ports:
        - "5432:5432"
  ```

* On startup, create a `transactions` table and install the trigger from the
  sample in `TURNSTILE.md`.
* Schema reset is controlled by the optional `--wipe` flag.
* Use the `[demo]` section in `Journal.toml` to provide a database URL
  (`postgres://demo:demo@localhost:5432/journal_demo`).
* When generating leaves, insert a row into the table in addition to appending
  to the journal. This makes it possible to search records forward or backward
  directly in SQL and cross-check with reconstructed snapshots.
* Support a `--persist` flag to keep data between runs.

## 11. Running the Demo

```bash
# Batch mode:
journal-demo run --mode batch

# Live mode:
journal-demo run --mode live --wipe

# Explore:
journal-demo explore --help
```

## 12. Tips & Best Practices

* **Seed control**: Fix `seed` for reproducible demos.
* **Logging**: Use structured logs (JSON) with demo tags.
* **Performance**: Increase DB pool size for high leaf rates.
* **Monitoring**: Expose Prometheus metrics for pages/sec.

## 13. Scaffold Hints

* Directory layout:

  * `demo/`

    * `mod.rs` (entrypoint)
    * `config.rs` (parse `[demo]` section)
    * `simulator.rs` (time loop)
    * `generator.rs` (leaf & patch logic)
    * `rollup.rs`, `snapshot.rs`
    * `explorer.rs` (CLI & HTTP handlers)
  * `demo/tests/` (end-to-end smoke tests)

* Use feature flags: `--features demo` to include this module only in dev builds.

## 14. Extending Demo Mode

* Add custom event patterns (e.g., mass forks/merges).
* Support injecting failures to test recovery logic.
* Integrate with UI frameworks for richer visualization.
