//! neon-app's integration tests, in one binary, which links the crate and its dependencies
//! once for all of them. Tests that need spans or logs recorded are binaries of their own:
//! those that install telemetry for the whole process, and those that read what a subscriber
//! of their thread records. tracing caches whether a callsite is enabled for every thread at
//! once, so a test beside them without a subscriber could leave one of theirs unrecorded.

#[path = "../support/mod.rs"]
mod support;

mod agents;
mod attachments;
mod auth_accounts;
mod auth_admin;
mod auth_invites;
mod auth_sessions;
mod comments;
mod databases;
mod history;
mod jobs;
mod mail;
mod notifications;
mod oauth;
mod pages;
mod search;
mod settings;
mod sync;
mod teamspaces;
mod transfer;
mod workspace_admin;
mod workspace_invites;
mod workspaces;
