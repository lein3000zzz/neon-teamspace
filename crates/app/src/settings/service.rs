use std::collections::BTreeMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neon_domain::settings::{
    Definition, Kind, Pinned, Problem, Registry, Snapshot, Source, Stored, Value,
};
use neon_store::settings::{self as repo, AuditEntry};
use serde_json::{Value as Json, json};
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::crypto::{Envelope, SealError};

/// The field a sealed secret is saved under: `{"sealed": "<base64>"}`.
const SEALED: &str = "sealed";

/// A change to one setting.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Saves a value, written as JSON of the setting's type.
    Set(Json),
    /// Removes the saved value, so the default applies again.
    Reset,
}

/// One setting as an admin sees it, with secrets redacted.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: &'static str,
    pub kind: Kind,
    pub description: &'static str,
    pub value: Json,
    pub default: Json,
    pub source: Source,
    pub env_var: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("invalid settings: {}", list(.0))]
    Invalid(Vec<Problem>),
    #[error("reading saved settings: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// Nothing was saved. Each problem names the setting to fix.
    #[error("invalid settings: {}", list(.0))]
    Invalid(Vec<Problem>),
    #[error("saving settings: {0}")]
    Database(#[from] sqlx::Error),
}

fn list(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Holds the resolved settings and saves changes made on the admin page.
///
/// Reading is a pointer load, so code reads the settings where it uses them rather than
/// keeping a copy, and a saved change applies at once without a restart.
///
/// Secret settings are saved sealed with the [`Envelope`], each bound to its key, and
/// refused while no secret key is configured, so the database never holds one in the
/// clear. An empty secret is saved as it is: there is nothing to hide.
pub struct SettingsService {
    registry: Registry,
    pinned: Pinned,
    pool: PgPool,
    envelope: Envelope,
    /// The values the database holds. Its lock also serialises writes, so two changes
    /// never start from the same old state.
    stored: Mutex<Stored>,
    current: ArcSwap<Snapshot>,
}

impl SettingsService {
    /// Reads the saved values and resolves every setting. Saved values that no longer
    /// parse, and secrets that do not open, are skipped and returned as warnings; rules
    /// that fail are an error, since the server would run with a combination they refuse.
    pub async fn load(
        pool: PgPool,
        registry: Registry,
        pinned: Pinned,
        envelope: Envelope,
    ) -> Result<(Self, Vec<Problem>), LoadError> {
        let rows = repo::all(&pool).await?;
        Self::from_rows(
            pool,
            registry,
            pinned,
            envelope,
            rows.into_iter().map(|row| (row.key, row.value)),
        )
    }

    /// Like [`load`](Self::load), with the saved rows already read.
    pub fn from_rows(
        pool: PgPool,
        registry: Registry,
        pinned: Pinned,
        envelope: Envelope,
        rows: impl IntoIterator<Item = (String, Json)>,
    ) -> Result<(Self, Vec<Problem>), LoadError> {
        let mut warnings = Vec::new();
        let rows: Vec<(String, Json)> = rows
            .into_iter()
            .filter_map(|(key, json)| {
                let secret = registry
                    .get(&key)
                    .is_some_and(|def| def.kind() == Kind::Secret);
                if !secret {
                    return Some((key, json));
                }
                match open_stored(&envelope, &key, &json) {
                    Ok(plain) => Some((key, plain)),
                    Err(message) => {
                        warnings.push(Problem::new(key, format!("saved value ignored: {message}")));
                        None
                    }
                }
            })
            .collect();
        let (stored, parse_warnings) = registry.parse_stored(rows);
        warnings.extend(parse_warnings);
        let snapshot = registry.resolve(&pinned, &stored);
        registry
            .check_rules(&snapshot)
            .map_err(LoadError::Invalid)?;

        let service = Self {
            registry,
            pinned,
            pool,
            envelope,
            stored: Mutex::new(stored),
            current: ArcSwap::from_pointee(snapshot),
        };
        Ok((service, warnings))
    }

    /// The settings now.
    pub fn current(&self) -> Arc<Snapshot> {
        self.current.load_full()
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Every setting in key order, for display.
    pub fn entries(&self) -> Vec<Entry> {
        let snapshot = self.current.load();
        self.registry
            .definitions()
            .filter_map(|def| {
                let resolved = snapshot.resolved(def.key())?;
                Some(Entry {
                    key: def.key(),
                    kind: def.kind(),
                    description: def.description(),
                    value: resolved.value.to_display_json(),
                    default: def.default_value().to_display_json(),
                    source: resolved.source,
                    env_var: def.env_var(),
                })
            })
            .collect()
    }

    /// Applies changes together: every one is checked, then the rules run against the
    /// settings as they would be, and only then are the values and their audit rows
    /// written in one transaction. Settings pinned by the environment or the YAML file
    /// cannot change here. Returns the new settings.
    pub async fn update(
        &self,
        changes: impl IntoIterator<Item = (String, Change)>,
        actor: Option<Uuid>,
    ) -> Result<Arc<Snapshot>, UpdateError> {
        let changes: BTreeMap<String, Change> = changes.into_iter().collect();
        let mut stored = self.stored.lock().await;
        let current = self.current();
        let mut next = stored.clone();
        let mut changed = Vec::new();
        let mut problems = Vec::new();

        for (key, change) in changes {
            let Some(def) = self.registry.get(&key) else {
                problems.push(Problem::new(key, "no such setting"));
                continue;
            };
            if let Err(message) = writable(def, &current) {
                problems.push(Problem::new(key, message));
                continue;
            }

            let before = next.get(def.key()).cloned();
            match change {
                Change::Set(json) => match def.parse_json(&json) {
                    Ok(value) => {
                        next.insert(def.key(), value);
                    }
                    Err(message) => {
                        problems.push(Problem::new(key, message));
                        continue;
                    }
                },
                Change::Reset => {
                    next.remove(def.key());
                }
            }
            if next.get(def.key()) != before.as_ref() {
                changed.push(def.key());
            }
        }

        if !problems.is_empty() {
            return Err(UpdateError::Invalid(problems));
        }
        if changed.is_empty() {
            return Ok(current);
        }

        let snapshot = self.registry.resolve(&self.pinned, &next);
        self.registry
            .check_rules(&snapshot)
            .map_err(UpdateError::Invalid)?;

        let mut saved = Vec::with_capacity(changed.len());
        for &key in &changed {
            match next.get(key).map(|value| self.stored_json(key, value)) {
                Some(Ok(json)) => saved.push((key, Some(json))),
                Some(Err(message)) => problems.push(Problem::new(key, message)),
                None => saved.push((key, None)),
            }
        }
        if !problems.is_empty() {
            return Err(UpdateError::Invalid(problems));
        }

        let mut tx = self.pool.begin().await?;
        for (key, json) in saved {
            match json {
                Some(json) => repo::upsert(&mut *tx, key, &json, actor).await?,
                None => repo::delete(&mut *tx, key).await?,
            }
            let entry = AuditEntry {
                key,
                old_value: &display(&current, key),
                new_value: &display(&snapshot, key),
                changed_by: actor,
            };
            repo::record_change(&mut *tx, entry).await?;
        }
        tx.commit().await?;

        *stored = next;
        let snapshot = Arc::new(snapshot);
        self.current.store(Arc::clone(&snapshot));
        tracing::info!(keys = ?changed, actor = ?actor, "settings changed");
        Ok(snapshot)
    }
}

impl SettingsService {
    /// The JSON a value is saved as: sealed for a secret that is not empty.
    fn stored_json(&self, key: &str, value: &Value) -> Result<Json, String> {
        let Value::Secret(secret) = value else {
            return Ok(value.to_stored_json());
        };
        if secret.expose().is_empty() {
            return Ok(Json::String(String::new()));
        }
        let sealed = self
            .envelope
            .seal(secret.expose().as_bytes(), &secret_aad(key))
            .map_err(|error| match error {
                SealError::NoKey => {
                    "cannot be saved: secret settings are kept encrypted, and no secret key \
                     is configured; set secrets.keys (NEON_SECRETS__KEYS) first"
                        .to_owned()
                }
                SealError::Encrypt => "cannot be saved: encrypting it failed".to_owned(),
            })?;
        Ok(json!({ SEALED: STANDARD.encode(sealed) }))
    }
}

/// Binds a sealed secret to its setting, so it does not open as another's.
fn secret_aad(key: &str) -> Vec<u8> {
    format!("setting:{key}").into_bytes()
}

/// The plain JSON string of a saved secret, or why it cannot be read. The messages never
/// hold the value.
fn open_stored(envelope: &Envelope, key: &str, json: &Json) -> Result<Json, String> {
    match json {
        Json::String(text) if text.is_empty() => Ok(json.clone()),
        Json::Object(fields) => {
            let sealed = fields
                .get(SEALED)
                .and_then(Json::as_str)
                .and_then(|text| STANDARD.decode(text).ok())
                .ok_or_else(|| "it is not a sealed secret".to_owned())?;
            let plain = envelope
                .open(&sealed, &secret_aad(key))
                .map_err(|error| error.to_string())?;
            String::from_utf8(plain.to_vec())
                .map(Json::String)
                .map_err(|_| "it is not text".to_owned())
        }
        _ => Err("a secret saved without encryption is not used".to_owned()),
    }
}

fn writable(def: &Definition, current: &Snapshot) -> Result<(), String> {
    match current.resolved(def.key()).map(|resolved| resolved.source) {
        Some(Source::Env) => Err(format!(
            "set by the environment variable {}; change it there",
            def.env_var()
        )),
        Some(Source::Yaml) => Err(format!(
            "set in the config file as {}; change it there",
            def.yaml_path()
        )),
        _ => Ok(()),
    }
}

fn display(snapshot: &Snapshot, key: &str) -> Json {
    snapshot
        .resolved(key)
        .map_or(Json::Null, |resolved| resolved.value.to_display_json())
}
