//! The `neon` server: configuration, the HTTP API, the CLI and the wiring that starts and
//! stops everything in order.

pub mod cli;
pub mod config;
mod gauges;
mod health;
pub mod http;
mod mail;
mod mcp;
mod runtime;
mod stall;
pub mod storage;
mod telemetry;
mod webhooks;
