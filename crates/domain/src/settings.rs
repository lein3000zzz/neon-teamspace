//! Runtime settings: what each one is, how raw values parse, and how the layers resolve.
//!
//! A value comes from the first layer that has one: an environment variable, the YAML
//! file, the database (written from the admin page), the setting's default. The first two
//! pin the setting, so the admin page cannot change it. The definitions themselves live
//! with the use cases; this module holds the types and rules they are built from.

mod definition;
mod docs;
mod registry;
mod snapshot;
mod value;

pub use definition::{Definition, ENV_PREFIX, Key, SettingType, YAML_PREFIX, checks};
pub use registry::{Registry, RegistryBuilder, RegistryError};
pub use snapshot::{Pinned, Problem, RawLayers, Resolved, Snapshot, Source, Stored};
pub use value::{Kind, REDACTED, Secret, Value};
