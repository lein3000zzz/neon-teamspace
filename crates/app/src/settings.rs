//! Runtime settings: the server's definitions, and the service that resolves them and
//! saves changes from the admin page.

mod definitions;
mod service;

pub use definitions::{
    INSTANCE_NAME, STORAGE_OPTIONS, attachments, auth, history, mail, mail_set_up, mcp,
    notifications, oauth, pages, registry, webhooks, workspaces,
};
pub use service::{Change, Entry, LoadError, SettingsService, UpdateError};
