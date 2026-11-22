use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde_json::Value as Json;

use super::definition::{Definition, ENV_PREFIX, YAML_PREFIX};
use super::snapshot::{Pinned, Problem, RawLayers, Resolved, Snapshot, Source, Stored};

type RuleCheck = Arc<dyn Fn(&Snapshot) -> Result<(), String> + Send + Sync>;

/// A check across settings, such as "email verification needs a mail server". Its error
/// is reported against `key`, the setting whose value the rule refuses.
#[derive(Clone)]
struct Rule {
    key: &'static str,
    check: RuleCheck,
}

/// Every known setting, and the rules across them.
#[derive(Clone)]
pub struct Registry {
    definitions: BTreeMap<&'static str, Definition>,
    rules: Vec<Rule>,
}

/// A definition or rule that cannot be registered. These are programming errors, so a
/// test builds every registry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("setting key {0:?} must be dotted lowercase words like \"instance.name\"")]
    InvalidKey(&'static str),
    #[error("setting {0:?} is defined twice")]
    Duplicate(&'static str),
    #[error("settings {0:?} and {1:?} overlap: a setting cannot sit inside another")]
    Overlap(&'static str, &'static str),
    #[error("the default of setting {key:?} is invalid: {message}")]
    InvalidDefault { key: &'static str, message: String },
    #[error("a rule refers to {0:?}, which is not defined")]
    UnknownRuleKey(&'static str),
}

/// Collects definitions and rules, then checks them together in [`build`](Self::build).
#[derive(Default)]
pub struct RegistryBuilder {
    definitions: Vec<Definition>,
    rules: Vec<Rule>,
}

impl RegistryBuilder {
    pub fn define(mut self, definition: Definition) -> Self {
        self.definitions.push(definition);
        self
    }

    /// Adds a rule over the resolved settings. It runs before every write, against the
    /// settings as they would be after the write, and on start: see
    /// [`Registry::check_rules_on_start`]. Saved values that break it only warn there, so
    /// what it guards must check again where it is used.
    pub fn rule<F>(mut self, key: &'static str, check: F) -> Self
    where
        F: Fn(&Snapshot) -> Result<(), String> + Send + Sync + 'static,
    {
        self.rules.push(Rule {
            key,
            check: Arc::new(check),
        });
        self
    }

    pub fn build(self) -> Result<Registry, RegistryError> {
        let mut definitions = BTreeMap::new();

        for definition in self.definitions {
            let key = definition.key();
            if !is_valid_key(key) {
                return Err(RegistryError::InvalidKey(key));
            }
            definition
                .parse_json(&definition.default_value().to_stored_json())
                .map_err(|message| RegistryError::InvalidDefault { key, message })?;
            if definitions.insert(key, definition).is_some() {
                return Err(RegistryError::Duplicate(key));
            }
        }

        // Keys are paths in one YAML tree, so `a` and `a.b` cannot both be settings.
        for outer in definitions.keys() {
            for inner in definitions.keys() {
                if inner
                    .strip_prefix(outer)
                    .is_some_and(|rest| rest.starts_with('.'))
                {
                    return Err(RegistryError::Overlap(outer, inner));
                }
            }
        }

        if let Some(rule) = self
            .rules
            .iter()
            .find(|rule| !definitions.contains_key(rule.key))
        {
            return Err(RegistryError::UnknownRuleKey(rule.key));
        }

        Ok(Registry {
            definitions,
            rules: self.rules,
        })
    }
}

/// Dotted words of lowercase letters, digits and single underscores, starting with a
/// letter. Double underscores are what separate levels in environment variable names.
fn is_valid_key(key: &str) -> bool {
    key.split('.').all(|word| {
        word.starts_with(|c: char| c.is_ascii_lowercase())
            && !word.ends_with('_')
            && !word.contains("__")
            && word
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    })
}

impl Registry {
    pub fn builder() -> RegistryBuilder {
        RegistryBuilder::default()
    }

    pub fn get(&self, key: &str) -> Option<&Definition> {
        self.definitions.get(key)
    }

    /// Every definition in key order.
    pub fn definitions(&self) -> impl Iterator<Item = &Definition> {
        self.definitions.values()
    }

    /// Parses the environment and YAML values, reporting every problem at once. Unknown
    /// keys are problems too, so a typo fails the start instead of being ignored.
    pub fn pin(&self, raw: &RawLayers) -> Result<Pinned, Vec<Problem>> {
        let mut pinned = Pinned::default();
        let mut problems = Vec::new();

        for (var, text) in &raw.env {
            let key = var
                .strip_prefix(ENV_PREFIX)
                .map(|rest| rest.to_ascii_lowercase().replace("__", "."));
            match key.as_deref().and_then(|key| self.definitions.get(key)) {
                Some(def) => match def.parse_text(text) {
                    Ok(value) => {
                        pinned.env.insert(def.key(), value);
                    }
                    Err(message) => problems.push(Problem::new(var, message)),
                },
                None => problems.push(Problem::new(var, "no such setting")),
            }
        }

        for (key, json) in &raw.yaml {
            let path = format!("{YAML_PREFIX}{key}");
            match self.definitions.get(key.as_str()) {
                Some(def) => match def.parse_json(json) {
                    Ok(value) => {
                        pinned.yaml.insert(def.key(), value);
                    }
                    Err(message) => problems.push(Problem::new(path, message)),
                },
                None => problems.push(Problem::new(path, "no such setting")),
            }
        }

        if problems.is_empty() {
            Ok(pinned)
        } else {
            Err(problems)
        }
    }

    /// Parses values saved in the database. A row that is unknown or no longer valid, say
    /// after an upgrade made a check stricter, is skipped and returned as a warning: it
    /// must not keep the server from starting, and an admin can save a new value.
    pub fn parse_stored(
        &self,
        rows: impl IntoIterator<Item = (String, Json)>,
    ) -> (Stored, Vec<Problem>) {
        let mut stored = Stored::new();
        let mut warnings = Vec::new();

        for (key, json) in rows {
            match self.definitions.get(key.as_str()) {
                Some(def) => match def.parse_json(&json) {
                    Ok(value) => {
                        stored.insert(def.key(), value);
                    }
                    Err(message) => {
                        warnings.push(Problem::new(key, format!("saved value ignored: {message}")))
                    }
                },
                None => warnings.push(Problem::new(key, "saved value ignored: no such setting")),
            }
        }

        (stored, warnings)
    }

    /// Layers the values: environment, then YAML, then the database, then the default.
    pub fn resolve(&self, pinned: &Pinned, stored: &Stored) -> Snapshot {
        let values = self
            .definitions
            .iter()
            .map(|(&key, def)| {
                let (value, source) = [
                    (pinned.env.get(key), Source::Env),
                    (pinned.yaml.get(key), Source::Yaml),
                    (stored.get(key), Source::Database),
                ]
                .into_iter()
                .find_map(|(value, source)| value.map(|value| (value, source)))
                .unwrap_or((def.default_value(), Source::Default));

                (
                    key,
                    Resolved {
                        value: value.clone(),
                        source,
                    },
                )
            })
            .collect();

        Snapshot { values }
    }

    /// Runs every rule as the server starts, against what `pinned` and `stored` resolve
    /// to. A rule that the pinned values break alone, over the defaults, is an error: the
    /// environment and the config file contradict each other, and only they can fix it.
    /// A rule broken only with values saved from the admin page is returned as a warning,
    /// so a change to the environment never keeps the server from starting over what an
    /// admin saved, which they can only change once it runs.
    pub fn check_rules_on_start(
        &self,
        pinned: &Pinned,
        stored: &Stored,
    ) -> Result<Vec<Problem>, Vec<Problem>> {
        let snapshot = self.resolve(pinned, stored);
        let pinned_alone = self.resolve(pinned, &Stored::new());
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        for rule in &self.rules {
            let Err(message) = (rule.check)(&snapshot) else {
                continue;
            };
            if (rule.check)(&pinned_alone).is_err() {
                errors.push(Problem::new(rule.key, message));
            } else {
                warnings.push(Problem::new(
                    rule.key,
                    format!("the saved values break a rule: {message}"),
                ));
            }
        }
        if errors.is_empty() {
            Ok(warnings)
        } else {
            Err(errors)
        }
    }

    /// Runs every rule, reporting every failure at once.
    pub fn check_rules(&self, snapshot: &Snapshot) -> Result<(), Vec<Problem>> {
        let problems: Vec<Problem> = self
            .rules
            .iter()
            .filter_map(|rule| {
                (rule.check)(snapshot)
                    .err()
                    .map(|message| Problem::new(rule.key, message))
            })
            .collect();

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("definitions", &self.definitions.keys().collect::<Vec<_>>())
            .field("rules", &self.rules.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::settings::{Key, Secret, Value, checks};

    const NAME: Key<str> = Key::new("instance.name");
    const VERIFY: Key<bool> = Key::new("auth.verify_email");
    const HOST: Key<str> = Key::new("mail.host");
    const PASSWORD: Key<Secret> = Key::new("mail.password");
    const IDLE: Key<Duration> = Key::new("auth.session_idle");

    fn registry() -> Registry {
        Registry::builder()
            .define(Definition::string(NAME, "Neon", "The name.").check(checks::non_blank))
            .define(Definition::bool(VERIFY, false, "Verify sign-ups."))
            .define(Definition::string(HOST, "", "The mail server."))
            .define(Definition::secret(PASSWORD, "The mail password."))
            .define(Definition::duration(
                IDLE,
                Duration::from_secs(3600),
                "Idle sign-out.",
            ))
            .rule(VERIFY.name(), |s| {
                if *s.get(VERIFY) && s.get(HOST).is_empty() {
                    Err("needs mail.host set first".to_owned())
                } else {
                    Ok(())
                }
            })
            .build()
            .expect("valid registry")
    }

    fn raw(env: &[(&str, &str)], yaml: &[(&str, Json)]) -> RawLayers {
        RawLayers {
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            yaml: yaml
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
        }
    }

    #[test]
    fn layers_resolve_env_then_yaml_then_database_then_default() {
        let registry = registry();
        let pinned = registry
            .pin(&raw(
                &[("NEON_SETTINGS__INSTANCE__NAME", "From env")],
                &[
                    ("instance.name", json!("From YAML")),
                    ("mail.host", json!("smtp.example.com")),
                ],
            ))
            .expect("valid layers");
        let (stored, warnings) = registry.parse_stored([
            ("mail.host".to_owned(), json!("db.example.com")),
            ("auth.verify_email".to_owned(), json!(true)),
        ]);
        assert!(warnings.is_empty());

        let snapshot = registry.resolve(&pinned, &stored);

        let source = |key: &str| snapshot.resolved(key).map(|r| r.source);
        assert_eq!(snapshot.get(NAME), "From env");
        assert_eq!(source("instance.name"), Some(Source::Env));
        assert_eq!(snapshot.get(HOST), "smtp.example.com");
        assert_eq!(source("mail.host"), Some(Source::Yaml));
        assert!(*snapshot.get(VERIFY));
        assert_eq!(source("auth.verify_email"), Some(Source::Database));
        assert_eq!(snapshot.get(IDLE), &Duration::from_secs(3600));
        assert_eq!(source("auth.session_idle"), Some(Source::Default));
    }

    #[test]
    fn pin_reports_every_problem_by_where_it_was_written() {
        let problems = registry()
            .pin(&raw(
                &[
                    ("NEON_SETTINGS__INSTANCE__NAME", "  "),
                    ("NEON_SETTINGS__INSTANCE__NMAE", "typo"),
                ],
                &[
                    ("auth.verify_email", json!("yes")),
                    ("auth.unknown", json!(1)),
                ],
            ))
            .expect_err("invalid layers");

        assert_eq!(
            problems,
            [
                Problem::new("NEON_SETTINGS__INSTANCE__NAME", "must not be empty"),
                Problem::new("NEON_SETTINGS__INSTANCE__NMAE", "no such setting"),
                Problem::new("settings.auth.unknown", "no such setting"),
                Problem::new("settings.auth.verify_email", "want true or false"),
            ]
        );
    }

    #[test]
    fn invalid_stored_values_become_warnings() {
        let (stored, warnings) = registry().parse_stored([
            ("instance.name".to_owned(), json!("")),
            ("removed.setting".to_owned(), json!(1)),
            ("mail.host".to_owned(), json!("smtp.example.com")),
        ]);

        assert_eq!(
            stored,
            Stored::from([("mail.host", Value::String("smtp.example.com".into()))])
        );
        assert_eq!(
            warnings,
            [
                Problem::new("instance.name", "saved value ignored: must not be empty"),
                Problem::new("removed.setting", "saved value ignored: no such setting"),
            ]
        );
    }

    #[test]
    fn rules_see_the_whole_snapshot() {
        let registry = registry();
        let verify_only = Stored::from([("auth.verify_email", Value::Bool(true))]);
        let with_host = Stored::from([
            ("auth.verify_email", Value::Bool(true)),
            ("mail.host", Value::String("smtp.example.com".into())),
        ]);

        let refused = registry.check_rules(&registry.resolve(&Pinned::default(), &verify_only));
        let allowed = registry.check_rules(&registry.resolve(&Pinned::default(), &with_host));

        assert_eq!(
            refused,
            Err(vec![Problem::new(
                "auth.verify_email",
                "needs mail.host set first"
            )])
        );
        assert_eq!(allowed, Ok(()));
    }

    #[test]
    fn on_start_only_the_pinned_values_breaking_a_rule_is_an_error() {
        let registry = registry();
        let raw = RawLayers::default().with_env([(
            "NEON_SETTINGS__AUTH__VERIFY_EMAIL".to_owned(),
            "true".to_owned(),
        )]);
        let verify_pinned = registry.pin(&raw).expect("valid layers");
        let host_saved = Stored::from([("mail.host", Value::String("smtp.example.com".into()))]);
        let verify_saved = Stored::from([("auth.verify_email", Value::Bool(true))]);

        // The environment breaks the rule by itself.
        assert_eq!(
            registry.check_rules_on_start(&verify_pinned, &Stored::new()),
            Err(vec![Problem::new(
                "auth.verify_email",
                "needs mail.host set first"
            )])
        );
        // What was saved completes what the environment pins.
        assert_eq!(
            registry.check_rules_on_start(&verify_pinned, &host_saved),
            Ok(Vec::new())
        );
        // What was saved breaks it: a warning, for an admin to fix.
        assert_eq!(
            registry.check_rules_on_start(&Pinned::default(), &verify_saved),
            Ok(vec![Problem::new(
                "auth.verify_email",
                "the saved values break a rule: needs mail.host set first"
            )])
        );
    }

    #[test]
    fn build_refuses_bad_definitions() {
        let build = |defs: Vec<Definition>| {
            defs.into_iter()
                .fold(Registry::builder(), RegistryBuilder::define)
                .build()
                .map(|_| ())
        };
        let name = |key: &'static str| Definition::string(Key::new(key), "x", "A name.");

        assert_eq!(
            build(vec![name("Instance.Name")]),
            Err(RegistryError::InvalidKey("Instance.Name"))
        );
        assert_eq!(
            build(vec![name("instance__name")]),
            Err(RegistryError::InvalidKey("instance__name"))
        );
        assert_eq!(
            build(vec![name("a.b"), name("a.b")]),
            Err(RegistryError::Duplicate("a.b"))
        );
        assert_eq!(
            build(vec![name("a.b"), name("a.b.c")]),
            Err(RegistryError::Overlap("a.b", "a.b.c"))
        );
        assert_eq!(build(vec![name("a.b"), name("a.bc")]), Ok(()));
        assert_eq!(
            build(vec![
                Definition::string(Key::new("a.b"), "", "Empty.").check(checks::non_blank)
            ]),
            Err(RegistryError::InvalidDefault {
                key: "a.b",
                message: "must not be empty".into()
            })
        );
        assert_eq!(
            build(vec![Definition::enumeration(
                Key::new("a.mode"),
                &["on", "off"],
                "auto",
                "A mode."
            )]),
            Err(RegistryError::InvalidDefault {
                key: "a.mode",
                message: "want one of on, off".into()
            })
        );
    }

    #[test]
    fn rules_must_name_a_defined_setting() {
        let result = Registry::builder()
            .define(Definition::string(NAME, "Neon", "The name."))
            .rule("mail.host", |_| Ok(()))
            .build();

        assert_eq!(
            result.map(|_| ()),
            Err(RegistryError::UnknownRuleKey("mail.host"))
        );
    }
}
