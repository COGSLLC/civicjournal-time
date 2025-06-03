**Rollup Mechanism Specification**

---

### 1. Overview & Purpose

The rollup mechanism is responsible for grouping, aggregating, and hierarchically folding raw “leaf” events (deltas) into progressively coarser epochs (minutes → hours → days → …). Its goals are:

1. Ensure every raw change (leaf) is eventually committed into a Merkle‐rooted “page” at Level 0.
2. Automatically aggregate Level N pages into Level N+1 when either:
   • The number of items (size) in a page reaches a configured threshold, or
   • The page’s age (as measured from its creation timestamp to the triggering leaf’s timestamp) exceeds a configured maximum.
3. Discard any empty pages (pages that contain no content) to save storage.
4. Propagate a “net patch” or page hash (plus window metadata) upward into the next level’s active page, recursively.
5. Record, for each finalized page, the time window it covers, Merkle root (or net state map), and first/last child references to enable forward/backward reconstruction.

---

### 2. Terminology & Definitions

* **Leaf (JournalLeaf):** A single raw event or delta, represented as a struct (e.g. `{ timestamp, object_id, field_name, old_value, new_value }`).

* **Level N page (“PageN”):** A container that aggregates either raw leaves (when N = 0) or “child items” (when N > 0). Each page has:
  • `level: u8` – its hierarchy level (0 for raw leaves, 1 for first aggregation, etc.).
  • `page_id: u64` – a monotonically increasing identifier for that page within its level.
  • `creation_timestamp: DateTime<Utc>` – the window’s start (floor(T, epoch\_length\_N)).
  • `window_end: DateTime<Utc>` – equals `creation_timestamp + epoch_length_N`.
  • `content` – either:
  – For Level 0: `Vec<JournalLeaf>` (all raw deltas in that epoch).
  – For Level N>0: either `Vec<ChildHash>` (if storing Merkle hashes) or `HashMap<ObjectID, FieldPatch>` (if storing net state patches).
  • `merkle_root: String` – cryptographic root of `content` (if using Merkle).
  • `first_child_timestamp: DateTime<Utc>` – timestamp of the earliest item in `content`.
  • `last_child_timestamp: DateTime<Utc>` – timestamp of the latest item in `content`.

* **Epoch Length (per level):** A fixed duration in seconds, defined in configuration. For Level 0, it might be 60 seconds; for Level 1, 3600 seconds; for Level 2, 86400 seconds; etc.

* **Max Items per Page (per level):** An integer threshold. When a page’s content length ≥ this threshold, it must finalize immediately.

* **Max Age per Page (per level):** An integer (seconds). When `(trigger_timestamp – creation_timestamp) ≥ max_age`, the page must finalize (provided it is nonempty).

* **Active Pages:** An in‐memory map `active_pages: HashMap<level_u8, Page>` that holds exactly one open (unfinalized) page per level (if it exists).

* **Trigger Timestamp:** The `JournalLeaf.timestamp` of the current leaf that caused a child page to finalize and roll up. At Level 0 it’s the new leaf’s timestamp; at higher levels it’s the same triggering timestamp passed from the child.

---

### 3. Configuration Parameters

All “Level N” parameters are read from a `config.toml` (or equivalent) under `[time_hierarchy]` and `[rollup]` sections:

```toml
[time_hierarchy]
levels = [
  { level = 0, epoch_seconds = 60   },   # Level 0 pages cover 60 seconds (minutes)
  { level = 1, epoch_seconds = 3600 },   # Level 1 pages cover 1 hour
  { level = 2, epoch_seconds = 86400 },  # Level 2 pages cover 1 day
  { level = 3, epoch_seconds = 2592000 },# Level 3 pages cover 30 days (month)
  # ... additional levels as needed ...
]

[rollup]
# same index ordering as `levels[]`
max_leaves_per_page = [1000, 500, 200, 100]        # index 0→L0, 1→L1, 2→L2, 3→L3
max_page_age_seconds = [60, 3600, 86400, 2592000]   # matching epoch lengths
```

**Notes:**

* The `epoch_seconds` and `max_page_age_seconds` need not be identical, but typically they match or exceed.
* `max_leaves_per_page[N]` — the number of items that force size‐based rollup at Level N. For N = 0, that’s raw leaves; for N > 0, that’s child‐level aggregates (hashes or patches).

---

### 4. Data Structures

1. **JournalLeaf (Level 0 item)**

   ```rust
   struct JournalLeaf {
     timestamp: DateTime<Utc>,
     object_id: String,
     field_name: String,
     old_value: serde_json::Value,
     new_value: serde_json::Value,
   }
   ```
2. **LevelNPage**

   ```rust
   struct LevelPage {
     level: u8,
     page_id: u64,
     creation_timestamp: DateTime<Utc>,
     window_end: DateTime<Utc>,
     content: PageContent,   // see enum below
     merkle_root: Option<String>,
     first_child_timestamp: Option<DateTime<Utc>>,
     last_child_timestamp: Option<DateTime<Utc>>,
   }

   enum PageContent {
     /// Level 0: raw leaves
     Leaves(Vec<JournalLeaf>),
     /// Level N>0: either
     ///   * Child hashes (Merkle)
     ///   * Or net state patches (object→final value)
     ChildHashes(Vec<String>),              // if using Merkle hash propagation
     NetPatches(HashMap<String, HashMap<String, serde_json::Value>>),
   }
   ```
3. **TimeHierarchyManager**

   ```rust
   struct TimeHierarchyManager {
     config: Config,  // holds epoch lengths, thresholds, etc.
     active_pages: HashMap<u8, LevelPage>,  
     next_page_ids: Vec<u64>,  // next page_id counter for each level
   }
   ```

---

### 5. API Surface (Core Methods)

1. **add\_leaf(leaf: \&JournalLeaf) → Result<(), CJError>**

   * Called whenever a new raw delta is generated.
   * Inserts `leaf` into the active Level 0 page (creating it if missing).
   * After insertion, checks size/age of Level 0. If finalization is needed, triggers `perform_rollup(0, leaf.timestamp)`.

2. **perform\_rollup(level: u8, trigger\_timestamp: DateTime<Utc>) → Result<(), CJError>**

   * Finalizes the active page at `level` (if exists and nonempty), persists it to disk, and merges its payload (child hashes or net patches) into Level N+1.
   * Recurses upward so that parent pages also finalize if they meet size/age conditions.

3. **get\_or\_create\_active\_page(level: u8, trigger\_timestamp: DateTime<Utc>) → \&mut LevelPage**

   * Returns a mutable reference to the active page at `level` whose `creation_timestamp` window covers `trigger_timestamp`.
   * If no such page exists, it:

     1. Calculates `window_start = floor(trigger_timestamp, epoch_seconds[level])`.
     2. Sets `window_end = window_start + epoch_seconds[level]`.
     3. Allocates a new `page_id = next_page_ids[level]`, increments the counter.
     4. Inserts a fresh `LevelPage` into `active_pages`.

4. **persist\_page(page: \&LevelPage) → Result<(), CJError>**

   * Serializes the finalized page (level, page\_id, window\_start, window\_end, merkle\_root or net patches, first/last child timestamps) into a JSON (or chosen format) file on disk.
   * Directory structure:

     ```
     ./journal/
       level_0/
         YYYY-MM-DD/
           HH-MM.json     # for minute‐level pages, e.g. 2025-06-03/12-34.json
       level_1/
         YYYY-MM-DD/
           HH.json        # for hour‐level pages, e.g. 2025-06-03/12.json
       level_2/
         YYYY-MM-DD.json  # for day‐level pages, e.g. 2025-06-03.json
       level_3/
         YYYY-MM.json     # for month‐level pages, e.g. 2025-06.json
       …  
     ```

---

### 6. Detailed Workflow

#### 6.1 Level 0: Handling a New Leaf

1. **Caller invokes `add_leaf(&leaf)`**

2. **Ensure active L0 page exists**

   * Let `T = leaf.timestamp`.
   * If `active_pages` does not contain key `0`, or if `T ≥ active_pages[0].window_end`, then create a new L0 page via `get_or_create_active_page(0, T)`.

3. **Append leaf to L0 page**

   ```rust
   let page0 = manager.get_or_create_active_page(0, leaf.timestamp)?;
   if let PageContent::Leaves(ref mut vec) = page0.content {
       vec.push(leaf.clone());
       // Update first/last child timestamps:
       page0.first_child_timestamp = page0.first_child_timestamp.or(Some(leaf.timestamp));
       page0.last_child_timestamp = Some(leaf.timestamp);
   }
   ```

4. **Recompute Merkle root or build net patch**

   * If using Merkle:

     ```rust
     let hashes: Vec<String> = page0.content
        .iter()
        .map(|leaf| sha256(serialize(leaf)))
        .collect();
     page0.merkle_root = Some(compute_merkle_root(&hashes));
     ```
   * If using net patches:

     ```rust
     let mut patch_map = HashMap::new();
     for leaf in &leaves {
        patch_map
           .entry(leaf.object_id.clone())
           .or_insert_with(HashMap::new)
           .insert(leaf.field_name.clone(), leaf.new_value.clone());
     }
     page0.content = PageContent::NetPatches(patch_map);
     page0.merkle_root = Some(sha256(serialize(&patch_map))); // optional
     ```

5. **Check rollup conditions for L0**

   ```rust
   let age_seconds = (leaf.timestamp - page0.creation_timestamp).num_seconds();
   let size_condition = page0.content.len() >= config.max_leaves_per_page[0];
   let age_condition = age_seconds >= config.max_page_age_seconds[0] && !page0.content.is_empty();
   if size_condition || age_condition {
      // Finalize L0 page
      let finalized_page = page0.clone();
      manager.active_pages.remove(&0);
      persist_page(&finalized_page)?;
      // Propagate upward (use leaf.timestamp as trigger)
      manager.perform_rollup(0, leaf.timestamp)?;
   }
   ```

   * **If `page0.content.is_empty()` (no leaves), and `age_condition` triggers**, simply drop the page (remove from `active_pages`) and do **not** call `perform_rollup(0, …)`.

---

#### 6.2 Level N > 0: `perform_rollup` & Cascading

```rust
async fn perform_rollup(&mut self, finalized_level: u8, trigger_ts: DateTime<Utc>) -> Result<(), CJError> {
    let parent_level = finalized_level + 1;
    // If no higher level is configured, stop:
    if parent_level >= self.config.levels.len() {
      return Ok(());
    }

    // 1. Load the finalized child page we just persisted.
    //    We need its content (hash or net patch) and its child window timestamps.
    let child_page = load_page_from_disk(finalized_level, page_id).unwrap();

    // 2. Build “child payload” for parent:
    //    a) If Merkle‐based: payload_hash = child_page.merkle_root.unwrap()
    //    b) If net‐patch‐based: payload_patch = child_page.net_patches.clone()

    // 3. Get or create the active parent page that covers trigger_ts:
    let mut parent_page = self.get_or_create_active_page(parent_level, trigger_ts).await?;

    // 4. Merge child payload into parent_page.content:
    match (&mut parent_page.content, &child_payload) {
      (PageContent::ChildHashes(hashes), Payload::Hash(child_hash)) => {
        hashes.push(child_hash.clone());
      }
      (PageContent::NetPatches(ref mut parent_map), Payload::Patch(child_map)) => {
        for (obj, fields) in child_map {
          for (field, value) in fields {
            parent_map
              .entry(obj.clone())
              .or_insert_with(HashMap::new)
              .insert(field.clone(), value.clone());
          }
        }
      }
      _ => { /* impossible: content type matches level */ }
    }

    // 5. Update parent_page.first/last_child_timestamp:
    parent_page.first_child_timestamp = parent_page.first_child_timestamp.or(Some(child_page.first_child_timestamp.unwrap()));
    parent_page.last_child_timestamp = Some(child_page.last_child_timestamp.unwrap());

    // 6. Recompute parent_page.merkle_root (over child hashes or hashed patches):
    parent_page.merkle_root = Some(compute_merkle_over_content(&parent_page.content));

    // 7. Check rollup conditions for parent:
    let age_secs = (trigger_ts - parent_page.creation_timestamp).num_seconds();
    let size_condition = parent_page.content.len() >= self.config.max_leaves_per_page[parent_level];
    let age_condition = age_secs >= self.config.max_page_age_seconds[parent_level] && !parent_page.content.is_empty();

    if size_condition || age_condition {
       // Finalize parent page
       let finalized_parent = parent_page.clone();
       self.active_pages.remove(&parent_level);
       persist_page(&finalized_parent)?;
       // Recurse upward
       self.perform_rollup(parent_level, trigger_ts).await?;
    }
    // If parent_page.content.is_empty() && age_condition, simply discard without recursion.
    Ok(())
}
```

**Notes on recursion and empty‐page handling:**

* Always pass the **same `trigger_ts`** (the original leaf timestamp) upward so that “parent age” = `(leaf_ts – parent.creation_timestamp)`.
* If a page is empty when its age triggers, just drop it (do not call recursion).
* If size triggers and content is nonempty, finalize and recurse.

---

### 7. Persistence & Directory Structure

When finalizing a page at Level N, write a JSON file with this schema:

```jsonc
{
  "level": N,
  "page_id": 42,
  "window_start": "2025-06-03T12:00:00Z",
  "window_end":   "2025-06-03T13:00:00Z",
  "merkle_root":  "abcd1234….",
  "first_child_ts": "2025-06-03T12:00:15Z",
  "last_child_ts":  "2025-06-03T12:59:59Z",
  // Then either:
  "child_hashes": [ "hash_of_L(N-1)_page1", "hash_of_L(N-1)_page2", … ]

  // Or, if using net patches:
  "net_patches": {
    "objectA": { "field1": 123, "field2": "foo" },
    "objectB": { "fieldX": true }
  }
}
```

Place this file under:

```
./journal/
  level_N/
    <path_based_on_window_start>/
      <page_id>.json
```

* For N = 0 (e.g. minute‐level), `path_based_on_window_start = YYYY-MM-DD/HH-MM` (e.g. `2025-06-03/12-34.json`).
* For N = 1 (hour), `YYYY-MM-DD/HH.json` (e.g. `2025-06-03/12.json`).
* For N = 2 (day), `YYYY-MM-DD.json` (e.g. `2025-06-03.json`).
* For N = 3 (month), `YYYY-MM.json` (e.g. `2025-06.json`).
* …and so on, matching each level’s epoch granularity.

---

### 8. Empty‐Page Discard Logic

Whenever a page’s **only** trigger is “age” (i.e. no size threshold was met) and its `content` is empty (no child hashes or net patches), perform the following:

1. Remove the page from `active_pages`.
2. **Do not** write any JSON to disk.
3. Do not call `perform_rollup` for that page level, since there is no payload to propagate.
4. Await the next incoming child leaf or rollup event that might create a new page in that level.

---

### 9. Retention & Pruning (Optional)

A separate retention mechanism (not part of core rollup) can periodically delete old page files to enforce organizational policies:

* **Level 0 (raw deltas)**: Keep only the last *X* days/weeks of minute pages (or boundary retention: keep only L1 summer  files older than 30 days).
* **Level 1 (hour)**: Keep up to *Y* months of hourly pages; delete older hours because daily nets suffice.
* **Higher levels**: Usually never prune L3 (monthly) if you want long‐term auditability—unless you provide archived backups.

Retention runs as a background job (e.g. a weekly cron) that scans `./journal/level_N/` directories and removes files whose `window_end < now – retention_period[N]`.

---

### 10. Example Rollup Timeline

1. **Leaf A arrives at 2025-06-03 12:00:15**

   * No L0 page active (because none yet), so `get_or_create_active_page(0, 12:00:15)` creates `L0P0` with `creation_timestamp = 12:00:00`, `window_end = 12:01:00`.
   * Append ΔA to L0P0 (size=1, age=15s < 60s, so do nothing).

2. **Leaf B arrives at 2025-06-03 12:00:45**

   * L0P0 is active. Append ΔB (size=2, age=45s < 60s). No rollup yet.

3. **Leaf C arrives at 2025-06-03 12:00:59**

   * Append ΔC (size=3, age=59s < 60s). No rollup.

4. **Leaf D arrives at 2025-06-03 12:01:10**

   * L0P0 exists but `trigger_ts – creation = (12:01:10 – 12:00:00) = 70s ≥ 60s`. Since L0P0.size=3>0, finalize L0P0:
     • Compute Merkle root over \[ΔA, ΔB, ΔC], record `first_child_ts=12:00:15`, `last_child_ts=12:00:59`.
     • Persist `L0P0` to `journal/level_0/2025-06-03/12-00.json`.
     • Remove L0P0 from `active_pages`.
     • Call `perform_rollup(0, trigger_ts=12:01:10)`.

   * Inside `perform_rollup(0, 12:01:10)`:
     a) Load persisted L0P0. Get its `merkle_root = H0` and child timestamps.
     b) `get_or_create_active_page(1, 12:01:10)` creates `L1P0` with `creation_timestamp=12:00:00`, `window_end=13:00:00`.
     c) Merge payload: L1P0.child\_hashes.push(H0). `first_child_ts=12:00:15`, `last_child_ts=12:00:59`.
     d) Recompute L1P0.merkle\_root. Check `(12:01:10–12:00:00)=70s<3600s` & `size=1<max_leaves` → no finalize. Keep L1P0 active.

   * Now handle ΔD:
     • Create new L0P1 with `creation_timestamp=12:01:00`, `window_end=12:02:00`.
     • Append ΔD there (size=1, age=10s < 60s).

5. **Leaf E arrives at 2025-06-03 13:00:05**

   * ΔE’s timestamp falls into hour window `13:00:00–14:00:00`, but first handle L0P1:
     – Check `(13:00:05–12:01:00)=3595s ≥ 60s`. Since L0P1.size=1>0, finalize L0P1:
     • Persist L0P1 as `journal/level_0/2025-06-03/12-01.json`, Merkle root H1.
     • Remove L0P1.
     • `perform_rollup(0, 13:00:05)` → get\_or\_create L1P0 (`window_start=12:00` still active), then merge H1 into L1P0. L1P0.content now has \[H0, H1]. Compute age=(13:00:05–12:00:00)=3605s; since max\_age=3600s, L1P0.age≥3600s, finalize L1P0:
     • Persist L1P0 (hour 12:00–13:00).
     • Remove L1P0.
     • `perform_rollup(1, 13:00:05)` → create L2P0 (`window_start=2025-06-03T00:00:00Z`), merge L1P0’s merkle root, but age=(13:00:05–00:00:00)=46805s<86400s → do not finalize.
   * Now create new L0P2 for ΔE (window\_start=13:00:00). Append ΔE.

---

### 11. Edge Cases & Additional Rules

1. **Out‐of‐Order Leaves**

* If a leaf arrives with `timestamp < active_level0.creation_timestamp`, treat it as “belonging to a previous page” (e.g. late‐arriving log). Options:
  a) Reject it (error).
  b) Insert it into an already‐finalized L0 page by reloading that page’s file, updating it, and re‐persisting (with risk of invalidating existing Merkle proofs).
* Simpler approach: require that leaves are ingested in (approximately) non-decreasing timestamp order; late deltas are appended into the current or next epoch depending on a “maximum lateness” policy.

2. **Simultaneous Finalization**

* If a single leaf triggers multiple levels to finalize (e.g. a long gap so that L0, L1, and L2 all exceed age), `perform_rollup` will cascade: finalize L0 → merge into L1 → finalize L1 → merge into L2 → finalize L2.
* Ensure recursion is depth‐bounded by the number of configured levels.

3. **Error Handling**

* Disk I/O errors when persisting a page should cause `add_leaf` to return an error; the calling application can retry or log the failure.
* If `perform_rollup` fails in the middle of cascading, ensure no “half‐written” pages remain—consider writing to a temporary file then renaming atomically to preserve consistency.

4. **Concurrent Writers**

* If multiple threads/processes may call `add_leaf` concurrently, guard `active_pages` with a mutex (or use a single‐threaded actor model).
* Page finalization must be atomic: hold the lock while finishing content, writing the file, removing from `active_pages`, then release before calling recursion.

5. **Retention & Cleanup**

* A separate job (not part of `add_leaf`/`perform_rollup`) can periodically delete old page files:

  ```rust
  for level in 0..max_levels {
    let retention = config.retention_days[level];
    for file in list_files(“journal/level_{level}/”) {
        let file_end = read_window_end_from_filename(file);
        if file_end + Duration::days(retention) < Utc::now() {
            delete(file);
        }
    }
  }
  ```
* Deleting old higher‐level files is safe as long as no user needs to reconstruct history before that retention window.

---

### 12. Summary of Key Behaviors

1. **Single Active Page per Level:**

* At any moment, there is at most one unfinalized page at each level in `active_pages`.

2. **Rollup Conditions (per Level N):**

* **Size‐based:** `content.len() ≥ max_leaves_per_page[N]` → finalize (if nonempty).
* **Age‐based:** `(trigger_ts – page.creation_timestamp) ≥ max_page_age_seconds[N]` → finalize (if nonempty).

3. **Empty Page Discard:**

* If age triggers but `content.is_empty()`, simply remove page from `active_pages` with no persistence or upstream propagation.

4. **Cascading Finalization:**

* Whenever a Level N page finalizes with nonempty content, call `perform_rollup(N, trigger_ts)`, which merges payload into Level N+1 and may similarly finalize that parent if conditions are met.

5. **Payload Propagation:**

* If using “child hashes,” propagate `child_merkle_root` to parent.
* If using “net patches,” propagate `child_net_patches` (a `HashMap<ObjectID, FieldPatch>`) to parent; the parent merges by overwriting older values.

6. **Recording Metadata:**

* For each finalized page, record in the JSON file:
  • Level and page\_id
  • Window start/end (aligned to epoch)
  • Merkle root (over content)
  • First/last child timestamps (for replay anchors)
  • Contents: either `child_hashes: [...]` or `net_patches: { ... }`

7. **Reconstruction & Queries (not part of “rollup” but enabled by it):**

* To reconstruct state at time T:

  1. Load the highest level page whose `window_start ≤ T`.
  2. If that page is net‐patch‐based, merge its map (or invert if going backward) up to T.
  3. If more granularity is needed, descend to lower levels (Day → Hour → Minute), merging/inverting patches or replaying leaves until T.

---

#### End of Specification

This document fully describes the configuration parameters, data structures, and algorithms for the multi‐level, time‐based rollup mechanism. It ensures that raw leaves are properly batched, aggregated, finalized, and persisted at each epoch level, that empty pages are discarded, and that parent pages accumulate payloads from children until their own rollup conditions are met. This hierarchical design guarantees a compact, verifiable, and reconstructible audit log spanning minutes through months (and beyond).
