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

Below is a **sketch of a “demo‐mode” CLI** that would let you:

1. **Simulate** 20 years of adds/updates (with errors)
2. **Query** or **revert** your “database” to any point in time
3. **Inspect** leaves and pages on the fly (with an interactive navigator)

You can use this as the blueprint for your `journal-demo` (or `cj-cli`) program.

---

## 1. Top-level CLI layout

```
cj-demo
├─ simulate     # run a time-travel simulation
├─ state        # show reconstructed DB state at T
├─ revert       # rollback your real DB to state at T
├─ leaf         # examine individual leaves
├─ page         # examine individual pages
└─ nav          # interactive navigator (arrow-keys)
```

---

## 2. simulate: generate 50 fields & 20 years of history

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

* `--container`: journal “table” or namespace
* `--fields`: number of fields (columns) to create
* `--duration`: total simulated span (e.g. `20y`, `6m`, `100d`)
* `--errors-parked`: simulate N transient errors (park + retry)
* `--errors-malformed`: simulate N malformed payloads (park then drop)
* `--start`: simulation start timestamp
* `--seed`: RNG seed (for reproducibility)

**What happens under the hood?**

1. Generate `fields = ["field1",…"field50"]`.
2. For each field, pick a random timestamp between `start` and `start + duration`, and call:

   ```rust
   let ticket = journal.turnstile_append(container, &payload, timestamp);
   db.write_with_ticket(ticket)?; // simulate occasional errors
   ```
3. Periodically (e.g. every simulated day), roll up pages in the 7‐level hierarchy.
4. When simulating an error:

   * **Transient**: park in “parking\_lot” queue, retry later (logs both append and retry)
   * **Malformed**: park then drop (log both the append and the drop decision)
5. At the end, take one final snapshot page and close the journal.

---

## 3. state: get DB state as-of a timestamp

```bash
cj-demo state \
  --container demoDB \
  --as-of 2010-06-15T12:34:56Z
```

* Reconstructs your “table” by:

  1. Finding the highest‐level page that covers `≤ as-of`
  2. Applying any finer patches or leaves up to `as-of`
  3. Printing the resulting JSON map
* **Output**: pretty-printed JSON of `{"field1": "...", …}`

---

## 4. revert: roll your real database back

```bash
cj-demo revert \
  --container demoDB \
  --as-of 2005-01-01T00:00:00Z \
  --db-url postgres://… 
```

* Uses the same reconstruction logic but then:

  1. Opens a DB transaction
  2. Deletes all rows in `your_table`
  3. Inserts each reconstructed record
  4. Commits and logs `revert` as a special CJ-T leaf
* **Safety**: wraps in a transaction so you can abort if anything goes wrong.
* **Implemented**: the `revert` command now connects to PostgreSQL, truncates the
  target table, inserts each `field`/`value` pair from the reconstructed JSON, and
  records the revert as a new leaf in the journal.

---

## 5. leaf & page: inspect individual entries

### leaf subcommands

* **List**:

  ```bash
  cj-demo leaf list --container demoDB
  ```

  Prints:

  ```
  ID   Timestamp               Hash
  1    2000-02-10T14:22:05Z    a1b2c3…
  2    2000-05-18T07:11:42Z    d4e5f6…
  …
  ```

* **Show**:

  ```bash
  cj-demo leaf show \
    --container demoDB \
    --leaf-id 17 \
    --pretty-json
  ```

  Outputs full payload, metadata, inclusion proof, etc.

### page subcommands

* **List** pages at a level:

  ```bash
  cj-demo page list \
    --container demoDB \
    --level 2
  ```

  Shows all pages of rollup‐level 2.

* **Show** one page:

  ```bash
  cj-demo page show \
    --container demoDB \
    --page-id 42 \
    --raw
  ```

  Dumps the Merkle root, child hashes, net patch (if any), and header bytes.

---

## 6. nav: interactive arrow-key explorer

Run:

```bash
cj-demo nav --container demoDB
```

You’ll get a full-screen CLI UI (using a library like `crossterm`):

```
┌────────────────────────────────────┐
│ Container: demoDB                 │
│ Current: leaf #17 @ 2000-05-18…   │
│ [← prev leaf] [→ next leaf]       │
│ [↑ parent page] [↓ child page]    │
└────────────────────────────────────┘
 Payload: {"field7":"updated…"}
  Hash: d4e5f6…
  …
```

* **←/→** keys step through leaves in chronological order.
* **↑/↓** keys move you up/down the page hierarchy (level 0 → level 1 → … → snapshot).
* **q** to quit.
* Press **H** for help on additional commands (search by hash, jump to timestamp, toggle raw header view).

---

## 7. Why this works for demonstrating CJ-T

* **Synthetic fields** let you simulate a “real” database schema without needing Postgres.
* **Turnstile integration** shows how CJ-T guarantees every write is logged (even on errors).
* **Error parking** proves you get full auditing of retries and malformed payloads.
* **State & revert** commands let you prove you can “time-travel” your DB.
* **Inspect & nav** showcase the underlying ledger: leaves, rollups, Merkle proofs, and snapshots.

With these commands in place, you can **walk anyone through**:

1. “Here’s when each field was first created.”
2. “Here’s the net effect after 5 years.”
3. “Here’s every attempt (including errors) the system made.”
4. “Let’s jump into the page hierarchy and verify the Merkle roots.”
5. “Now revert the DB to exactly that snapshot.”

All from a **pure CLI**, no GUI needed—just your terminal, CJ-T, and a willingness to explore.

## 8. Retro TUI Interface and Database Tools

Demo Mode centers around a throwback text interface inspired by 1980s terminals. When you launch `cj-demo nav` the screen fills with a bordered layout reminiscent of classic bulletin board systems. Navigation relies entirely on the arrow keys:

The TUI uses a blue background with white and gray text to evoke the feel of old bulletin board systems. It attempts to expand the terminal to at least 80×20 characters so the layout fits comfortably, but you can still resize the window to any larger dimension, including full screen.

* **←/→** cycle through leaves in chronological order.
* **↑/↓** move between parent and child pages.
* **Enter** expands a focused item (leaf or page) to show raw JSON and metadata.
* **H** opens a help pane describing available commands.
* **Q** quits the browser.

Inside the TUI you can perform common database operations without leaving the interface:

* Press **S** to show the container state. The TUI prompts you for a timestamp
  and displays a template like `YYYY-MM-DDTHH:MM:SSZ` so you know the expected
  format.
* Press **R** to revert a connected database to that state (confirmation required).
* Press **F** to search leaves by hash or timestamp.
* Press **D** to dump the current page or leaf to a file for offline analysis.

This lightweight interface requires only a terminal emulator yet gives full access to the journal. Use it to demo time‑travel queries, verify Merkle proofs, and even roll your database backward and forward interactively.
