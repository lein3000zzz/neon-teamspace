//! Runtime settings: the server's definitions, and the service that resolves them and
//! saves changes from the admin page.

mod definitions;
mod service;

pub use definitions::{INSTANCE_NAME, auth, registry};
pub use service::{Change, Entry, LoadError, SettingsService, UpdateError};
