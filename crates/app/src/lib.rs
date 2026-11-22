//! Use cases.
//!
//! Use cases call the repositories directly and own their transactions. Traits exist only
//! for side effects that tests replace: mail, blob storage, realtime broadcast, rate limits.

pub mod auth;
pub mod clock;
pub mod crypto;
pub mod jobs;
pub mod mail;
pub mod periodic;
pub mod ratelimit;
pub mod settings;
