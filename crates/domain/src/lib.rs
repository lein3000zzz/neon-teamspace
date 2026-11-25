//! Domain types, validation and permission rules.
//!
//! This crate does no I/O and has no async code. Everything here is plain data and pure
//! functions, so the rules can be tested exhaustively without a database.

#[macro_use]
mod text_enum;

pub mod agents;
pub mod auth;
pub mod databases;
pub mod duration;
pub mod hue;
pub mod jobs;
pub mod mail;
pub mod pages;
pub mod settings;
pub mod workspaces;
