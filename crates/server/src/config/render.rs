//! The configuration as `neon config check` prints it.

use std::fmt::Write as _;

use neon_domain::settings::{Registry, Source, Stored};
use serde::Serialize;

use super::{DatabaseConfig, Loaded, ServerConfig};

#[derive(Serialize)]
struct Shown<'a> {
    server: &'a ServerConfig,
    database: &'a DatabaseConfig,
    observability: &'a neon_observability::Config,
}

impl Loaded {
    /// The configuration as YAML with secrets redacted, followed by every runtime
    /// setting as the environment and the file leave it, each marked with its source.
    /// Ignored `NEON_` variables and the observability section's warnings are listed
    /// first, as comments.
    pub fn render(&self, registry: &Registry) -> Result<String, serde_saphyr::SerializeError> {
        let mut out = match &self.file {
            Some(path) => format!("# Read from {} and the environment.\n", path.display()),
            None => "# No config file: defaults and the environment only.\n".to_owned(),
        };
        if !self.ignored_vars.is_empty() {
            // Writing to a String cannot fail.
            let _ = writeln!(
                out,
                "# Ignored, as they set no configuration key: {}.",
                self.ignored_vars.join(", ")
            );
        }
        // A loaded configuration has no errors, so only warnings are left.
        let warnings = self
            .config
            .observability
            .validate()
            .unwrap_or_else(|invalid| invalid.warnings);
        for warning in warnings {
            // Writing to a String cannot fail.
            let _ = writeln!(out, "# Warning: {warning}.");
        }
        out.push_str(&serde_saphyr::to_string(&Shown {
            server: &self.config.server,
            database: &self.config.database,
            observability: &self.config.observability,
        })?);

        out.push_str(
            "# Runtime settings. Those at their default take the value saved on the admin \
             page, if there is one.\nsettings:\n",
        );
        let snapshot = registry.resolve(&self.settings, &Stored::new());
        for (key, resolved) in snapshot.iter() {
            let Some(def) = registry.get(key) else {
                continue;
            };
            let source = match resolved.source {
                Source::Env => format!("set by {}", def.env_var()),
                Source::Yaml => "set in the config file".to_owned(),
                Source::Database | Source::Default => "default".to_owned(),
            };
            // Writing to a String cannot fail.
            let _ = writeln!(
                out,
                "  {key}: {} # {source}",
                resolved.value.to_display_json()
            );
        }

        Ok(out)
    }
}
