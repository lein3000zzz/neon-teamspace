//! Runtime settings: the server's definitions, and the service that resolves them and
//! saves changes from the admin page.

mod definitions;
mod service;

pub use definitions::{
    INSTANCE_NAME, auth, mail, mail_set_up, mcp, pages, registry, webhooks, workspaces,
};
pub use service::{Change, Entry, LoadError, SettingsService, UpdateError};
