# CivicJournal Demo Mode

This document describes the new **Demo Mode** for CivicJournal. The CLI is now named `cj-demo` and provides commands to simulate years of history, inspect ledger state, and experiment with rollbacks.

## 1. CLI Overview

```
cj-demo
├─ simulate     # run a time-travel simulation
├─ state        # show reconstructed DB state at T
├─ revert       # rollback your real DB to state at T
├─ leaf         # examine individual leaves
├─ page         # examine individual pages
└─ nav          # interactive navigator (arrow-keys)
```

Each subcommand exposes options tailored to the operation. Only the scaffolding is currently implemented.

## 2. simulate

The `simulate` command generates synthetic fields and appends leaves over a span of time. Example:

```bash
cj-demo simulate \
  --container demoDB \
  --fields 50 \
  --duration 20y \
  --errors-parked 2 \
  --errors-malformed 1 \
  --start 2000-01-01T00:00:00Z \
  --seed 42
```

* `--container`: journal table or namespace
* `--fields`: number of fields to create
* `--duration`: total simulated span (`20y`, `6m`, `100d`, ...)
* `--errors-parked`: simulate transient errors (park then retry)
* `--errors-malformed`: simulate malformed payloads (park then drop)
* `--start`: simulation start timestamp
* `--seed`: RNG seed for reproducibility

During simulation the tool appends synthetic updates, logs transient and malformed attempts, and periodically rolls up pages. A final snapshot page is taken before closing the journal.

## 3. state

Print the reconstructed state of a container as-of a timestamp:

```bash
cj-demo state --container demoDB --as-of 2010-06-15T12:34:56Z
```

The command finds the highest rollup page covering the timestamp, applies finer patches, and outputs the resulting JSON map.

## 4. revert

Rollback a real database to the ledger state at a given time:

```bash
cj-demo revert \
  --container demoDB \
  --as-of 2005-01-01T00:00:00Z \
  --db-url postgres://…
```

The revert operation reconstructs the state, wipes the target table, inserts each record inside a transaction and logs the revert as a special leaf.

## 5. leaf & page

The `leaf` and `page` subcommands inspect individual ledger entries.

### leaf

* **list** – list all leaves for a container
* **show** – display one leaf with optional pretty JSON

### page

* **list** – list pages at a specific rollup level
* **show** – dump a single page in raw or structured form

## 6. nav

`nav` launches an interactive full‑screen browser using arrow keys to traverse leaves and pages. It shows payloads, hashes, and hierarchy relationships. Press `H` for help and `q` to quit.

## 7. Rationale

Demo Mode demonstrates CJ‑T’s guarantees without requiring a full application. Synthetic fields mimic a real schema, error parking shows auditability, and the state/revert commands prove you can time‑travel a database.

