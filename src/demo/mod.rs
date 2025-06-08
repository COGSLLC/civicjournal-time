//! Demo mode utilities for generating synthetic data and verifying database integration.
//!
//! This module imitates a production application that writes journal deltas and
//! persists them to a PostgreSQL database. It exposes helpers for driving the
//! simulator, generating leaves, performing rollups and snapshots, and exploring
//! historical state.

#[cfg(feature = "demo")]
pub mod config;
#[cfg(feature = "demo")]
pub mod simulator;
#[cfg(feature = "demo")]
pub mod generator;
#[cfg(feature = "demo")]
pub mod rollup;
#[cfg(feature = "demo")]
pub mod snapshot;
#[cfg(feature = "demo")]
pub mod explorer;
#[cfg(feature = "demo")]
pub mod postgres;

#[cfg(feature = "demo")]
pub use config::DemoConfig;
#[cfg(feature = "demo")]
pub use simulator::Simulator;
