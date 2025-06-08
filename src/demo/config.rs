#![cfg(feature = "demo")]
use serde::Deserialize;
use chrono::NaiveDate;
use crate::{CJError, CJResult};
use std::fs;
use toml;

/// Configuration options for Demo Mode parsed from `Journal.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct DemoConfig {
    /// Start date of the simulated timeline (YYYY-MM-DD)
    pub start_date: NaiveDate,
    /// End date of the simulated timeline (YYYY-MM-DD)
    pub end_date: NaiveDate,
    /// Number of simulated users generating activity
    pub users: u32,
    /// Number of containers in the system
    pub containers: u32,
    /// Seed for the RNG to make runs reproducible
    pub seed: u64,
    /// Generation rate configuration
    #[serde(default)]
    pub rate: RateConfig,
    /// Leaf generation per tick configuration
    #[serde(default)]
    pub leaf_rate: LeafRate,
    /// Database connection string used for optional PostgreSQL integration
    pub database_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateConfig {
    pub real_seconds_per_month: f32,
}

impl Default for RateConfig {
    fn default() -> Self {
        Self {
            real_seconds_per_month: 0.5,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LeafRate {
    pub per_real_second: u32,
}

impl Default for LeafRate {
    fn default() -> Self {
        Self { per_real_second: 10 }
    }
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            start_date: NaiveDate::from_ymd_opt(2005, 1, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            users: 10,
            containers: 20,
            seed: 42,
            rate: RateConfig::default(),
            leaf_rate: LeafRate::default(),
            database_url: None,
        }
    }
}

impl DemoConfig {
    /// Load the `[demo]` section from a Journal.toml file.
    pub fn load(path: &str) -> CJResult<Self> {
        let data = fs::read_to_string(path)?;
        let value: toml::Value = toml::from_str(&data)?;
        match value.get("demo") {
            Some(section) => section.clone().try_into().map_err(CJError::from),
            None => Ok(DemoConfig::default()),
        }
    }
}
