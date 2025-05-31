// src/time/level.rs

use crate::error::CJError;
use serde::{Deserialize, Serialize};

// TODO: Define TimeLevel enum or structs for time hierarchy levels
// (Minute, Hour, Day, Month, Year, Decade, Century)
// Include methods for getting duration, parent/child levels, etc.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, PartialOrd, Ord)]
/// Represents the hierarchical levels in the time-based aggregation structure of the journal.
/// Each level corresponds to a specific time granularity.
pub enum TimeLevel {
    /// Level 0: Represents a minute time granularity.
    Minute,
    /// Level 1: Represents an hourly time granularity.
    Hour,
    /// Level 2: Represents a daily time granularity.
    Day,
    /// Level 3: Represents a monthly time granularity.
    Month,
    /// Level 4: Represents an annual time granularity.
    Year,
    /// Level 5: Represents a decadal (10 years) time granularity.
    Decade,
    /// Level 6: Represents a centurial (100 years) time granularity.
    Century,
}

impl TimeLevel {
    /// Converts a numeric representation (0-6) to a `TimeLevel` variant.
    ///
    /// # Arguments
    ///
    /// * `level_num` - The numeric level to convert.
    ///
    /// # Errors
    ///
    /// Returns `CJError::InvalidInput` if `level_num` is not between 0 and 6 (inclusive).
    pub fn from_level_num(level_num: u8) -> Result<Self, CJError> {
        match level_num {
            0 => Ok(TimeLevel::Minute),
            1 => Ok(TimeLevel::Hour),
            2 => Ok(TimeLevel::Day),
            3 => Ok(TimeLevel::Month),
            4 => Ok(TimeLevel::Year),
            5 => Ok(TimeLevel::Decade),
            6 => Ok(TimeLevel::Century),
            _ => Err(CJError::InvalidInput(format!("Invalid time level number: {}", level_num))),
        }
    }

    /// Converts a `TimeLevel` variant to its numeric representation (0-6).
    pub fn to_level_num(&self) -> u8 {
        match self {
            TimeLevel::Minute => 0,
            TimeLevel::Hour => 1,
            TimeLevel::Day => 2,
            TimeLevel::Month => 3,
            TimeLevel::Year => 4,
            TimeLevel::Decade => 5,
            TimeLevel::Century => 6,
        }
    }

    /// Returns the approximate duration of this time level in seconds.
    pub fn duration_seconds(&self) -> u64 {
        match self {
            TimeLevel::Minute => 60,
            TimeLevel::Hour => 3_600,             // 60 * 60
            TimeLevel::Day => 86_400,            // 24 * 3600
            TimeLevel::Month => 2_592_000,       // 30 * 86400 (approx)
            TimeLevel::Year => 31_536_000,       // 365 * 86400 (approx)
            TimeLevel::Decade => 315_360_000,     // 10 * 31536000 (approx)
            TimeLevel::Century => 3_153_600_000,  // 100 * 31536000 (approx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_conversion() {
        assert_eq!(TimeLevel::from_level_num(0).unwrap(), TimeLevel::Minute);
        assert_eq!(TimeLevel::Minute.to_level_num(), 0);
        assert!(TimeLevel::from_level_num(7).is_err());
    }

    #[test]
    fn test_duration() {
        assert_eq!(TimeLevel::Minute.duration_seconds(), 60);
        assert_eq!(TimeLevel::Hour.duration_seconds(), 3600);
    }
}
