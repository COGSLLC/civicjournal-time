# CivicJournal Time

A hierarchical time-series delta compression system for efficient historical state management.

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
civicchain-time = { path = "../civicchain-time" }
```

Basic example:

```rust
use civicchain_time::{TimeHierarchy, Delta};
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
6. **Centuries**: 100 years

Each level can be configured with custom durations and roll-up thresholds.

## Performance

- **Insertion**: O(log n) for finding the appropriate chunk
- **Query**: O(log n + k) where k is the number of deltas to apply
- **Storage**: O(n) where n is the number of unique time chunks

## License

Copyright (C) 2025 [Your Name or Organization]

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.

### Commercial Licensing

For proprietary or commercial use, alternative licensing options are available. This allows you to use this software under different terms than the AGPL-3.0-only license. Please contact the maintainers at [your-email@example.com] for more information about commercial licensing options.

### Contributing

Contributions are welcome! Please note that all contributions will be licensed under the same AGPL-3.0-only license that covers the project. By contributing, you agree to license your contributions under these terms.
