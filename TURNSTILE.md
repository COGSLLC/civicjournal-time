Turnstile System Documentation (CivicJournal-Time)

Table of Contents

Introduction

Concept Overview

What Is the Turnstile System?

Key Benefits

Core Components

CivicJournal-Time (CJT)

Ticket Manager (TM)

Database (DB) Integration

Turnstile Workflow

Initialization

Appending a Payload (Turnstile Entry)

Database Insert Attempt

Confirming or Rejecting a Ticket

Parking Lot and Retry Logic

API/FFI Reference

Initialization & Shutdown

Turnstile Append

Confirming Tickets

Retry Pending Payloads

Lookup & Monitoring

Database Trigger Configuration

Canonical JSON Reconstruction

Sample PostgreSQL Trigger

Integration Examples

C/C++ Example

Ruby on Rails Example

General Pseudocode

Configuration & Tuning

Storage Paths

Page Sizes & Rollup Thresholds

Parking Lot Settings

Best Practices

Security Considerations

Payload Size & Rate Limiting

JSON Schema & Validation

Handling Orphaned Entries

Troubleshooting & Common Pitfalls

Further Reading & References

Introduction

This document provides a comprehensive guide to the Turnstile system built on CivicJournal-Time (CJT). The Turnstile pattern ensures that every payload intended for a database is first immutably recorded in a tamper-evident ledger. If the database rejects the payload, it is parked for later review and retry, preserving data integrity and providing a transparent audit trail.

Use this as your primary reference for:

Understanding the Turnstile architecture

Implementing CJT and Ticket Manager (TM) in your applications

Configuring database triggers for canonical JSON verification

Handling error scenarios and orphaned payloads

Security, performance, and best practices

Concept Overview

What Is the Turnstile System?

The Turnstile system enforces a strict “ledger-first, database-second” workflow:

Ledger Append: Each payload is sent to CivicJournal-Time, which computes a cryptographic hash (the "ticket"), persists the leaf in a Merkle hierarchy, and holds it as “pending.”

Database Insert Attempt: The payload, along with its ticket (leaf hash) and the previous chain anchor, is inserted into the database. A trigger verifies the ticket against the reconstructed JSON.

Confirm or Reject: If the DB accepts, the ticket is confirmed, and the chain advances. If the DB rejects, the ticket remains pending, CJT logs an orphan event, and the payload goes to a parking lot.

Retry Mechanism: Parking-lot entries are retried—recomputed and re-submitted—until successful or flagged as permanent failures.

This enforces immutable, verifiable logging of all write operations, ensuring transparency, auditability, and tamper evidence.

Key Benefits

Tamper-Evidence: Any attempt to retroactively alter a logged payload breaks the Merkle chain.

Auditability: External auditors and users can request and verify inclusion proofs without full DB access.

Data Integrity: Database never accepts any row without a matching, pre-recorded ticket.

Error Handling: Orphaned payloads get parked and retried automatically, so no data is lost.

Scalability: Hierarchical rollups keep memory usage low and allow compact long-term archival.

Core Components

CivicJournal-Time (CJT)

CJT is a Rust-based ledger that maintains an append-only log of JSON payloads. It:

Stores leaves in a Merkle tree with hierarchical pages (L0, L1, ...)

Computes SHA-256–based hashes chaining each payload to its predecessor

Exposes a C FFI (and optionally network API) for turnstile operations

Manages pending tickets, orphan logging, and parking-lot state

Key responsibilities:

Hash Computation: new_hash = SHA256(prev_hash || payload_JSON)

Persistent Storage: Stores raw pages on disk (or cloud) and caches minimal state in memory

Pending Management: Temporarily holds tickets until DB confirmation

Orphan Events: Logs failed tickets as separate Merkle leaves with error context

Ticket Manager (TM)

TM is the orchestration layer embedded in CJT (or as a companion module) that:

Tracks Pending Tickets: Holds on-disk queue of tickets not yet committed to the DB

Implements Parking Lot: Moves failed payloads into a “pending_retries” store

Handles Retries: Periodically re-attempts DB inserts via a callback

Flags Permanent Failures: After configurable retry attempts, marks a payload as “failed_permanently” and raises an alert

Advances the Chain: Only updates prev_leaf_hash when a ticket is confirmed by the DB

Database (DB) Integration

The DB (commonly PostgreSQL) enforces turnstile semantics via:

Schema Additions: Each audited table adds prev_cj_leaf_hash CHAR(64) NOT NULL and cj_leaf_hash CHAR(64) NOT NULL columns

Triggers: A BEFORE INSERT/UPDATE trigger reconstructs the canonical JSON from row fields, computes its SHA-256 chained hash, and compares it to NEW.cj_leaf_hash. Mismatches cause a rejection (exception) and leave the ticket pending.

Unique Constraints / Indexes: Optionally, a unique index on cj_leaf_hash prevents replay attacks.

Together, CJT + TM + DB trigger create an end-to-end system guaranteeing that no unverified or tampered data ever enters the database.

Turnstile Workflow

This section outlines the step-by-step lifecycle of a payload passing through the Turnstile system.

1. Initialization

CJT Initialization

Call cjt_init(config_json) to create a CJTClient instance.

config_json includes:

storage_path: filesystem or blob storage root for pages and metadata

max_leaves_per_page, max_children_per_page: rollup thresholds

max_retries, retry_interval_ms: parking-lot settings

On startup, CJT loads:

Latest prev_leaf_hash from its state file

Any pending tickets from its on-disk pending store

DB Initialization

Modify audited tables:

ALTER TABLE my_table
  ADD COLUMN prev_cj_leaf_hash CHAR(64) NOT NULL,
  ADD COLUMN cj_leaf_hash      CHAR(64) NOT NULL;

Create a BEFORE INSERT (and/or BEFORE UPDATE) trigger that:

Rebuilds canonical JSON from NEW row fields

Computes computed_hash = SHA256(raw(prev_cj_leaf_hash) || UTF8(rebuilt_JSON))

Raises exception if computed_hash <> NEW.cj_leaf_hash

Ensure a unique index: CREATE UNIQUE INDEX ON my_table(cj_leaf_hash); to prevent replays.

Application-State

Application may store its own copy of prev_leaf_hash in memory or DB for convenience.

If missing, fetch from CJT via cjt_get_latest_leaf_hash(client, buffer).

2. Appending a Payload (Turnstile Entry)

Build Payload JSON

The application constructs a JSON object containing:

prev_leaf_hash: the string hex of the last confirmed ticket (Hₙ₋₁)

data: a nested object representing the actual fields to store (e.g. user action, row values)

Any metadata: action, timestamp, user_id, etc.

Example:

{
  "prev_leaf_hash":"a1b2c3...",
  "data": {
    "action":"cast_vote",
    "vote_id":42,
    "user_id":123,
    "proposal_id":456,
    "choice":"yes",
    "timestamp":"2025-06-15T14:32:10Z"
  }
}

Canonicalization: Keys must be ordered consistently. Use a library or builder that guarantees lexicographic ordering (e.g. jsonb_build_object in Postgres, or serde_json::to_string(&BTreeMap) in Rust).

CJT Append Call

Invoke:

char ticket_hash[65];
int rc = cjt_turnstile_append(
  client_ptr,
  payload_json,
  (uint64_t)time(NULL),
  ticket_hash  // out parameter for 64-char hex + NUL
);
if (rc != 0) { /* handle CJT error: disk full, invalid JSON, etc. */ }

Internally, CJT:

Reads current_prev_hash from memory (Hₙ₋₁).

Concatenates raw bytes of Hₙ₋₁ (32 bytes) with UTF-8 bytes of payload_json.

Computes digest = SHA256(concatenation) → new leaf hash Hₙ (32-byte binary).

Converts digest to a 64-character hex string → ticket_hash.

Persists a “pending” entry: { prev_hash: Hₙ₋₁, payload_json, leaf_hash: Hₙ, timestamp, status: pending }.

Returns ticket_hash to the application.

Note: CJT’s in-memory prev_leaf_hash is not updated yet—remains Hₙ₋₁ until confirmation.

3. Database Insert Attempt

Application Performs DB Insert

Use your standard DB client/framework (e.g. ActiveRecord for Rails, db_execute in C, or Python psycopg2) to run:

INSERT INTO my_table
  (col1, col2, ..., prev_cj_leaf_hash, cj_leaf_hash)
VALUES
  ($1, $2, ..., 'Hₙ₋₁', 'Hₙ');

Replace $1, $2, ... with your actual data fields.

DB Trigger Verification

The BEFORE INSERT trigger fires:

Reconstructs rebuilt_json exactly as the application did:

rebuilt_json := jsonb_build_object(
   'prev_leaf_hash',   NEW.prev_cj_leaf_hash,
   'data',             jsonb_build_object(
                          'action',      NEW.action,
                          'vote_id',     NEW.vote_id,
                          'user_id',     NEW.user_id,
                          'proposal_id', NEW.proposal_id,
                          'choice',      NEW.choice,
                          'timestamp',   NEW.timestamp
                       )
)::TEXT;

Computes computed_hash := encode(digest(decode(NEW.prev_cj_leaf_hash,'hex') || rebuilt_json::BYTEA, 'sha256'), 'hex');

If computed_hash <> NEW.cj_leaf_hash, raises EXCEPTION 'Ticket mismatch: expected % but got %', computed_hash, NEW.cj_leaf_hash;

If no exception, the INSERT proceeds.

Result

Success: The row is inserted. Trigger or unique index enforces chain integrity. Application sees successful return code.

Failure: Trigger/constraint error bubbles up to the application. At this point, CJT has a pending ticket Hₙ, but the DB did not accept it.

4. Confirming or Rejecting a Ticket

After the DB INSERT attempt returns, the application must inform CJT of the outcome:

If DB insert succeeded:

int rc = cjt_confirm_ticket(client_ptr, ticket_hash, 1, NULL);
if (rc != 0) { /* handle CJT error: should not happen in normal flow */ }

CJT marks the pending entry for leaf_hash = Hₙ as committed.

Updates its internal prev_leaf_hash to Hₙ. Future turnstile appends chain from Hₙ.

If DB insert failed:

const char *error_msg = "DB rejected row: unique constraint violation";
int rc = cjt_confirm_ticket(client_ptr, ticket_hash, 0, error_msg);
if (rc != 0) { /* handle CJT internal error */ }

CJT leaves the pending entry status as pending (chain anchor remains Hₙ₋₁).

Creates an “orphan event” leaf:

{
  "type":      "orphan",
  "orig_hash": "<Hₙ>",
  "error_msg": "DB rejected row: unique constraint violation",
  "timestamp":"2025-06-30T16:45:22Z"
}

Chained from Hₙ₋₁ → new orphan leaf hash.

Pushes (Hₙ, payload_JSON) into CJT’s internal parking lot with retry_count = 0.

Outcome

Application gets back success code from cjt_confirm_ticket. It can then log locally or raise alerts (“Payload parked for review”).

CJT now holds:

prev_leaf_hash = Hₙ₋₁ (unchanged)

A pending entry for Hₙ in the parking lot.

A new orphan leaf in the chain (optional but recommended for audit clarity).

5. Parking Lot and Retry Logic

Pending Records Store

CJT maintains an on-disk table (e.g. RocksDB or SQLite) of pending entries:

pending_ticket {
  leaf_hash_hex:   CHAR(64),  // Hₙ
  prev_hash_hex:   CHAR(64),  // Hₙ₋₁
  payload_JSON:    TEXT,
  timestamp:       UINT64,
  retry_count:     INT,
  last_error:      TEXT,
  status:          ENUM('pending','committed','failed_permanent')
}

A background job or thread periodically invokes cjt_retry_next_pending to process these.

Retry Next Pending

int rc = cjt_retry_next_pending(client_ptr, my_db_insert_callback);
switch (rc) {
  case 0:
    // That pending entry just committed. Chain advanced.
    break;
  case 1:
    // Still pending (DB rejected again). retry_count incremented.
    break;
  case 2:
    // Permanent failure. status set to 'failed_permanent'.
    break;
  default:
    // No pending entries or internal error.
    break;
}

Internally:

CJT selects the oldest entry with status = 'pending'.

Recomputes computed_hash = SHA256(raw(prev_hash) || payload_JSON) → hex string.

If computed_hash != leaf_hash_hex (indicates payload was modified or logic mismatch),
CJT marks the entry failed_permanent and returns -2.

Calls the provided db_insert_callback(prev_hash, payload_JSON, leaf_hash_hex).

The callback attempts the same DB insert:
INSERT INTO my_table(..., prev_cj_leaf_hash=prev_hash, cj_leaf_hash=leaf_hash_hex).

Returns 1 on success, 0 on rejection.

If callback returned 1:

CJT marks entry committed, updates prev_leaf_hash = leaf_hash_hex, and returns 0.

If callback returned 0:

CJT increments retry_count, logs a new orphan event if desired, and returns 1.

If retry_count exceeds max_retries configured, CJT marks entry failed_permanent, returns 2.

Manual Review of Parking Lot

Admins can list pending hashes via cjt_list_pending, examine payload_JSON and last_error, correct any issues, and allow retries.

Optionally, implement a small CLI or web UI for admins to “retry,” “skip,” or “delete” pending entries.

API/FFI Reference

This section enumerates the C FFI functions CJT exposes for the Turnstile system. All function definitions assume the user has included the appropriate header (e.g. cjt_ffi.h) and linked against libcjt.so (or libcjt.dylib / cjt.dll).

Initialization & Shutdown

// Initialize the CJT client with a JSON config. Returns an opaque pointer (client handle).
void *cjt_init(const char *config_json);

// Gracefully shut down CJT, flush state, persist pending store, and free resources.
void cjt_destroy(void *client_ptr);

config_json fields:

storage_path (string): Directory or bucket root for CJT pages, state, and pending store.

max_leaves_per_page (int, optional): L0 page capacity (default: 1024).

max_children_per_page (int, optional): Capacity for higher-level pages (default: 1024).

max_retries (int, optional): Number of parking-lot retry attempts before permanent failure (default: 5).

retry_interval_ms (int, optional): Milliseconds between automatic replays (default: 5000).

Other fields: Logging level, optional TLS certificates if CJT is run as a service, etc.

Turnstile Append

// Append a payload JSON as a new pending ticket. Returns 0 on success.
// - client_ptr: CJT client handle.
// - payload_json: NUL-terminated UTF-8 JSON string.
// - timestamp_unix_secs: Seconds since epoch (UTC) for this payload.
// - out_ticket: char[65] buffer for the 64-char hex hash + NUL.

int cjt_turnstile_append(
    void *client_ptr,
    const char *payload_json,
    uint64_t timestamp_unix_secs,
    char out_ticket[65]
);

Behavior:

Reads current prev_leaf_hash (Hₙ₋₁).

Concatenates raw_binary(Hₙ₋₁) (32 bytes) + UTF8(payload_json).

Computes SHA-256 digest → 32 bytes.

Hex-encodes digest → 64 char string → writes into out_ticket.

Stores a pending entry: { prev_hash: Hₙ₋₁, payload_json, leaf_hash: Hₙ, timestamp, retry_count=0, status='pending' }.

Leaves prev_leaf_hash unchanged.

Returns 0.

Errors: Returns nonzero if:

JSON too large or invalid

Storage full

Internal inconsistency

Confirming Tickets

// Confirm or reject a pending ticket. Returns 0 on success.
// - client_ptr: CJT client handle.
// - leaf_hash_hex: 64-char hex string of the ticket to confirm.
// - status: 1 if DB insert succeeded, 0 if DB insert failed.
// - error_msg: optional NUL-terminated string (only for status=0) explaining the failure.

int cjt_confirm_ticket(
    void *client_ptr,
    const char leaf_hash_hex[65],
    int status,
    const char *error_msg
);

Behavior:

If status == 1:

Marks pending entry leaf_hash_hex → committed.

Sets prev_leaf_hash = leaf_hash_hex in memory and persists.

If status == 0:

Leaves pending entry status as pending (chain anchor still Hₙ₋₁).

Appends an “orphan event” leaf:

{ "type":"orphan", "orig_hash":"leaf_hash_hex", "error_msg":"error_msg", "timestamp":"…" }

Chained from Hₙ₋₁, stored permanently in Merkle tree.

Moves (leaf_hash_hex, payload_json) to the parking lot with retry_count=0, last_error=error_msg, status='pending'.

Returns 0 on success, nonzero if leaf_hash_hex is not found among pending or some internal error.

Retry Pending Payloads

// Retries the next pending payload. Returns:
//   0 = payload just committed (chain advanced)
//   1 = still pending (DB rejected again)
//   2 = marked as permanent failure (exceeded max_retries)
//  <0 = no pending entries or internal error

int cjt_retry_next_pending(
    void *client_ptr,
    int (*db_insert_callback)(
        const char *prev_leaf_hash,
        const char *payload_json,
        const char *leaf_hash_hex
    )
);

Behavior:

Pop the oldest entry with status='pending'.

Recompute digest = SHA256(raw(prev_hash) || payload_json) → hex → computed_hash.

If computed_hash != leaf_hash_hex, mark entry failed_permanent, return -2.

Call db_insert_callback(prev_hash, payload_json, leaf_hash_hex).

If callback returns 1 (success):

Mark entry committed.

Update prev_leaf_hash = leaf_hash_hex (persist to state).

Return 0.

If callback returns 0 (failure):

Increment retry_count, set last_error.

If retry_count < max_retries: return 1.

Else (>= max_retries): mark failed_permanent, return 2.

Lookup & Monitoring

// Check if a leaf exists (in pending or committed). Returns 1 if yes, 0 if no, <0 on error.
int cjt_leaf_exists(void *client_ptr, const char leaf_hash_hex[65]);

// Get the latest committed leaf hash. Fills out_recent_hash[65]. Returns 0 on success.
int cjt_get_latest_leaf_hash(void *client_ptr, char out_recent_hash[65]);

// Get the count of pending entries. Returns >=0 count.
int cjt_pending_count(void *client_ptr);

// List up to max_entries pending leaf hashes into out_hashes (array of char[65]).
// Returns number filled, <0 on error.
int cjt_list_pending(
    void *client_ptr,
    char (*out_hashes)[65],
    int max_entries
);

Database Trigger Configuration

To enforce chain integrity, each audited table must have a trigger that rebuilds the canonical JSON and checks the ticket. Below is a sample PostgreSQL trigger for a transactions table.

Canonical JSON Reconstruction

Keys must appear in the exact same order the application uses when building payload_json.

In Postgres, jsonb_build_object preserves the insertion order. Casting to TEXT yields a canonical representation (keys sorted lexicographically if built via jsonb_build_object).

Example fields:

prev_leaf_hash: stored in transactions.prev_leaf_hash

user_id: NEW.user_id

amount: NEW.amount

currency: NEW.currency

timestamp: NEW.created_at

Sample PostgreSQL Trigger

-- 1. Define the function that verifies CJT chain
CREATE OR REPLACE FUNCTION transactions_verify_turnstile() RETURNS TRIGGER AS $$
DECLARE
  rebuilt_json JSONB;
  computed_hash TEXT;
BEGIN
  IF TG_OP = 'INSERT' THEN
    -- Step A: Rebuild the JSON as the application did
    rebuilt_json := jsonb_build_object(
       'prev_leaf_hash', NEW.prev_leaf_hash,
       'data',          jsonb_build_object(
                            'action',   'create_transaction',
                            'tx_id',    NEW.id,
                            'user_id',  NEW.user_id,
                            'amount',   NEW.amount,
                            'currency', NEW.currency,
                            'timestamp', to_char(NEW.created_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
                         )
    );

    -- Step B: Compute SHA256(raw(prev_hash) || UTF8(rebuilt_json))
    computed_hash := encode(
       digest(
         decode(NEW.prev_leaf_hash, 'hex') || rebuilt_json::TEXT::BYTEA,
         'sha256'
       ),
       'hex'
    );

    -- Step C: Compare to provided leaf hash
    IF computed_hash <> NEW.cj_leaf_hash THEN
      RAISE EXCEPTION 'Turnstile hash mismatch: expected % but got %',
        computed_hash, NEW.cj_leaf_hash;
    END IF;
    RETURN NEW;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- 2. Attach the trigger to the table
CREATE TRIGGER transactions_turnstile_check
  BEFORE INSERT ON transactions
  FOR EACH ROW
  EXECUTE FUNCTION transactions_verify_turnstile();

-- 3. Unique index to prevent replay
CREATE UNIQUE INDEX ON transactions(cj_leaf_hash);

Notes:

If you also want to verify updates, extend the trigger to IF TG_OP IN ('INSERT', 'UPDATE').

Ensure the data object’s keys and ordering match exactly how the application built payload_json.

If your table has additional fields, include them in data in precise order.

Integration Examples

C/C++ Example

Below is a concise example demonstrating how a C application might use CJT’s FFI for the Turnstile pattern.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "cjt_ffi.h"  // Declarations of CJT functions

// Global CJT client handle
static void *cjt_client = NULL;

// Simple DB insert function (pseudocode-wrapping your database client)
int db_insert_transaction(
    int tx_id,
    int user_id,
    double amount,
    const char *currency,
    const char *prev_hash,
    const char *leaf_hash)
{
    // Example using a hypothetical db_execute() API:
    char *sql = "INSERT INTO transactions (id, user_id, amount, currency, prev_cj_leaf_hash, cj_leaf_hash) VALUES ($1,$2,$3,$4,$5,$6)";
    return db_execute(sql, tx_id, user_id, amount, currency, prev_hash, leaf_hash);
}

int main(void) {
    // 1. Initialize CJT
    const char *config = "{\"storage_path\":\"/var/lib/cjt_data\", \"max_retries\":5, \"retry_interval_ms\":5000 }";
    cjt_client = cjt_init(config);
    if (!cjt_client) {
        fprintf(stderr, "[FATAL] Unable to initialize CJT\n");
        return 1;
    }

    // 2. Determine prev_hash (genesis or last known)
    char prev_hash[65];
    if (cjt_get_latest_leaf_hash(cjt_client, prev_hash) != 0) {
        // Genesis: all-zero hash if none yet
        memset(prev_hash, '0', 64);
        prev_hash[64] = '\0';
    }

    // 3. Build payload JSON
    int tx_id = 101;
    int user_id = 42;
    double amount = 125.50;
    const char *currency = "EUR";
    char payload[512];
    snprintf(payload, sizeof(payload),
        "{ \"prev_leaf_hash\": \"%s\", \"data\": { \"action\": \"create_transaction\", \"tx_id\": %d, \"user_id\": %d, \"amount\": %.2f, \"currency\": \"%s\", \"timestamp\": \"%s\" } }",
        prev_hash,
        tx_id,
        user_id,
        amount,
        currency,
        "2025-06-30T17:15:00Z" // Ideally current UTC time in ISO8601
    );

    // 4. Turnstile append
    char leaf_hash[65];
    int rc = cjt_turnstile_append(cjt_client, payload, (uint64_t)time(NULL), leaf_hash);
    if (rc != 0) {
        fprintf(stderr, "[ERROR] CJT turnstile append failed: %d\n", rc);
        cjt_destroy(cjt_client);
        return 1;
    }
    printf("Ticket issued: %s\n", leaf_hash);

    // 5. Attempt DB insert
    int db_rc = db_insert_transaction(tx_id, user_id, amount, currency, prev_hash, leaf_hash);
    if (db_rc == 0) {
        // 6a. DB success → confirm ticket
        cjt_confirm_ticket(cjt_client, leaf_hash, 1, NULL);
        printf("Transaction committed; chain advanced to %s\n", leaf_hash);
    } else {
        // 6b. DB failure → orphan
        cjt_confirm_ticket(cjt_client, leaf_hash, 0, "DB insert failure: duplicate tx_id");
        fprintf(stderr, "Transaction rejected; moved to parking lot\n");
    }

    // 7. Periodic retry (in production, run in background thread or cron)
    int retry_rc = cjt_retry_next_pending(cjt_client, db_insert_transaction);
    if (retry_rc == 0) {
        char new_hash[65];
        cjt_get_latest_leaf_hash(cjt_client, new_hash);
        printf("Pending payload committed; chain is now %s\n", new_hash);
    } else if (retry_rc == 1) {
        printf("Pending payload still failing; will retry later\n");
    } else if (retry_rc == 2) {
        printf("Pending payload marked as permanent failure\n");
    } else {
        printf("No pending tickets or internal error on retry\n");
    }

    // 8. Shutdown
    cjt_destroy(cjt_client);
    return 0;
}

Ruby on Rails Example

Below is an excerpt showing how a Rails app (e.g., Decidim or COGS) might integrate CJT in a Vote model.

# Gemfile
# Add the CJT Ruby wrapper gem (assume it's published as 'civic_journal')
gem 'civic_journal', '~> 1.0'

# config/initializers/cjt.rb
CJT.init(
  {
    storage_path:      '/var/lib/cjt_data',
    max_retries:       5,
    retry_interval_ms: 5000
  }.to_json
)

# app/models/vote.rb
class Vote < ApplicationRecord
  # Virtual attributes to hold CJT state during callbacks
  attr_accessor :cjt_prev_hash, :cjt_leaf_hash

  before_validation :append_to_cjt  # before saving, get a ticket
  after_create      :confirm_cjt    # after successful DB insert, confirm

  private

  def append_to_cjt
    # 1) Get current prev_hash from CJT
    @cjt_prev_hash = CJT.get_latest_hash rescue '0' * 64

    # 2) Build canonical JSON
    payload_data = {
      action:      'cast_vote',
      vote_id:     id || 'TBD',
      user_id:     user_id,
      proposal_id: proposal_id,
      choice:      choice,
      timestamp:   Time.now.utc.iso8601
    }
    payload_json = {
      prev_leaf_hash: @cjt_prev_hash,
      data:            payload_data
    }.to_json  # assume JSON.generate sorts keys lexicographically

    # 3) Turnstile append
    @cjt_leaf_hash = CJT.turnstile_append(payload_json)

    # 4) Assign to record so it's persisted
    self.prev_cj_leaf_hash = @cjt_prev_hash
    self.cj_leaf_hash      = @cjt_leaf_hash
  end

  def confirm_cjt
    # Rebuild JSON with actual ID & created_at
    payload_data = {
      action:      'cast_vote',
      vote_id:     id,
      user_id:     user_id,
      proposal_id: proposal_id,
      choice:      choice,
      timestamp:   created_at.utc.iso8601
    }
    payload_json = {
      prev_leaf_hash: @cjt_prev_hash,
      data:            payload_data
    }.to_json

    begin
      # DB already inserted; confirm success
      CJT.confirm_ticket(@cjt_leaf_hash, true)
    rescue => e
      # Unlikely, but if verification fails
      CJT.confirm_ticket(@cjt_leaf_hash, false, e.message)
    end
  end
end

Database Trigger (Postgres)

CREATE OR REPLACE FUNCTION votes_verify_turnstile() RETURNS TRIGGER AS $$
DECLARE
  rebuilt_json JSONB;
  computed_hash TEXT;
BEGIN
  IF TG_OP = 'INSERT' THEN
    rebuilt_json := jsonb_build_object(
       'prev_leaf_hash', NEW.prev_cj_leaf_hash,
       'data',         jsonb_build_object(
                           'action',      'cast_vote',
                           'vote_id',     NEW.id,
                           'user_id',     NEW.user_id,
                           'proposal_id', NEW.proposal_id,
                           'choice',      NEW.choice,
                           'timestamp',   to_char(NEW.created_at,'YYYY-MM-DD"T"HH24:MI:SS"Z"')
                        )
    );
    computed_hash := encode(
      digest(
        decode(NEW.prev_cj_leaf_hash, 'hex') || rebuilt_json::TEXT::BYTEA,
        'sha256'
      ),
      'hex'
    );
    IF computed_hash <> NEW.cj_leaf_hash THEN
      RAISE EXCEPTION 'Turnstile hash mismatch: expected % but got %',
        computed_hash, NEW.cj_leaf_hash;
    END IF;
    RETURN NEW;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER votes_turnstile_check
  BEFORE INSERT ON votes
  FOR EACH ROW
  EXECUTE FUNCTION votes_verify_turnstile();

CREATE UNIQUE INDEX ON votes(cj_leaf_hash);

General Pseudocode

Below is language-agnostic pseudocode capturing the Turnstile pattern. Adapt to your framework or environment.

function handleNewPayload(dataFields):
  # 1. Fetch the current chain anchor
  prev_hash = CJT.getLatestHash() or GENESIS_HASH

  # 2. Build canonical JSON
  payload_obj = {
    "prev_leaf_hash": prev_hash,
    "data": dataFields
  }
  payload_json = canonicalizeJSON(payload_obj)  # lex-order keys, no extra whitespace

  # 3. Append to CJT
  ticket_hash = CJT.turnstileAppend(payload_json, currentTime())

  # 4. Attempt DB insert
  success = db.insert(
    tableName,
    dataFields + {
      prev_cj_leaf_hash: prev_hash,
      cj_leaf_hash:      ticket_hash
    }
  )

  # 5. Confirm or orphan
  if success:
    CJT.confirmTicket(ticket_hash, status=1)
    prev_hash = ticket_hash  # chain advances
  else:
    CJT.confirmTicket(ticket_hash, status=0, error_msg="DB rejected row")
    # payload is now parked in CJT’s parking lot

# Background job (e.g. cron, scheduled thread)
function retryPending():
  while (true):
    result = CJT.retryNextPending(dbInsertCallback)
    if result == 0:
      # one payload committed; chain advanced
      continue  # pick next pending, if any
    else if result == 1:
      # still pending; wait before retry
      sleep(retryInterval)
      continue
    else if result == 2:
      # permanent failure; move on to next
      continue
    else:
      # no pending or internal error
      break

function dbInsertCallback(prev_hash, payload_json, leaf_hash):
  dataFields = extractDataFromJSON(payload_json)
  success = db.insert(
    tableName,
    dataFields + {
      prev_cj_leaf_hash: prev_hash,
      cj_leaf_hash:      leaf_hash
    }
  )
  return success ? 1 : 0

Configuration & Tuning

Fine‐tuning CJT’s performance, memory footprint, and parking‐lot behavior often requires adjusting configuration parameters.

Storage Paths

storage_path: Base directory or cloud bucket (e.g. s3://my-cjt-bucket) where CJT writes:

Merkle pages (L0, L1, ...)

State file (holding prev_leaf_hash)

Pending store file/database

Logs (optional)

Ensure exclusive access: only one CJT instance should write to a given storage_path unless you enable multi-instance coordination (advanced).

Page Sizes & Rollup Thresholds

max_leaves_per_page (L0):

Default: 1024

Memory trade-off: Larger → fewer disk writes but higher memory usage (L0 buffer holds N × average_leaf_size).

For small payloads (<1 KB), you can raise to 4096 or 8192; for large payloads (>100 KB), drop to 256.

max_children_per_page (L1+):

Default: 1024

Each child is a 32-byte hash + a few bytes metadata → ~32 KB per level.

Larger values mean fewer rollups but more memory for intermediate pages.

Parking Lot Settings

max_retries:

Default: 5

How many times to retry a DB insert before giving up (marking failed_permanent).

If DB is temporarily down, increase to a higher number (e.g. 10 or 20) so automated retries can bridge short outages.

retry_interval_ms:

Default: 5000 (5 seconds)

Delay between retry attempts. Setting too low may overload the DB; too high means delays in recovering from transient failures.

orphan_event_logging:

Boolean (true/false). When true, CJT logs every rejected ticket as an orphan leaf. If you want a leaner chain, set to false (payloads still park but no orphan leaf is recorded).

Memory & Disk Monitoring

Peak Memory Usage:

At most one L0 buffer + minimal metadata for L1+.

With default L0=1024, average payload=256 B → ~256 KB memory at peak.

Each higher level uses ≤ (max_children_per_page × 32 B) (default: ~32 KB).

Total: ~300–400 KB in memory plus overhead for atomics and mutexes.

Disk Usage:

Pages are written to storage_path as files or blobs. Each leaf is a small JSON or compressed JSON.

Typical compressed leaf: 100–500 B. With 1M leaves, expect 100–500 MB of storage before garbage collection.

Hierarchical rollups compress history: after L0 seals, only a 32 B hash moves up; raw leaves stay on disk for proof extraction.

Best Practices

Security Considerations

CMC/JWT Authentication

If exposing CJT over a network API, require mTLS or signed JWTs for each request. Only authorized clients should be able to call turnstileAppend or confirmTicket.

Filesystem Permissions

Ensure storage_path is only writable by the CJT service user. Run CJT under a dedicated, low-privilege OS account.

State Integrity

Store a signed copy of prev_leaf_hash (e.g. HMAC with a separate key) so any tampering with the state file is detectable. CJT verifies the signature on startup.

Backup & Disaster Recovery

Regularly snapshot storage_path to an offsite, read-only location (e.g. nightly incremental backups).

Periodically verify the integrity of backups: restore and run cjt_leaf_exists for random historical hashes.

Payload Size & Rate Limiting

Maximum Payload Size

Enforce a hard cap on payload_json size (e.g. 10 KB). Reject larger payloads immediately in cjt_turnstile_append.

Rate Limiting

At the application or API gateway level, throttle calls to cjt_turnstile_append per client (e.g. 10 calls/sec). Prevent a flood of requests from overwhelming CJT.

JSON Schema & Validation

Strict Schema Validation

Define a JSON Schema (e.g., using [JSON Schema Draft-07]) for each payload type (e.g. cast_vote, create_proposal).

Validate payload_json against the schema before hashing. Reject if missing fields or extras.

This prevents injection of unexpected fields or malicious content.

Key Ordering & Canonicalization

Use a consistent, deterministic JSON canonicalizer:

In Rust: serde_json::to_string(&BTreeMap) or canonical_json crate.

In Postgres trigger: jsonb_build_object (in insertion order) and cast to TEXT.

In Rails: use ActiveSupport::JSON.encode on an HashWithIndifferentAccess sorted by keys.

Handling Orphaned Entries

Monitoring & Alerts

Track the count of pending entries (cjt_pending_count). Set thresholds: if >100 pending, trigger an alert.

If any entry reaches failed_permanent, notify an administrator with details (leaf hash, payload, last error).

Admin Review Dashboard

Build a lightweight UI or CLI that:

Lists pending hashes (cjt_list_pending).

Displays payload_JSON and last_error for each.

Allows manual “retry”, “skip (mark permanent)”, or “delete” operations.

Escalation Policies

Establish a rule: if a payload fails 3 times consecutively, assign to a developer to inspect. If it fails 10 times, automatically mark failed_permanent and route to legal/compliance team if data integrity is critical.

Orphan Event Auditing

Orphan leaves (logged with type:"orphan") become part of the Merkle log. Keep a separate table/collection of these events for quick lookup.

Export a CSV of all orphan events periodically for audit teams.

Troubleshooting & Common Pitfalls

"Hash Mismatch" Errors in DB Trigger

Symptoms: Application sees Turnstile hash mismatch: expected X but got Y.

Causes:

JSON key ordering mismatches between application and DB trigger.

Differences in timestamp formats (e.g. including milliseconds vs. not).

Whitespace or encoding differences (e.g. UTF-8 vs. UTF-16).

Remedies:

Use a canonical JSON builder (e.g. serde_json::to_string(&BTreeMap) and jsonb_build_object in Postgres).

Confirm both sides serialize the same fields in identical order.

Normalize timestamps with Time#iso8601 (no fractional seconds) on both sides.

Persistent Pending Count

Symptoms: Many pending entries accumulate, retries never succeed.

Causes:

Parking lot callback’s db_insert_callback has a bug (mismatched SQL or missing foreign key).

Database schema changed (e.g., new NOT NULL column) but JSON payload not updated.

Remedies:

Inspect last_error for the top pending entry.

Rebuild payload JSON to include new fields or fix data types.

Manually retry and confirm the corrected schema.

Disk Space Exhaustion

Symptoms: CJT errors with “disk full” or “unable to write page”.

Causes:

Unbounded growth of raw pages and orphan logs.

No cleanup of old L0/L1 pages after long-term archival.

Remedies:

Configure a retention policy: keep high-level rollups for last N years, archive older pages offline.

Enable compression on page files (if CJT supports zstd or gzip).

Monitor disk usage and set alerts at 80 % capacity.

Race Conditions in Prev_Hash Updates

Symptoms: Two concurrent cjt_turnstile_append calls return the same ticket_hash or use stale prev_leaf_hash.

Causes:

CJT’s internal prev_leaf_hash read/write not locked, allowing concurrent readers.

Multiple app instances connected to the same storage_path without coordination.

Remedies:

Ensure CJT is running as a single process or coordinate using a distributed lock (e.g. Redis lock) before calling cjt_turnstile_append.

If multi-instance, use a leader‐election pattern so only one node issues tickets, or use a centralized CJT service.

Unexpected Orphans After Confirm

Symptoms: cjt_confirm_ticket(..., status=1) still leaves the ticket pending.

Causes:

Application never persisted the DB insert (rolled back after confirm).

CJT’s pending store corrupted or out of sync.

Remedies:

Confirm the DB insert truly committed before calling confirm_ticket.

Check CJT logs for errors while marking as committed.

 
Confirm the DB insert truly committed before calling 
confirm_ticket .
 Check CJT logs for errors while marking as committed.
 If pending-store corruption suspected, restore from backup and replay known roots.
 Further Reading & References
 
CivicJournal-Time GitHub Repository: https://github.com/your_org/civicjournal-time
 AGPL 3.0 License: https://www.gnu.org/licenses/agpl-3.0.en.html
 JSON Schema Specification (Draft 07): https://json-schema.org/draft-07/
 PostgreSQL JSONB & Hashing Functions:
 jsonb_build_object : https://www.postgresql.org/docs/current/functions-json.html
 digest (pgcrypto): https://www.postgresql.org/docs/current/pgcrypto.html
 encode /
 decode : https://www.postgresql.org/docs/current/functions-binarystring.html
 Merkle Tree Fundamentals: https://en.wikipedia.org/wiki/Merkle_tree
 Rust 
serde_json Canonical JSON: https://docs.rs/serde_json/latest/serde_json/
 CJI’s Parking Lot Design Patterns: https://blog.yourorg.com/turnstile-parking-lot
 End of TURNSTILE.md: Comprehensive Turnstile System documentation for CivicJournal-Time