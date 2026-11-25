//! Pages' markdown with block ids and its edits, claims of rows, the tasks assigned to
//! someone, the pages changed last, and whom a request acts as, over HTTP.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use super::agents_tests::{Bearer, agent, agent_token};
use super::auth_tests::{Answer, user};
use super::databases_tests::{database, database_id, option, property};
use super::pages_tests::{general, guest};
use super::tests::state;
use super::workspaces_tests::{id, team};
use super::*;

/// The ids the lines naming blocks give, in order.
fn block_ids(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("<!-- block ")
                .and_then(|rest| rest.strip_suffix(" -->"))
                .map(str::to_owned)
        })
        .collect()
}

/// The markdown without the lines naming blocks.
fn text(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| !line.trim().starts_with("<!-- block "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn markdown_of(answer: &Answer) -> &str {
    answer.body["markdown"]
        .as_str()
        .unwrap_or_else(|| panic!("no markdown: {}", answer.body))
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn markdown_names_each_block_and_edits_name_them_back(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;

    let created = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({
                "teamspace_id": general,
                "title": "Plan",
                "markdown": "# Goals\n\nShip it\n\n- one\n- two",
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let page = id(&created);
    let markdown = format!("/api/v1/pages/{page}/markdown");

    let read = t.dee.get(&markdown).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_eq!(read.body["page"]["title"], "Plan");
    assert_eq!(read.body.get("properties"), None);
    let ids = block_ids(markdown_of(&read));
    assert_eq!(ids.len(), 4, "{}", markdown_of(&read));
    assert_eq!(
        text(markdown_of(&read)),
        "# Goals\n\nShip it\n\n- one\n- two"
    );

    let edited = t
        .dee
        .send(
            Method::PATCH,
            &markdown,
            Some(json!({"operations": [
                {"op": "insert_after", "block_id": ids[1], "markdown": "Then *more*"},
                {"op": "find_replace", "find": "Ship it", "replace": "Ship it soon"},
                {"op": "replace_range", "from": ids[2], "to": ids[3], "markdown": "- uno\n- dos"},
                {"op": "append", "markdown": "The end"},
            ]})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    let results = edited.body["results"].as_array().expect("results");
    assert_eq!(results.len(), 4);
    assert_eq!(results[0]["added"].as_array().map(Vec::len), Some(1));
    assert_eq!(results[1]["replaced"], 1);
    assert_eq!(results[2]["removed"], 2);
    let read = t.ada.get(&markdown).await;
    assert_eq!(
        text(markdown_of(&read)),
        "# Goals\n\nShip it soon\n\nThen *more*\n\n- uno\n- dos\n\nThe end"
    );

    // The whole body, sent back as read, changes nothing; with a block left out, that
    // block alone goes.
    let ids = block_ids(markdown_of(&read));
    let same = t
        .ada
        .send(
            Method::PATCH,
            &markdown,
            Some(json!({"operations": [
                {"op": "replace_body", "markdown": markdown_of(&read)},
            ]})),
        )
        .await;
    assert_eq!(same.status, StatusCode::OK, "{}", same.body);
    assert_eq!(same.body["results"][0]["removed"], 0);
    assert_eq!(same.body["results"][0]["added"], json!([]));
    let shorter = t
        .ada
        .send(
            Method::PATCH,
            &markdown,
            Some(json!({"operations": [
                {"op": "replace_body", "markdown": "# Goals\n\nShip it soon\n\nThe end"},
            ]})),
        )
        .await;
    assert_eq!(shorter.body["results"][0]["removed"], 3);
    let read = t.ada.get(&markdown).await;
    let kept = block_ids(markdown_of(&read));
    assert_eq!(kept, [ids[0].clone(), ids[1].clone(), ids[5].clone()]);

    // A refused operation names itself, and nothing changes.
    let refused = t
        .ada
        .send(
            Method::PATCH,
            &markdown,
            Some(json!({"operations": [
                {"op": "append", "markdown": "Lost"},
                {"op": "insert_after", "block_id": "nope", "markdown": "x"},
            ]})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert_eq!(refused.code(), "edit_refused");
    assert_eq!(
        refused.body["error"]["message"],
        "edit 1: the page has no block nope; nothing was changed"
    );
    assert_eq!(
        t.ada
            .send(Method::PATCH, &markdown, Some(json!({"operations": []})))
            .await
            .code(),
        "invalid_input"
    );
    assert_eq!(
        t.ada
            .send(
                Method::PATCH,
                &markdown,
                Some(json!({"operations": [{"op": "shout"}]}))
            )
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(!markdown_of(&t.ada.get(&markdown).await).contains("Lost"));

    // A guest it is shared with to view reads it, and edits nothing; a token with the
    // read scope alone neither.
    user(&state, "gus@example.com").await;
    let gus = guest(&app, &t.ada, ws, "gus@example.com").await;
    let hidden = gus.get(&markdown).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    let shared = t
        .ada
        .post(
            &format!("/api/v1/pages/{page}/shares"),
            json!({"user_id": gus.get("/api/v1/account/me").await.body["id"], "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    assert_eq!(gus.get(&markdown).await.status, StatusCode::OK);
    let refused = gus
        .send(
            Method::PATCH,
            &markdown,
            Some(json!({"operations": [{"op": "append", "markdown": "x"}]})),
        )
        .await;
    assert_eq!(refused.code(), "forbidden");
    let reader = Bearer::new(&app, t.dee.token(&["read"]).await);
    assert_eq!(reader.get(&markdown).await.status, StatusCode::OK);
    let refused = reader
        .send(
            Method::PATCH,
            &markdown,
            Some(json!({"operations": [{"op": "append", "markdown": "x"}]})),
        )
        .await;
    assert_eq!(refused.code(), "insufficient_scope");

    // A database has no body of its own.
    let db = database_id(&database(&t.ada, ws).await);
    assert_eq!(
        t.ada
            .get(&format!("/api/v1/pages/{db}/markdown"))
            .await
            .code(),
        "invalid_input"
    );
    state.sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn rows_are_read_with_their_properties_claimed_and_changed_by_unique_id(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let mut db = database(&t.ada, ws).await;
    let db_id = database_id(&db);
    for property in [
        json!({"name": "Assignee", "type": "person"}),
        json!({"name": "Key", "type": "unique_id", "prefix": "NW"}),
    ] {
        let added = t
            .ada
            .post(&format!("/api/v1/databases/{db_id}/properties"), property)
            .await;
        assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    }
    db = t.ada.get(&format!("/api/v1/databases/{db_id}")).await.body;
    let row = t
        .ada
        .post(
            &format!("/api/v1/databases/{db_id}/rows"),
            json!({"values": {"Name": "Fix login"}}),
        )
        .await;
    assert_eq!(row.status, StatusCode::CREATED, "{}", row.body);
    let row_id = id(&row);
    assert_eq!(row.body["unique_id"], "NW-1");

    let read = t.dee.get(&format!("/api/v1/pages/{row_id}/markdown")).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    let properties = read.body["properties"].as_array().expect("properties");
    let named = |name: &str| {
        properties
            .iter()
            .find(|property| property["name"] == name)
            .unwrap_or_else(|| panic!("no {name}: {properties:?}"))
            .clone()
    };
    assert_eq!(named("Name")["text"], "Fix login");
    assert_eq!(named("Key")["text"], "NW-1");
    assert_eq!(named("Status")["value"], Value::Null);

    // Dee claims it by its unique id; Cy finds it held, by her.
    let claimed = t
        .dee
        .post(
            &format!("/api/v1/databases/{db_id}/rows/nw-1/claim"),
            json!({}),
        )
        .await;
    assert_eq!(claimed.status, StatusCode::OK, "{}", claimed.body);
    let dee_id = t.dee.get("/api/v1/account/me").await.body["id"].clone();
    assert_eq!(
        claimed.body["values"][property(&db, "Assignee")],
        json!([dee_id])
    );
    assert_eq!(
        claimed.body["values"][property(&db, "Status")],
        json!(option(&db, "Status", "In progress"))
    );
    let held =
        t.cy.post(&format!("/api/v1/rows/{row_id}/claim"), json!({}))
            .await;
    assert_eq!(held.status, StatusCode::CONFLICT, "{}", held.body);
    assert_eq!(held.code(), "already_claimed");
    let message = held.body["error"]["message"].as_str().expect("a message");
    assert!(
        message.contains(dee_id.as_str().expect("an id")),
        "{message}"
    );
    let read = t.cy.get(&format!("/api/v1/pages/{row_id}/markdown")).await;
    let assignee = read.body["properties"]
        .as_array()
        .expect("properties")
        .iter()
        .find(|property| property["name"] == "Assignee")
        .cloned()
        .expect("an assignee");
    assert_eq!(assignee["text"], "dee");
    let named =
        t.cy.post(
            &format!("/api/v1/rows/{row_id}/claim?person_property=Status"),
            json!({}),
        )
        .await;
    assert_eq!(named.code(), "invalid_input");

    // By unique id, its values change too.
    let changed =
        t.cy.send(
            Method::PATCH,
            &format!("/api/v1/databases/{db_id}/rows/NW-1"),
            Some(json!({"values": {"Status": "Done"}})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(
        changed.body["values"][property(&db, "Status")],
        json!(option(&db, "Status", "Done"))
    );
    assert_eq!(
        t.cy.send(
            Method::PATCH,
            &format!("/api/v1/databases/{db_id}/rows/NW-9"),
            Some(json!({"values": {"Status": "Done"}})),
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    // Dee's tasks hold it, with its status.
    let tasks = t.dee.get(&format!("/api/v1/workspaces/{ws}/tasks")).await;
    assert_eq!(tasks.status, StatusCode::OK, "{}", tasks.body);
    assert_eq!(tasks.body["tasks"][0]["row"]["id"], row_id.as_str());
    assert_eq!(tasks.body["tasks"][0]["database_title"], "Tasks");
    assert_eq!(tasks.body["tasks"][0]["status"]["name"], "Done");
    assert_eq!(tasks.body["tasks"][0]["status"]["group"], "done");
    let none = t.cy.get(&format!("/api/v1/workspaces/{ws}/tasks")).await;
    assert_eq!(none.body["tasks"], json!([]));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn recent_pages_and_whom_a_request_acts_as(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    for title in ["First", "Second"] {
        let made = t
            .ada
            .post(
                &format!("/api/v1/workspaces/{ws}/pages"),
                json!({"teamspace_id": general, "title": title}),
            )
            .await;
        assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    }
    let recent = t
        .dee
        .get(&format!("/api/v1/workspaces/{ws}/recent-pages?limit=1"))
        .await;
    assert_eq!(recent.status, StatusCode::OK, "{}", recent.body);
    assert_eq!(recent.body["pages"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        t.dee
            .get(&format!("/api/v1/workspaces/{ws}/recent-pages?limit=0"))
            .await
            .code(),
        "invalid_input"
    );

    // A session acts as its person, everywhere.
    let me = t.dee.get("/api/v1/whoami").await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert_eq!(me.body["identity"]["kind"], "human");
    assert_eq!(me.body["identity"]["display_name"], "dee");
    assert_eq!(me.body["credential"], "session");
    assert_eq!(me.body["workspace"], Value::Null);

    // A token bound to the workspace, as a member; an agent's, as the agent of its owner.
    let made = t
        .ada
        .post(
            "/api/v1/account/tokens",
            json!({"name": "bound", "scopes": ["read"], "workspace_id": ws}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let bound = Bearer::new(
        &app,
        format!("Bearer {}", made.body["token"].as_str().expect("a token")),
    );
    let me = bound.get("/api/v1/whoami").await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert_eq!(me.body["credential"], "token");
    assert_eq!(me.body["scopes"], json!(["read"]));
    assert_eq!(me.body["workspace"]["id"], ws.as_str());
    assert_eq!(me.body["workspace"]["name"], "Design");
    assert_eq!(me.body["workspace"]["role"], "member");

    let scout = agent(&t.cy, ws, "Scout").await;
    let token = agent_token(&t.cy, ws, &scout, &["read", "write"]).await;
    let me = Bearer::new(&app, token).get("/api/v1/whoami").await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert_eq!(me.body["identity"]["kind"], "agent");
    assert_eq!(me.body["identity"]["display_name"], "Scout");
    assert_eq!(me.body["identity"]["owner"]["display_name"], "cy");
    assert_eq!(me.body["workspace"]["role"], "member");
    assert_eq!(me.body["scopes"], json!(["read", "write"]));
}
