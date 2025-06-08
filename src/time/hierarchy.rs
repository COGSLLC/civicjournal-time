// src/time/hierarchy.rs

// Assuming JournalPage will be defined
// Assuming TimeLevel will be defined

use crate::error::CJError;
use crate::time::level::TimeLevel;
use chrono::{DateTime, Duration, Utc};

/// In-memory representation of the configured time hierarchy.
///
/// This type provides basic utilities for calculating page windows and
/// inspecting the hierarchy. Higher level orchestration lives in
/// `TimeHierarchyManager` but this struct encapsulates the static level
/// configuration.
#[derive(Debug, Clone)]
pub struct TimeHierarchy {
    /// Ordered list of levels from most granular (index 0) upward.
    pub levels: Vec<TimeLevel>,
}

impl TimeHierarchy {
    /// Create a new hierarchy from a list of levels.
    pub fn new(levels: Vec<TimeLevel>) -> Self {
        Self { levels }
    }

    /// Returns the configured levels.
    pub fn levels(&self) -> &[TimeLevel] {
        &self.levels
    }

    /// Calculate the start and end time window for a timestamp at a given level.
    pub fn page_window(
        &self,
        level_idx: usize,
        timestamp: DateTime<Utc>,
    ) -> Result<(DateTime<Utc>, DateTime<Utc>), CJError> {
        let level = self
            .levels
            .get(level_idx)
            .ok_or_else(|| CJError::InvalidInput(format!("Invalid level {level_idx}")))?;

        let duration = Duration::seconds(level.duration_seconds() as i64);
        let ts_nanos = timestamp.timestamp_nanos_opt().unwrap_or_default();
        let gran_nanos = duration.num_nanoseconds().unwrap_or(0);
        if gran_nanos == 0 {
            return Err(CJError::InvalidInput(
                "level duration cannot be zero".into(),
            ));
        }
        let window_start_nanos = (ts_nanos / gran_nanos) * gran_nanos;
        let start = DateTime::<Utc>::from_timestamp_nanos(window_start_nanos);
        Ok((start, start + duration))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn page_window_calculations() {
        let hierarchy = TimeHierarchy::new(vec![TimeLevel::Minute, TimeLevel::Hour]);

        let ts = Utc.with_ymd_and_hms(2024, 5, 15, 10, 30, 45).unwrap();

        let (start_min, end_min) = hierarchy.page_window(0, ts).unwrap();
        assert_eq!(
            start_min,
            Utc.with_ymd_and_hms(2024, 5, 15, 10, 30, 0).unwrap()
        );
        assert_eq!(
            end_min,
            Utc.with_ymd_and_hms(2024, 5, 15, 10, 31, 0).unwrap()
        );

        let (start_hr, end_hr) = hierarchy.page_window(1, ts).unwrap();
        assert_eq!(
            start_hr,
            Utc.with_ymd_and_hms(2024, 5, 15, 10, 0, 0).unwrap()
        );
        assert_eq!(end_hr, Utc.with_ymd_and_hms(2024, 5, 15, 11, 0, 0).unwrap());
    }
}
