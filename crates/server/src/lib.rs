//! The `neon` server: configuration, the HTTP API, the CLI and the wiring that starts and
//! stops everything in order.

pub mod cli;
pub mod config;
mod gauges;
pub mod health;
pub mod http;
pub mod runtime;
mod telemetry;
