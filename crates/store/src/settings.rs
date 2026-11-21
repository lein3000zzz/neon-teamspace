//! Runtime settings saved from the admin page, and their audit trail.

use serde_json::Value as Json;
use sqlx::PgExecutor;
use uuid::Uuid;

/// A saved setting value, as JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSetting {
    pub key: String,
    pub value: Json,
}

/// Every saved setting, in key order.
pub async fn all(executor: impl PgExecutor<'_>) -> sqlx::Result<Vec<StoredSetting>> {
    sqlx::query_as!(
        StoredSetting,
        "SELECT key, value FROM settings ORDER BY key"
    )
    .fetch_all(executor)
    .await
}

/// Saves a value, replacing any saved before.
pub async fn upsert(
    executor: impl PgExecutor<'_>,
    key: &str,
    value: &Json,
    updated_by: Option<Uuid>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO settings (key, value, updated_by)
         VALUES ($1, $2, $3)
         ON CONFLICT (key) DO UPDATE
         SET value = EXCLUDED.value, updated_at = now(), updated_by = EXCLUDED.updated_by",
        key,
        value,
        updated_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Removes a saved value, so the setting falls back to its default.
pub async fn delete(executor: impl PgExecutor<'_>, key: &str) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM settings WHERE key = $1", key)
        .execute(executor)
        .await?;
    Ok(())
}

/// One change for the audit table. The values are what applied before and after, with
/// secrets already redacted by the caller.
#[derive(Debug, Clone, Copy)]
pub struct AuditEntry<'a> {
    pub key: &'a str,
    pub old_value: &'a Json,
    pub new_value: &'a Json,
    pub changed_by: Option<Uuid>,
}

/// Records a change and returns its id.
pub async fn record_change(
    executor: impl PgExecutor<'_>,
    entry: AuditEntry<'_>,
) -> sqlx::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO settings_audit (id, key, old_value, new_value, changed_by)
         VALUES ($1, $2, $3, $4, $5)",
        id,
        entry.key,
        entry.old_value,
        entry.new_value,
        entry.changed_by,
    )
    .execute(executor)
    .await?;
    Ok(id)
}
