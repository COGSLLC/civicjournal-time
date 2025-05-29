# CivicJournal Time

CivicJournal Time is a time-hierarchical Merkle-chained delta-log for managing historical state with cryptographic integrity.
It allows you to efficiently record, compress, and query state changes across seconds, centuries, or anything in between.

## Quick Start

### Prerequisites
- Rust toolchain (latest stable version recommended)
- Cargo (Rust's package manager)
- For C FFI: A C compiler (gcc, clang, or MSVC)

### Installation

1. Add the dependency to your `Cargo.toml`:
   ```toml
   [dependencies]
   civicjournal-time = { git = "https://github.com/COGSLLC/civicjournal-time" }
   ```

2. Or clone and use locally:
   ```bash
   git clone https://github.com/COGSLLC/civicjournal-time.git
   cd civicjournal-time
   cargo build
   ```

## Overview

CivicJournal Time provides a way to store and query state changes over time with varying levels of granularity, from milliseconds to centuries, using a fractal-like hierarchy of deltas. It's designed to efficiently manage historical data while maintaining the ability to reconstruct the state at any point in time.

## Features

- **Time-based chunking**: Automatically groups deltas into hierarchical time windows
- **Efficient storage**: Uses delta compression to minimize storage requirements
- **Fast queries**: Quickly retrieve state at any point in time
- **Cryptographic verification**: All changes are hashed and can be verified
- **Scalable**: Handles everything from seconds to centuries efficiently

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
civicjournal-time = { path = "../civicjournal-time" }
```

Basic example:

```rust
use civicjournal_time::{TimeHierarchy, Delta};
use chrono::{Utc, TimeZone};

fn main() -> anyhow::Result<()> {
    let mut hierarchy = TimeHierarchy::new();
    
    // Add a delta
    let time = Utc.with_ymd_and_hms(2023, 6, 15, 14, 30, 45).unwrap();
    let delta = Delta {
        timestamp: time,
        data: b"Initial state".to_vec(),
        hash: [0; 32],
        prev_hash: None,
    };
    
    hierarchy.add_delta(delta)?;
    
    // Get state at a specific time
    if let Some(state) = hierarchy.get_state_at_time(time)? {
        println!("State at {}: {:?}", time, state);
    }
    
    Ok(())
}
```

## Time Hierarchy

The system organizes time into a hierarchy of chunks:

1. **Minutes**: 60 seconds
2. **Hours**: 60 minutes
3. **Days**: 24 hours
4. **Months**: ~30 days
5. **Years**: 12 months
6. **Decades**: 10 years
7. **Centuries**: 10 decades

Each level can be configured with custom durations and roll-up thresholds.

## C FFI Interface

The library provides a C-compatible Foreign Function Interface (FFI) for integration with other languages, allowing for performance optimizations when interfacing with C or other languages that can consume C libraries.

### Features

- **Thread-safe operations**: Safe to use in multi-threaded environments
- **Memory safety**: Proper memory management with explicit allocation/deallocation
- **Error handling**: Comprehensive error codes and messages
- **Extensible**: Easy to add new functions as the library evolves

### Header File

Create a header file named `civicjournal_time.h` with the following content:

```c
/* civicjournal_time.h */
#ifndef CIVICJOURNAL_TIME_H
#define CIVICJOURNAL_TIME_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle for TimeHierarchy */
typedef struct CJTimeHierarchy civicjournal_time_hierarchy_t;

/* Error codes */
typedef enum {
    CIVICJOURNAL_SUCCESS = 0,
    CIVICJOURNAL_ERROR = 1,
    CIVICJOURNAL_IO_ERROR = 2,
    CIVICJOURNAL_SERIALIZATION_ERROR = 3,
    CIVICJOURNAL_VERIFICATION_ERROR = 4
} civicjournal_error_code_t;

/* Create a new time hierarchy */
civicjournal_time_hierarchy_t* civicjournal_time_hierarchy_new(void);

/* Free a time hierarchy */
void civicjournal_time_hierarchy_free(civicjournal_time_hierarchy_t* hierarchy);

/* Get time hierarchy statistics as JSON */
char* civicjournal_time_hierarchy_stats(const civicjournal_time_hierarchy_t* hierarchy);

/* Free a string allocated by the library */
void civicjournal_free_string(char* str);

/* Get error message for an error code */
char* civicjournal_get_error_message(civicjournal_error_code_t error_code);

#ifdef __cplusplus
}
#endif

#endif /* CIVICJOURNAL_TIME_H */
```

### Basic Usage

Here's a simple example of using the C API:

```c
#include <stdio.h>
#include "civicjournal_time.h"

int main() {
    // Create a new time hierarchy
    civicjournal_time_hierarchy_t* hierarchy = civicjournal_time_hierarchy_new();
    if (!hierarchy) {
        printf("Failed to create time hierarchy\n");
        return 1;
    }

    // Get statistics as JSON
    char* stats = civicjournal_time_hierarchy_stats(hierarchy);
    if (stats) {
        printf("Time hierarchy stats: %s\n", stats);
        civicjournal_free_string(stats);
    }

    // Clean up
    civicjournal_time_hierarchy_free(hierarchy);
    return 0;
}
```

### Building and Linking

1. **Build the Rust library**:
   ```bash
   cargo build --release
   ```

2. **Link with your C code**:

   * **Windows (MSVC)**:
     ```bash
     cl main.c /link civicjournal_time.lib
     ```

   * **Linux**:
     ```bash
     gcc -o main main.c -L. -lcivicjournal_time
     ```

   * **macOS**:
     ```bash
     gcc -o main main.c -L. -lcivicjournal_time
     ```

3. **Dynamic Libraries**:
   - Windows: `civicjournal_time.dll`
   - Linux: `libcivicjournal_time.so`
   - macOS: `libcivicjournal_time.dylib`

### Error Handling

All FFI functions that can fail return a `civicjournal_error_code_t`. The library provides a function to get a human-readable error message:

```c
const char* error_msg = civicjournal_get_error_message(error_code);
if (error_msg) {
    printf("Error: %s\n", error_msg);
    civicjournal_free_string((char*)error_msg);
}
```

## Contributing

We welcome contributions! Here's how you can help:

1. **Reporting Issues**
   - Check existing issues before creating a new one
   - Include steps to reproduce any bugs
   - Provide as much context as possible

2. **Code Contributions**
   - Fork the repository and create a feature branch
   - Follow the existing code style
   - Write tests for new functionality
   - Update documentation as needed
   - Submit a pull request with a clear description of changes

3. **Development Setup**
   ```bash
   # Clone the repository
   git clone https://github.com/yourusername/civicjournal-time.git
   cd civicjournal-time
   
   # Build in debug mode
   cargo build
   
   # Run tests
   cargo test
   
   # Build for release (with optimizations)
   cargo build --release
   ```

4. **Code Style**
   - Follow Rust's official style guidelines
   - Run `cargo fmt` before committing
   - Use `cargo clippy` to catch common mistakes

5. **Testing**
   - Unit tests should be placed in the same file as the code they test
   - Integration tests go in the `tests/` directory
   - Run all tests with `cargo test --all-features`

6. **Documentation**
   - Document all public APIs with Rustdoc comments
   - Keep the README up to date
   - Add examples for new features

### Using with Other Languages

#### Java (JNI)

The C FFI makes it easier to create Java bindings using JNI:

```java
public class CivicJournalTime {
    static {
        System.loadLibrary("civicjournal_time");
    }

    private long nativeHandle;

    public CivicJournalTime() {
        nativeHandle = createTimeHierarchy();
    }

    public String getStats() {
        return getTimeHierarchyStats(nativeHandle);
    }

    public void close() {
        if (nativeHandle != 0) {
            freeTimeHierarchy(nativeHandle);
            nativeHandle = 0;
        }
    }

    @Override
    protected void finalize() {
        close();
    }

    private native long createTimeHierarchy();
    private native String getTimeHierarchyStats(long handle);
    private native void freeTimeHierarchy(long handle);
}
```

#### Python (ctypes)

Similarly, Python can use the C FFI via ctypes:

```python
import ctypes
from ctypes import c_void_p, c_char_p

# Load the library
if sys.platform == "win32":
    lib = ctypes.CDLL("./civicjournal_time.dll")
else:
    lib = ctypes.CDLL("./libcivicjournal_time.so")

# Define function signatures
lib.civicjournal_time_hierarchy_new.restype = c_void_p
lib.civicjournal_time_hierarchy_stats.argtypes = [c_void_p]
lib.civicjournal_time_hierarchy_stats.restype = c_char_p
lib.civicjournal_free_string.argtypes = [c_char_p]

# Create a time hierarchy
hieararchy = lib.civicjournal_time_hierarchy_new()

# Get statistics
stats_json = lib.civicjournal_time_hierarchy_stats(hierarchy)
stats = ctypes.string_at(stats_json).decode('utf-8')
print(f"Stats: {stats}")

# Clean up
lib.civicjournal_free_string(stats_json)
lib.civicjournal_time_hierarchy_free(hierarchy)
```

## Performance

- **Insertion**: O(log n) for finding the appropriate chunk
- **Query**: O(log n + k) where k is the number of deltas to apply
- **Storage**: O(n) where n is the number of unique time chunks

## Please Use

This crate is part of the CivicJournal system and may form the backbone of cross-platform civic data tools.
If you're interested in contributing, porting to other languages, or using it in your project—reach out or fork away!

## Free Software License

Copyright (C) 2025 Cooperative Open Government Systems, LLC

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License along with this program.  If not, see <https://www.gnu.org/licenses/>.

### Commercial Licensing

For proprietary or commercial use, alternative licensing options are available. This allows you to use this software under different terms than the AGPL-3.0-only license. Please contact the maintainers at INFO@COGS.LLC for more information about commercial licensing options.

### Contributing

Contributions are welcome! Please note that all third-party contributions are accepted under the Apache License 2.0, unless otherwise agreed upon in writing with the maintainers.
By submitting a contribution, you grant the project maintainers the right to use, modify, sublicense, and commercialize your contribution under the terms of this project’s dual-licensing structure.
