use std::fmt::Write as _;

use super::registry::Registry;
use super::value::{Kind, Value};

impl Registry {
    /// The settings reference as a markdown page: one table row per setting.
    pub fn reference_markdown(&self) -> String {
        let mut out = String::from(
            "# Settings reference\n\
             \n\
             Runtime settings apply without a restart. Each one can be set in four places, \
             and the first that has a value wins: its environment variable, the YAML file \
             under `settings.<key>`, the admin page, then the default. A value from the \
             environment or the YAML file cannot be changed on the admin page.\n\
             \n\
             | Key | Type | Default | Environment variable | Description |\n\
             |---|---|---|---|---|\n",
        );

        for def in self.definitions() {
            // Writing to a String cannot fail.
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | `{}` | {} |",
                def.key(),
                kind_label(def.kind()),
                default_label(def.default_value()),
                def.env_var(),
                escape(def.description()),
            );
        }

        out
    }
}

fn kind_label(kind: Kind) -> String {
    match kind {
        Kind::Enum(options) => format!("enum: {}", options.join(", ")),
        other => other.to_string(),
    }
}

fn default_label(value: &Value) -> String {
    match value {
        Value::Secret(_) => "empty".to_owned(),
        Value::String(s) if s.is_empty() => "empty".to_owned(),
        other => format!("`{}`", escape(&other.to_string())),
    }
}

/// Keeps a value inside its table cell.
fn escape(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::settings::{Definition, Key};

    use super::*;

    #[test]
    fn rows_list_key_type_default_env_and_description() {
        let registry = Registry::builder()
            .define(Definition::string(
                Key::new("instance.name"),
                "Neon Workspace",
                "The name | shown.",
            ))
            .define(Definition::enumeration(
                Key::new("auth.registration"),
                &["open", "invite", "closed"],
                "invite",
                "Who may sign up.",
            ))
            .define(Definition::duration(
                Key::new("auth.session_idle"),
                Duration::from_secs(7 * 24 * 3600),
                "Idle sign-out.",
            ))
            .define(Definition::secret(
                Key::new("mail.password"),
                "The password.",
            ))
            .build()
            .expect("valid registry");

        let markdown = registry.reference_markdown();
        let rows: Vec<&str> = markdown.lines().filter(|l| l.starts_with("| `")).collect();

        assert_eq!(
            rows,
            [
                "| `auth.registration` | enum: open, invite, closed | `invite` | `NEON_SETTINGS__AUTH__REGISTRATION` | Who may sign up. |",
                "| `auth.session_idle` | duration | `7days` | `NEON_SETTINGS__AUTH__SESSION_IDLE` | Idle sign-out. |",
                "| `instance.name` | string | `Neon Workspace` | `NEON_SETTINGS__INSTANCE__NAME` | The name \\| shown. |",
                "| `mail.password` | secret | empty | `NEON_SETTINGS__MAIL__PASSWORD` | The password. |",
            ]
        );
    }
}
