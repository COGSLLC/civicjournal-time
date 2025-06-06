Test Plan for CivicJournal-Time
The test plan is organized by test type (unit, integration, E2E, API) and by module/file. Each item references functions or behaviors to cover. Citations point to the relevant code sections.
1. Unit Tests
core::leaf.rs (JournalLeaf & LeafData)
✅ Test JournalLeaf::new(...) with various inputs: no previous hash and with a given prev_hash, verifying that leaf_id increments (global counter reset via test_utils::reset_global_ids) and that leaf_hash changes if any input changes. For example, creating two leaves with identical payloads but different prev_hash should yield different hashes.

<!-- Test that JournalLeaf::new returns an error when given an invalid payload (e.g. a serde_json::Value that fails serialization). -->
✅ Test the LeafData enum and its V1 variant: serialization/deserialization round-trips and equality.
core::page.rs (JournalPage)
✅ Test creating a new JournalPage at various levels (e.g. L0 and L1) and adding content: adding leaves to L0 pages and thrall hashes to higher-level pages

✅ Verify that after add_leaf() or add_thrall_hash(), the page’s merkle_root, page_hash, and prev_page_hash fields are updated correctly (e.g. by calling recalculate_merkle_root_and_page_hash())

✅ Test serialization/deserialization: serialize a page to JSON or bytes and deserialize back, checking all fields match.
✅ Test behavior when creating an empty page (no leaves) and finalizing it: ensure list_finalized_pages_summary includes it (storage tests below cover this).
core::merkle.rs (MerkleTree)
✅ Test MerkleTree::new with a known list of hashes, verifying the computed root matches a precomputed value.
✅ Test get_proof(idx) returns a correct proof array for each index, and that feeding the proof into a verifier recovers the root.
✅ Test error cases, e.g. building a tree with zero leaves (should error) or requesting get_proof with an out-of-range index.
core::hash.rs
✅ If there are helper functions like sha256_hash_concat, test that they produce expected SHA256 digests for simple concatenations (compare against a known hash).
core::time_manager.rs (TimeHierarchyManager)
✅ Test adding leaves via the manager triggers correct page assignment: e.g. append multiple leaves (timestamps within the same window) and verify they go into the same L0 page, then when enough leaves or time passes, a new L0 page is created.
✅ Test roll-up logic: e.g. configure a small max_items_per_page, append leaves to overflow, and verify that the excess leaf is rolled up to L1 as a thrall hash.
✅ Test multi-leaf parent pages: configure parent levels with max_leaves_per_page > 1 and verify they accumulate multiple thrall hashes before finalizing.
✅ Test age-based rollups: configure max_page_age_seconds to trigger rollups based on time rather than item count, including cascading rollups through multiple levels.
✅ Test retention policies: configure a short retention period, create a few pages, simulate time passing (by adjusting timestamps), and verify old pages are deleted (using the storage backend’s list_finalized_pages_summary).
config::mod.rs (Config and related)
✅ Test Config::default() produces expected defaults (e.g. 4 levels, force_rollup_on_shutdown == true)

✅ Test Config::load(path) when the file exists with valid TOML: that fields are parsed correctly. Also test fallback when file is missing: e.g. point to a nonexistent file and verify it returns Ok(default_config)

✅ Test apply_env_vars(): set an environment variable like CJ_LOGGING_LEVEL=debug, call apply_env_vars(), and verify config.logging.level is updated accordingly. Also test that invalid env values produce a ConfigError.
✅ Test validate(): create a Config with invalid values (e.g. negative durations or contradictory settings) and verify validate() returns an error (based on validation::validate_config).
<!--query::engine.rs (QueryEngine)
get_leaf_inclusion_proof(leaf_hash):
Set up a storage backend (e.g. in-memory) with known pages and leaves. Invoke get_leaf_inclusion_proof for an existing leaf hash; verify the returned LeafInclusionProof has the correct leaf, page_id, level, and a valid Merkle proof (you can recompute the Merkle root separately to check). Cite logic: it searches L0 pages and constructs a MerkleTree
 
 
.
✅ Test the “leaf not found” path: call with a hash not in any page and verify it returns an Err(QueryError::LeafNotFound)
 
.
✅ Test the case where a found leaf has no matching stored JournalLeaf: e.g. if a leaf hash is in a page but load_leaf_by_hash returns None, the code returns Err(QueryError::LeafNotFound).
reconstruct_container_state(container_id, at_timestamp):
Build a sequence of pages with leaves having a specific container_id and timestamps. Call reconstruct_container_state at a timestamp after some leaves; verify it returns a ReconstructedState whose state_data equals the cumulative delta (merged via apply_delta) of all matching leaves up to that time
 
.
Test “container not found” path: if no leaf with that container_id exists up to the given time, the function should return Err(QueryError::ContainerNotFound(container_id))
 
.
get_delta_report(container_id, from, to):
Create leaves within a page spanning a time range. Call get_delta_report with a range covering some of them, and verify the returned DeltaReport.deltas contains exactly those leaves (sorted by timestamp)
 
.
Test the InvalidParameters error: call with from > to and verify it returns Err(QueryError::InvalidParameters)
 
.
Test “container not found” if no matching leaves in range: expect Err(QueryError::ContainerNotFound).
get_page_chain_integrity(level, from, to):
Create a series of pages at a level with known prev_page_hash chain, and modify some (simulate corruption) so that recalculate_merkle_root_and_page_hash() yields a different hash. Call get_page_chain_integrity and verify it returns a list of PageIntegrityReport entries: pages with no issues should have is_valid=true, and any with mismatched merkle_root or prev_page_hash should list the appropriate issue message. Logic: it recalculates each page’s hashes and compares to originals
 
.
Test the InvalidParameters case: e.g. from=5, to=3 should return an Err(QueryError::InvalidParameters)
--> 
.
<!-- Test pages missing from storage: include a summary with a page_id that has no stored file, and verify the report for that page has is_valid=false with issue “page missing” -->
 
.
<!--api::sync_api.rs
Test Journal::new(config): uses create_storage_backend and TimeHierarchyManager::new. The existing test verifies it returns Ok
 
. Also test that if create_storage_backend fails (e.g. invalid file path for FileStorage), it returns an Err(CJError).
get_page(level, page_id):
Test retrieving a non-existent page returns Err(CJError::PageNotFound)
 
. This is already done in tests. Also test retrieving an existing page returns Ok(page).
(Optional) If any other methods exist (e.g. leaf inclusion, reports in sync_api), test them similarly by calling the underlying async query methods via the tokio runtime.
api::async_api.rs
Journal::new(config).await: test success (already covered). Also simulate failure: for example, pass a config with unsupported storage type or invalid base_path to cause create_storage_backend to error.
append_leaf(timestamp, parent_hash, container_id, data):
Test appending a single leaf returns a PageContentHash::LeafHash with a 32-byte hash
 
.
Test multiple appends: ensure each returned hash is unique
 
.
Error path: simulate a storage write failure. For example, use MemoryStorage with set_fail_on_store as in tests
 
, then call append_leaf. It should return Err(CJError::StorageError) with the simulated error message
 
.
Rollup trigger: configure a tiny max_items_per_page and append enough leaves to force a roll-up, verifying the call still succeeds (the tests illustrate this)
 
.
get_page(level, page_id): test non-existent page returns Err(CJError::PageNotFound)
 
 (already covered). Test success for an existing page.
Async query methods (get_leaf_inclusion_proof, reconstruct_container_state, get_delta_report, get_page_chain_integrity): these simply await the QueryEngine methods. Write async tests that set up known data (via previous append_leaf calls) and verify these methods return correct results or errors, paralleling the QueryEngine unit tests above.-->
storage::memory.rs (MemoryStorage)
✅ Test new(): it should start empty (is_empty()==true)

✅ store_page/load_page: store a dummy JournalPage (use JournalPage::new) and verify load_page returns the identical page

✅ Also test that storing multiple pages works and they can be retrieved.
✅ page_exists: verify page_exists is false before storing and true after storing a page

✅ clear: store pages, call clear(), and ensure storage is empty and page_exists is false for all pages
 
.
<!-- Fail-on-store: use set_fail_on_store(level, page_id) to simulate errors (the tests show this)

E.g., configure a fail on level 0 (any page) and ensure store_page returns an Err(CJError::StorageError). Verify the error message matches the format (contains “Simulated MemoryStorage write failure”)

Then clear the failure condition and ensure store succeeds. -->
<!-- list_finalized_pages_summary: after storing some pages, verify that list_finalized_pages_summary(level) returns summaries with correct page_id and level for all pages at that level.
backup_journal and restore_journal: For MemoryStorage, backup_journal is a no-op (logs a warning)

Test that calling it returns Ok(()) and does not alter storage. restore_journal should return a “not supported” error

; test that it returns Err(CJError::NotImplemented) or similar. -->
✅ load_page_by_hash: test retrieving by page hash: after storing pages, take one page’s page_hash and call load_page_by_hash; it should return the full page
 
.
✅ load_leaf_by_hash: covered by tests. In an L0 page with multiple leaves, each leaf’s hash should be found. Also verify that leaves in L1 pages are not found (only L0 is searched) and that non-existent hashes return Ok(None)
 
.
storage::file.rs (FileStorage)
✅ FileStorage::new(path, compression): Test creating a new FileStorage in a temp directory: it should create the base path and a marker file (.civicjournal-time). Verify the marker exists.
✅ store_page and load_page: Store a page without compression (CompressionConfig { enabled: false }) and verify that reading it returns an identical JournalPage. The file format should begin with the magic string CJTP

✅ Store a page with each compression algorithm (Zstd, Lz4, Snappy) enabled. For each, verify load_page returns the same page (the code handles decompression)

✅ Corrupt header tests: manually create a file with wrong magic or version (or write junk to the first bytes of a valid .cjt file) and verify load_page returns Err(CJError::InvalidFileFormat)


<!--
page_exists and delete_page: verify page_exists(level,page_id) matches filesystem state. Test deleting a page file removes it (and page_exists returns false afterward)

Deleting a non-existent page should still return Ok(()).
✅ list_finalized_pages_summary: after storing multiple pages across levels, ensure summaries include all existing pages. Also test that if the level directory is missing or empty, it returns an empty list

✅ load_page_by_hash: store several pages at various levels; take one page_hash and call load_page_by_hash. Verify it finds and returns the correct page

✅ Test that it skips files with wrong magic or extension (code uses MAGIC_STRING and known extensions) and returns Ok(None) if not found

load_leaf_by_hash: similar to MemoryStorage: for L0 pages with leaves, verify each leaf’s hash is found
-->
 
<!--. If no L0 dir exists, it should return Ok(None)-->

<!-- Verify skipping of non-page_ files (code checks file name prefix) -->
 
.
<!--backup_journal(backup_path):
Empty journal: if the journal subdirectory is absent, calling backup_journal should create an empty zip containing only a manifest with no files
 
. Test that an empty zip is created and that the manifest inside has files: [].
Non-empty journal: store a few pages, call backup_journal, and then open the resulting zip. Verify that it contains a manifest and compressed page files. Check the manifest’s files entries (paths and hashes) are correct (this verifies both backup and manifest creation logic).
restore_journal(backup_path, target_dir):
Test error if backup_path does not exist: it should return Err(CJError::StorageError)
 
.
For a valid backup zip created above, call restore_journal to a new directory. Then verify that the restored target_dir/journal/level_X/page_Y.cjt files exist and match the originals. Verify metadata (file permissions, etc) are preserved as coded.-->
<!--turnstile::mod.rs (Turnstile Manager)
append(&mut self, payload_json, timestamp): test that appending valid JSON produces a ticket (hex hash) and adds a pending entry. The existing test computes a specific hash for {"foo":"bar"}
 
. Also test that pending_count() increments and list_pending() returns the ticket.
confirm_ticket(leaf_hash, status, error_msg):
Confirming true should set the entry’s status to Committed, update prev_leaf_hash to leaf_hash, move the hash into committed, and remove it from pending
 
. Test these side-effects (e.g. ts.latest_leaf_hash() equals the ticket, ts.pending_count()==0).
Confirming false (with an error message) should set last_error and increment retries: if retries exceed max_retries, status becomes FailedPermanent. It should also call log_orphan_leaf if log_orphans=true, creating an OrphanEvent. Verify that an orphan is recorded in orphan_events() with matching fields.
Test error conditions: confirming a non-existent ticket should return Err(CJError::NotFound).
retry_next_pending(callback):
Case 1: callback returns success (1): pending entry becomes committed (similar to confirm), and retry_next_pending returns Ok(0)
 
. Verify pending count drops and prev_leaf_hash updates.
Case 2: hash mismatch (tampered payload): the code checks computed != leaf_hash and returns Ok(-2), marking status FailedPermanent
 
. Test by manually altering an entry’s payload_json as in existing test
 
.
Case 3: callback returns failure (!=1): increment retry_count. If retry_count < max_retries, return Ok(1) and ensure status remains Pending and last_error updated. If retry_count >= max_retries, set status to FailedPermanent and return Ok(2). In both cases, log_orphan_leaf should be called (recording an orphan). Verify orphan_events() grows and contains the expected orig_hash, error_msg, etc.
Verify that if no pending entries exist, retry_next_pending returns Ok(-1).
Other methods:
leaf_exists(leaf_hash): test that it returns true if the hash is in pending or committed, otherwise false.
list_pending(max): test it returns up to max hashes of status Pending.-->
2. Integration Tests
✅ TimeHierarchy + Storage Integration: use a shared test config with MemoryStorage or a temp FileStorage to simulate actual journal usage:
✅ Append a series of leaves (via async API or directly via TimeHierarchyManager) and then use the query engine (Journal::get_delta_report, etc.) to fetch reports. Verify consistency of data across components.
✅ Test roll-up across levels: e.g. append enough leaves to fill and finalize L0 pages, then check that L1 pages are created with correct thrall hashes (using get_page_chain_integrity).
<!-- Configuration + Init Integration: call civicjournal_time::init(config_path) with a path to a custom TOML file (or none) and verify the global config is initialized and accessible via config(). Test that environment overrides are applied at init (e.g. set CJ_LOGGING_LEVEL before init). -->
<!-- End-to-End Workflow (API): simulate an application scenario:
Initialize the system with a test config (e.g. in-memory storage).
Append several deltas for multiple containers over time via the async/sync API.
Query the container states and delta reports; verify they match expected outcomes (this exercises append + query integration).
Perform a backup to a file, then restore into a new directory; verify the restored data yields identical query results. -->
3. Edge Cases & Error Handling
<!-- Invalid Inputs:
Passing a non-JSON or malformed JSON string to Turnstile::append or compute_hash should cause an error (the code uses serde_json::from_str

). Test that it returns Err. -->
<!-- Calling QueryEngine methods with nonsensical parameters (empty container IDs, levels out of range, etc.) and verify proper InvalidParameters or ContainerNotFound errors. -->
<!-- File I/O errors: e.g. simulate write permission denied (set storage path to a read-only directory) and verify operations return CJError::StorageError. -->
<!-- In load_page, if the file is too short (len<6) or has wrong magic/version, it returns InvalidFileFormat

. Test these by writing custom invalid files. -->
Boundary Conditions:
<!-- Test pages with zero leaves (empty pages) and maximum allowed leaves (if any). -->
<!-- Time edges: leaves with timestamps exactly on roll-up boundaries. -->
4. API (Endpoint) Tests
(No HTTP endpoints are defined in this library.) The “API” here refers to the Rust synchronous/asynchronous interfaces described above. Their key behaviors are covered in unit/integration tests. If a future version exposes REST or CLI commands, those would require corresponding tests.
5. Third-Party Dependencies
Compression Libraries:
Verify that each supported compression (Zstd, Lz4, Snappy, None) works end-to-end in FileStorage. The unit tests above ensure encoding/decoding doesn’t corrupt data
 
 
✅ Edge-case: test with very small (trivial) pages and very large pages to ensure no panics in compression libraries.
dashmap in MemoryStorage: assume correct (use existing tests). No special tests needed beyond those verifying MemoryStorage functionality.
tokio: mostly used under the hood for async I/O; unit tests for async methods already cover it.
chrono, serde, serde_json: covered indirectly via creating timestamps and JSON values. No need to test them specifically.
Encryption/Hash (sha2): correctness of SHA256 isn’t tested here (we assume the crate is correct). We do test that our code calls it consistently (via known hash outputs in tests, e.g. turnstile).
Sources: The plan is based on the CivicJournal-Time codebase; key functions and behaviors are cited from the source. For example, JournalLeaf::new is defined in core::leaf.rs
 
, and the Turnstile append/confirm logic (with test example) is in turnstile/mod.rs
 
. The MemoryStorage and FileStorage implementations (with simulated failures and file format logic) are cited from storage/memory.rs
 
 and storage/file.rs
 
 
. These citations pinpoint the code that the tests will cover.