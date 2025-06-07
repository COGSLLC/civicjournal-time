# Demo Mode Specification

This document outlines the **Demo Mode** for the Journal system. It provides scaffolding, configuration hints, and tips for developers to generate, explore, and validate 20 years of simulated journal data.

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

## 3. Configuration

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

## 4. Time Simulator

* **Mode**: Batch vs. Live

  * **Batch**: generate entire dataset in one run, then exit.
  * **Live**: pace generation by sleeping `rate.real_seconds_per_month` per simulated month.
* **Implementation**:

  * Compute total months.
  * Loop from `start_date` to `end_date`, advancing by one month per iteration.
  * Within each month, spawn `leaf_rate.per_real_second * rate` leaves.

## 5. Leaf Generator

* For each tick:

  1. Pick random user ID (`1..users`).
  2. Pick random container ID (`1..containers`).
  3. Generate content via lorem ipsum library.
  4. Random delta type: CREATE / EDIT / COMMENT.
  5. Call `create_leaf()` API.

* **Traceability**: embed tags like `[demo:<month>:<seq>]` in content to aid tracing.

## 6. Rollup Triggering

* After each leaf insertion, check all configured levels:

  * Size threshold for L0
  * Time threshold for L1/L2
* Invoke `seal_page(level)` when conditions are met.
* Support **burstiness** by injecting random spikes in `leaf_rate`.

## 7. Snapshot Generation

* At each interval defined by `snapshot.every` (e.g., yearly):

  * Call `create_snapshot(as_of_timestamp)`.
  * Log snapshot hashes.
  * Invoke **snap\_off** to archive lower rollup pages fully covered by this snapshot.

## 8. Explorer Interface

Provide two options:

### 8.1 CLI Tool

```
journal-demo explore \
  --container 5 \
  --as-of 2013-06-01 \
  --show-leafs \
  --show-rollup-chain
```

* Subcommands: `list-users`, `list-containers`, `get-state`, `trace-leaf <id>`.

### 8.2 Minimal Web UI

* Simple HTML + JS app served on `localhost:4000`
* Endpoints:

  * `/api/leafs?container=X&from=...&to=...`
  * `/api/rollups?level=0&...`
  * `/api/snapshots`
* Visualize via D3 or plain tables.

## 9. Database Isolation

* Use a dedicated Postgres database (e.g., `journal_demo`).
* Schema reset on startup (optional `--wipe` flag).
* Support `--persist` flag to keep data between runs.

## 10. Running the Demo

```bash
# Batch mode:
journal-demo run --mode batch

# Live mode:
journal-demo run --mode live --wipe

# Explore:
journal-demo explore --help
```

## 11. Tips & Best Practices

* **Seed control**: Fix `seed` for reproducible demos.
* **Logging**: Use structured logs (JSON) with demo tags.
* **Performance**: Increase DB pool size for high leaf rates.
* **Monitoring**: Expose Prometheus metrics for pages/sec.

## 12. Scaffold Hints

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

## 13. Extending Demo Mode

* Add custom event patterns (e.g., mass forks/merges).
* Support injecting failures to test recovery logic.
* Integrate with UI frameworks for richer visualization.
