use std::collections::BTreeMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use neon_domain::settings::{
    Definition, Kind, Pinned, Problem, Registry, Snapshot, Source, Stored,
};
use neon_store::settings::{self as repo, AuditEntry};
use serde_json::Value as Json;
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

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
pub struct SettingsService {
    registry: Registry,
    pinned: Pinned,
    pool: PgPool,
    /// The values the database holds. Its lock also serialises writes, so two changes
    /// never start from the same old state.
    stored: Mutex<Stored>,
    current: ArcSwap<Snapshot>,
}

impl SettingsService {
    /// Reads the saved values and resolves every setting. Saved values that no longer
    /// parse are skipped and returned as warnings; rules that fail are an error, since
    /// the server would run with a combination they refuse.
    pub async fn load(
        pool: PgPool,
        registry: Registry,
        pinned: Pinned,
    ) -> Result<(Self, Vec<Problem>), LoadError> {
        let rows = repo::all(&pool).await?;
        Self::from_rows(
            pool,
            registry,
            pinned,
            rows.into_iter().map(|row| (row.key, row.value)),
        )
    }

    /// Like [`load`](Self::load), with the saved rows already read.
    pub fn from_rows(
        pool: PgPool,
        registry: Registry,
        pinned: Pinned,
        rows: impl IntoIterator<Item = (String, Json)>,
    ) -> Result<(Self, Vec<Problem>), LoadError> {
        let (stored, warnings) = registry.parse_stored(rows);
        let snapshot = registry.resolve(&pinned, &stored);
        registry
            .check_rules(&snapshot)
            .map_err(LoadError::Invalid)?;

        let service = Self {
            registry,
            pinned,
            pool,
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

        let mut tx = self.pool.begin().await?;
        for &key in &changed {
            match next.get(key) {
                Some(value) => repo::upsert(&mut *tx, key, &value.to_stored_json(), actor).await?,
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
