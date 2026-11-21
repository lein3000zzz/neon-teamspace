use neon_store::settings::{self, AuditEntry, StoredSetting};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn upsert_replaces_and_delete_removes(pool: PgPool) {
    let admin = Uuid::now_v7();
    settings::upsert(&pool, "mail.host", &json!("a.example.com"), None)
        .await
        .unwrap();
    settings::upsert(&pool, "instance.name", &json!("Acme"), None)
        .await
        .unwrap();
    settings::upsert(&pool, "mail.host", &json!("b.example.com"), Some(admin))
        .await
        .unwrap();

    assert_eq!(
        settings::all(&pool).await.unwrap(),
        [
            StoredSetting {
                key: "instance.name".into(),
                value: json!("Acme"),
            },
            StoredSetting {
                key: "mail.host".into(),
                value: json!("b.example.com"),
            },
        ]
    );
    let updated_by: Option<Uuid> =
        sqlx::query_scalar("SELECT updated_by FROM settings WHERE key = 'mail.host'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(updated_by, Some(admin));

    settings::delete(&pool, "mail.host").await.unwrap();
    let keys: Vec<String> = settings::all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.key)
        .collect();
    assert_eq!(keys, ["instance.name"]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn record_change_appends_to_the_audit(pool: PgPool) {
    let admin = Uuid::now_v7();

    let id = settings::record_change(
        &pool,
        AuditEntry {
            key: "instance.name",
            old_value: &json!("Neon Workspace"),
            new_value: &json!("Acme"),
            changed_by: Some(admin),
        },
    )
    .await
    .unwrap();

    let row: (String, serde_json::Value, serde_json::Value, Option<Uuid>) = sqlx::query_as(
        "SELECT key, old_value, new_value, changed_by FROM settings_audit WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "instance.name".to_owned(),
            json!("Neon Workspace"),
            json!("Acme"),
            Some(admin)
        )
    );
}
