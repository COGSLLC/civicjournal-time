# CivicJournal Time - Testing Strategy

## Table of Contents
1. [Testing Philosophy](#testing-philosophy)
2. [Global Test Architecture](#global-test-architecture)
3. [Test Types](#test-types)
4. [Test Organization](#test-organization)
5. [Test Data Management](#test-data-management)
6. [Test Fixtures](#test-fixtures)
7. [Time-based Testing](#time-based-testing)
8. [Rollup Testing](#rollup-testing)
9. [Performance Testing](#performance-testing)
10. [Property Testing](#property-testing)
11. [Fuzz Testing](#fuzz-testing)
12. [Test Coverage](#test-coverage)
13. [CI/CD Integration](#cicd-integration)
14. [Test Execution](#test-execution)
15. [Test Naming Conventions](#test-naming-conventions)

## Testing Philosophy

Our testing approach follows these core principles:

- **Reliability**: Tests should be deterministic and not flaky
- **Isolation**: Tests should not depend on external services or shared state
- **Performance**: Tests should be fast to encourage frequent execution
- **Maintainability**: Tests should be as clear and simple as possible
- **Coverage**: Aim for high coverage of both happy paths and edge cases
- **Reproducibility**: Tests should produce the same results given the same inputs
- **Documentation**: Tests should serve as living documentation of the system's behavior

## Global Test Architecture

### Global State Management

To ensure test isolation and reliability, we use a global mutex for managing shared test state:

```rust
// In src/core/mod.rs
lazy_static! {
    pub static ref SHARED_TEST_ID_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::new(());
}

/// Resets all global ID counters for tests
pub fn reset_global_ids() {
    use crate::core::page::NEXT_PAGE_ID;
    use crate::core::leaf::NEXT_LEAF_ID;
    
    NEXT_PAGE_ID.store(0, std::sync::atomic::Ordering::SeqCst);
    NEXT_LEAF_ID.store(0, std::sync::atomic::Ordering::SeqCst);
}
```

### Test Structure

Each test that modifies global state should follow this pattern:

```rust
#[tokio::test]
async fn test_example() {
    // 1. Acquire the global test mutex
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    
    // 2. Reset global state
    reset_global_ids();
    
    // 3. Run test code
    // ...
    
    // 4. The mutex is automatically released when _guard goes out of scope
}
```

### Async Testing

For async tests, we use `tokio::test` with the multi-threaded runtime:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_concurrent_operations() {
    // Test code here
}
```

## Test Types

### 1. Unit Tests
- **Location**: `src/**/tests/`
- **Purpose**: Test individual functions and methods in isolation
- **Dependencies**: Use mocks for all external dependencies
- **Examples**:
  - Time window calculations
  - Delta application logic
  - Chunk management

### 2. Integration Tests
- **Location**: `tests/integration/`
- **Purpose**: Test interactions between components
- **Dependencies**: Real implementations of dependencies
- **Examples**:
  - Storage layer interactions
  - API request/response cycles
  - Cross-module functionality

### 3. End-to-End Tests
- **Location**: `tests/e2e/`
- **Purpose**: Test complete workflows
- **Dependencies**: Full application stack
- **Examples**:
  - Complete write/read cycle
  - Recovery scenarios
  - Configuration changes

### 4. Property Tests
- **Location**: `tests/property/`
- **Purpose**: Test properties of functions with generated inputs
- **Dependencies**: `proptest` crate
- **Examples**:
  - Serialization/deserialization roundtrips
  - Idempotent operations
  - Commutative operations

### 5. Fuzz Tests
- **Location**: `tests/fuzz/`
- **Purpose**: Find edge cases through random input generation
- **Dependencies**: `cargo-fuzz`
- **Examples**:
  - Corrupt chunk detection
  - Malformed input handling

## Test Organization

### Directory Structure

```
tests/
├── integration/       # Integration tests
│   ├── storage/       # Storage backend tests
│   ├── api/           # API integration tests
│   └── helpers.rs     # Test helpers
├── e2e/               # End-to-end tests
│   ├── recovery/      # Recovery scenarios
│   └── workflows/     # Common workflows
├── property/          # Property-based tests
│   ├── delta.rs       # Delta properties
│   └── chunk.rs       # Chunk properties
└── fuzz/              # Fuzz tests
    ├── fuzz_targets/  # Cargo-fuzz targets
    └── corpus/        # Test corpora
```

Recent test files in the root `tests/` directory include `file_storage_tests.rs`, `time_manager_tests.rs`, `basic_types_tests.rs`, and the expanded `memory_storage_tests.rs` which verifies in-memory backend behavior. The memory storage tests now exercise leaf lookup, page deletion, clearing logic, concurrency checks, targeted and level-wide failure simulation in addition to page summaries. The basic types suite checks conversions for common enums and error helper constructors. The `turnstile_tests.rs` file validates persistence and retry logic along with invalid JSON and bad hash handling. A `page_behavior_tests.rs` file covers canonical hashing of net patches and page finalization rules.

### Test Modules

Each test module should follow this structure:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::{setup, teardown};
    
    #[test]
    fn test_feature() {
        // Arrange
        let fixture = setup();
        
        // Act
        let result = function_under_test();
        
        // Assert
        assert!(result.is_ok());
        
        // Cleanup
        teardown(fixture);
    }
}
```

## Time-based Testing

### Controlling Time in Tests

For time-dependent tests, we use fixed timestamps to ensure determinism:

```rust
use chrono::{DateTime, TimeZone, Utc};

fn create_test_timestamps() -> Vec<DateTime<Utc>> {
    let base_time = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    vec![
        base_time,
        base_time + chrono::Duration::seconds(30),  // 30 seconds later
        base_time + chrono::Duration::minutes(5),   // 5 minutes later
        base_time + chrono::Duration::hours(1),     // 1 hour later
    ]
}
```

## Rollup Testing

### Testing Rollup Behavior

When testing rollup functionality, verify:

1. **Page Finalization**
   - Pages are finalized when they reach max_leaves_per_page
   - Pages are finalized when they exceed max_page_age_seconds
   - Finalized pages are properly hashed and stored

2. **Rollup Process**
   - Child pages are correctly rolled up to parent pages
   - Merkle roots are recalculated during rollup
   - The rollup process respects the time hierarchy levels

3. **Active Page Management**
   - Only one active page per level exists at any time
   - New pages are created when needed
   - Active pages correctly accept new leaves

## Test Data Management

### Test Data Generation

Use the `test_utils` module for generating test data:

```rust
mod test_utils {
    use crate::core::delta::{Delta, Operation};
    use uuid::Uuid;
    
    pub fn generate_test_deltas(count: usize) -> Vec<Delta> {
        (0..count)
            .map(|i| Delta {
                operation: Operation::Create {
                    object_id: Uuid::new_v4(),
                    data: format!("test_data_{}", i).into_bytes(),
                },
                timestamp: Utc::now(),
            })
            .collect()
    }
}
```

### Test Isolation

- Each test should clean up after itself
- Use unique identifiers for test resources
- Consider using `tempfile` for temporary storage

## Test Fixtures

### Common Fixtures

Define common test scenarios in `tests/fixtures.rs`:

```rust
pub struct TestContext {
    pub storage: Arc<dyn StorageBackend>,
    pub config: Config,
}

impl TestContext {
    pub fn new() -> Self {
        let config = Config {
            chunk_size: 1000,
            retention_period: Duration::days(7),
            // ... other test defaults
        };
        
        Self {
            storage: Arc::new(MemoryStorage::new()),
            config,
        }
    }
}
```

## Performance Testing

### Benchmarking

Use `criterion` for benchmarks in `benches/`:

```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn benchmark_chunk_creation(c: &mut Criterion) {
    c.bench_function("create_chunk", |b| {
        b.iter(|| {
            // Benchmark code here
        })
    });
}

criterion_group!(benches, benchmark_chunk_creation);
criterion_main!(benches);
```

### Load Testing

- Use `k6` or `locust` for HTTP API load testing
- Test with realistic workloads
- Measure:
  - Throughput
  - Latency distribution
  - Memory usage

## Property Testing

### Using Proptest

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_serialization_roundtrip(delta in any::<Delta>()) {
        let serialized = serde_json::to_vec(&delta).unwrap();
        let deserialized: Delta = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(delta, deserialized);
    }
}
```

## Fuzz Testing

### Fuzz Target Example

```rust
// fuzz/fuzz_targets/chunk_parsing.rs
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(chunk) = Chunk::from_bytes(data) {
        let _ = chunk.validate();
    }
});
```

## Test Coverage

### Coverage Reporting

Use `cargo-tarpaulin` for coverage:

```bash
# Install
cargo install cargo-tarpaulin

# Generate coverage report
cargo tarpaulin --out Html
```

### Coverage Goals

- Core logic: 95%+
- Storage backends: 90%+
- API endpoints: 85%+
- Overall: 90%+

## CI/CD Integration

### GitHub Actions

```yaml
name: Test

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          components: rustfmt, clippy
      
      - name: Run tests
        run: cargo test --all-features
        
      - name: Run clippy
        run: cargo clippy -- -D warnings
        
      - name: Check formatting
        run: cargo fmt -- --check
```

## Test Execution

### Running Tests

```bash
# Run all tests
cargo test

# Run integration tests only
cargo test --test integration

# Run tests with logging
RUST_LOG=debug cargo test -- --nocapture

# Run benchmarks
cargo bench

# Run fuzz tests
cargo +nightly fuzz run chunk_parsing
```

## Test Naming Conventions

### Test Naming

Use the following patterns for test names:

- `test_<method>_when_<condition>_then_<expected_behavior>`
- `test_<method>_with_<input>_expect_<output>`
- `test_<feature>_<scenario>`

### Example Test Names

```rust
#[test]
fn test_add_delta_when_storage_full_then_returns_error() { ... }

#[test]
fn test_chunk_merge_with_empty_chunk_returns_original() { ... }

#[test]
fn test_recovery_after_crash() { ... }
```

## Best Practices

1. **Test One Thing**: Each test should verify a single behavior
2. **Use Descriptive Names**: Test names should describe the scenario and expected outcome
3. **Keep Tests Fast**: Tests should run in milliseconds
4. **Minimize Dependencies**: Mock external services
5. **Test Error Cases**: Verify error handling and edge cases
6. **Use Property Tests**: For verifying invariants
7. **Document Test Assumptions**: Use comments to explain non-obvious test logic
8. **Clean Up Resources**: Ensure tests don't leak resources
9. **Run Tests in Parallel**: When possible, enable parallel test execution
10. **Review Test Failures**: Don't ignore flaky tests - fix the underlying issues

## Adding New Tests

When adding new functionality, follow this process:

1. Add unit tests for new functions and methods
2. Add integration tests for component interactions
3. Add property tests for core algorithms
4. Update relevant documentation
5. Ensure all tests pass before merging

## Debugging Tests

For debugging test failures:

```bash
# Run a specific test with debug output
RUST_BACKTRACE=1 cargo test test_name -- --nocapture

# Run with logging
RUST_LOG=debug cargo test -- --nocapture

# Debug with LLDB
rust-lldb -- cargo test test_name
```

## Performance Considerations

- Use `#[ignore]` for slow tests that don't need to run on every commit
- Consider using `#[cfg(test)]` to conditionally compile test-only code
- Use `lazy_static` for expensive test setup
- Consider using `test-case` for parameterized tests

## Security Testing

- Test for common vulnerabilities (e.g., SQL injection, XSS)
- Fuzz test all parsers and deserializers
- Test with malformed inputs
- Verify proper error handling and logging

## Documentation Tests

```rust
/// Adds two numbers together.
///
/// # Examples
///
/// ```
/// # use my_crate::add;
/// assert_eq!(add(2, 2), 4);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

Run documentation tests with:

```bash
cargo test --doc
```

## Continuous Integration

Ensure your CI pipeline includes:

1. Unit tests
2. Integration tests
3. Documentation tests
4. Code formatting check
5. Clippy lints
6. Security audit (cargo-audit)
7. Coverage reporting

## Implemented Tests

The repository includes a comprehensive suite of tests covering all major
components:

- `src/core/leaf.rs` – unit tests for `JournalLeaf` creation, ID management and
  `LeafData` serialization.
- `src/core/page.rs` – unit tests validating page hashing logic and
  serialization round trips.
- `src/core/merkle.rs` – exhaustive tests of `MerkleTree` construction and
  proof generation.
- `src/config/tests/config_mod_tests.rs` – configuration loading,
  environment overrides and validation.
- `tests/hash_tests.rs` – hashing helper functions.
- `tests/memory_storage_*` – unit tests for the in‑memory storage backend.
- `tests/file_storage_*` – unit and integration tests for the file storage
  backend, including an error test for permission-denied writes, a check
  that `load_page_by_hash` skips invalid files, and validation that loading
  a page fails when the file header is too short.
- `tests/time_manager_*` – tests for the time hierarchy manager including
  rollup and retention behaviour.
- `tests/query_engine_tests.rs` – query engine tests such as delta reports and
  page chain integrity (including the missing-page scenario).
- `tests/api_*` – synchronous and asynchronous API tests.
- `tests/turnstile_*` – tests for the turnstile queuing system.
- `tests/integration/full_workflow.rs` – full workflow integration test.

These tests can be run with `cargo test` and are executed automatically in CI.

## Test Maintenance

- Regularly review and update tests
- Remove or update flaky tests
- Keep test data up to date
- Monitor test execution time
- Update documentation when test behavior changes
