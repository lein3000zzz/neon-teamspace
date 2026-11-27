//! A page's history over HTTP: its versions listed, read with their blocks and markdown,
//! compared with the page as it is and restored on the revision compared, refused once the
//! page changed, and kept from those who may not see or edit it.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use super::agents_tests::{Bearer, agent, agent_token};
use super::pages_tests::{general, guest};
use super::tests::state;
use super::workspaces_tests::{id, team};
use super::*;

/// The markdown without the lines naming blocks.
fn text(markdown: &Value) -> String {
    markdown
        .as_str()
        .expect("markdown")
        .lines()
        .filter(|line| !line.trim().starts_with("<!-- block "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn versions_are_listed_read_compared_and_restored(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let created = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({"teamspace_id": general, "title": "Plan", "markdown": "# Goals\n\n- Ship it"}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let page = id(&created);
    let versions = format!("/api/v1/pages/{page}/versions");

    let listed = t.dee.get(&versions).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(listed.body["page"]["id"], page.as_str());
    let first = &listed.body["versions"][0];
    assert_eq!(first["added"], 2);
    assert_eq!(first["authors"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed.body["next"], Value::Null);
    let version = first["id"].as_str().expect("an id").to_owned();

    let read = t.dee.get(&format!("{versions}/{version}")).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_eq!(text(&read.body["markdown"]), "# Goals\n\n- Ship it");
    assert_eq!(read.body["blocks"][0]["type"], "heading");
    assert_eq!(read.body["blocks"][1]["type"], "bulletListItem");

    let markdown = format!("/api/v1/pages/{page}/markdown");
    let appended = t
        .dee
        .send(
            Method::PATCH,
            &markdown,
            Some(json!({"operations": [{"op": "append", "markdown": "Later"}]})),
        )
        .await;
    assert_eq!(appended.status, StatusCode::OK, "{}", appended.body);

    let compared = t.dee.get(&format!("{versions}/{version}/compare")).await;
    assert_eq!(compared.status, StatusCode::OK, "{}", compared.body);
    let changes: Vec<_> = compared.body["changes"]
        .as_array()
        .expect("changes")
        .iter()
        .map(|change| {
            (
                change["change"].as_str().expect("a change"),
                change["kind"].as_str().expect("a kind"),
                change["before"].as_str(),
                change["after"].as_str(),
                change["after_text"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        changes,
        [
            ("same", "heading", None, Some("# Goals\n"), Some("Goals")),
            (
                "same",
                "bulletListItem",
                None,
                Some("- Ship it\n"),
                Some("Ship it")
            ),
            ("added", "paragraph", None, Some("Later\n"), Some("Later")),
        ]
    );
    assert_eq!(
        (
            &compared.body["added"],
            &compared.body["changed"],
            &compared.body["removed"]
        ),
        (&json!(1), &json!(0), &json!(0))
    );

    let restore = format!("{versions}/{version}/restore");
    let restored = t
        .dee
        .post(&restore, json!({"base": compared.body["revision"]}))
        .await;
    assert_eq!(restored.status, StatusCode::OK, "{}", restored.body);
    assert_eq!(restored.body["version"]["id"], version.as_str());
    assert!(restored.body["revision"].is_string());
    let now = t.ada.get(&markdown).await;
    assert_eq!(text(&now.body["markdown"]), "# Goals\n\n- Ship it");
    assert_eq!(now.body["revision"], restored.body["revision"]);

    // The same comparison again is stale: the page changed since.
    let stale = t
        .dee
        .post(&restore, json!({"base": compared.body["revision"]}))
        .await;
    assert_eq!(stale.status, StatusCode::CONFLICT, "{}", stale.body);
    assert_eq!(stale.body["error"]["code"], "edit_conflict");

    // Two versions now: the page before the restore, and the restore itself.
    let listed = t.ada.get(&format!("{versions}?limit=1")).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(
        listed.body["versions"][0]["restored_from"],
        version.as_str()
    );
    let next = listed.body["next"].as_str().expect("a next part");
    let rest = t
        .ada
        .get(&format!("{versions}?limit=10&after={next}"))
        .await;
    assert_eq!(rest.body["versions"].as_array().map(Vec::len), Some(2));
    state.sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn history_keeps_to_what_the_caller_may_see_and_do(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let created = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({"teamspace_id": general, "title": "Plan", "markdown": "First"}),
        )
        .await;
    let page = id(&created);
    let versions = format!("/api/v1/pages/{page}/versions");
    let version = t.ada.get(&versions).await.body["versions"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let base = t
        .ada
        .get(&format!("{versions}/{version}/compare"))
        .await
        .body["revision"]
        .clone();

    // A guest sees nothing until the page is shared with them; shared to view, sees the page
    // and not its history, and restores nothing; shared to edit, sees it without authors.
    super::auth_tests::user(&state, "gus@example.com").await;
    let gus = guest(&app, &t.ada, ws, "gus@example.com").await;
    let hidden = gus.get(&versions).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{}", hidden.body);
    let gus_id = gus.get("/api/v1/account/me").await.body["id"].clone();
    let shared = t
        .ada
        .post(
            &format!("/api/v1/pages/{page}/shares"),
            json!({"user_id": gus_id, "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    let page_seen = gus.get(&format!("/api/v1/pages/{page}")).await;
    assert_eq!(page_seen.status, StatusCode::OK, "{}", page_seen.body);
    for path in [
        versions.clone(),
        format!("{versions}/{version}"),
        format!("{versions}/{version}/compare"),
    ] {
        let refused = gus.get(&path).await;
        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "{path}: {}",
            refused.body
        );
        assert_eq!(refused.body["error"]["code"], "forbidden");
    }
    let refused = gus
        .post(
            &format!("{versions}/{version}/restore"),
            json!({"base": base}),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    let gus_id = gus_id.as_str().expect("an id");
    let reshared = t
        .ada
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{page}/shares/{gus_id}"),
            Some(json!({"access": "can_edit"})),
        )
        .await;
    assert_eq!(reshared.status, StatusCode::OK, "{}", reshared.body);
    let seen = gus.get(&versions).await;
    assert_eq!(seen.status, StatusCode::OK, "{}", seen.body);
    assert_eq!(seen.body["versions"][0]["authors"], json!([]));

    // A token that only reads, of an agent that edits the page as its owner does, lists and
    // compares, and restores nothing.
    let scout = agent(&t.ada, ws, "Scout").await;
    let reader = Bearer::new(&app, agent_token(&t.ada, ws, &scout, &["read"]).await);
    let listed = reader.send(Method::GET, &versions, None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let refused = reader
        .send(
            Method::POST,
            &format!("{versions}/{version}/restore"),
            Some(json!({"base": base})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert_eq!(refused.body["error"]["code"], "insufficient_scope");

    // A version of no page of this one, and a database, which has no history.
    let missing = t
        .ada
        .get(&format!("{versions}/0193a5b2-0000-7000-8000-0000000000f1"))
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.body);
    let database = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/databases"),
            json!({"teamspace_id": general, "title": "Tasks"}),
        )
        .await;
    assert_eq!(database.status, StatusCode::CREATED, "{}", database.body);
    let database = database.body["page"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let refused = t
        .ada
        .get(&format!("/api/v1/pages/{database}/versions"))
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    let bad = t.ada.get(&format!("{versions}?after=nonsense")).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
    state.sync.shutdown().await;
}
