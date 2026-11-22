//! Databases over HTTP and WebSocket: the routes, their answers and error codes, the
//! scopes they need and the origin check, a board queried with a filter and a sort, and
//! the feed of changes that every client following a database hears.

use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::{self, Message as Frame};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::auth_tests::{Answer, call, user};
use super::pages_tests::{general, guest};
use super::sync_tests::Server;
use super::tests::state;
use super::workspaces_tests::{Browser, id, team};
use super::*;

/// Long enough for any step here; reaching it means something never came.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// Makes a database in General, as `by`.
async fn database(by: &Browser<'_>, workspace: &str) -> Value {
    let general = general(by, workspace).await;
    let made = by
        .post(
            &format!("/api/v1/workspaces/{workspace}/databases"),
            json!({"teamspace_id": general, "title": "Tasks"}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    made.body
}

fn database_id(database: &Value) -> String {
    database["page"]["id"].as_str().expect("an id").to_owned()
}

/// The id of the property of `database` named `name`.
fn property(database: &Value, name: &str) -> String {
    database["properties"]
        .as_array()
        .expect("properties")
        .iter()
        .find(|property| property["name"] == name)
        .and_then(|property| property["id"].as_str())
        .unwrap_or_else(|| panic!("no property {name}: {database}"))
        .to_owned()
}

/// The id of the option of `property` named `name`.
fn option(database: &Value, property: &str, name: &str) -> String {
    database["properties"]
        .as_array()
        .expect("properties")
        .iter()
        .find(|p| p["name"] == property)
        .and_then(|p| p["options"].as_array())
        .and_then(|options| options.iter().find(|option| option["name"] == name))
        .and_then(|option| option["id"].as_str())
        .unwrap_or_else(|| panic!("no option {name} of {property}"))
        .to_owned()
}

fn titles(answer: &Answer) -> Vec<&str> {
    answer.body["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("no rows: {}", answer.body))
        .iter()
        .map(|row| row["title"].as_str().expect("a title"))
        .collect()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn databases_their_properties_and_views_are_made_and_changed(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let made = database(&t.ada, ws).await;
    assert_eq!(made["page"]["kind"], "database");
    assert_eq!(made["page"]["title"], "Tasks");
    assert_eq!(made["inline"], false);
    assert_eq!(made["properties"][0]["type"], "title");
    assert_eq!(made["properties"][1]["type"], "status");
    assert_eq!(made["properties"][1]["options"][0]["name"], "Not started");
    assert_eq!(made["properties"][1]["options"][0]["group"], "to_do");
    assert_eq!(made["views"][0]["layout"], "table");
    assert_eq!(made["views"][1]["layout"], "board");
    assert_eq!(
        made["views"][1]["group"]["property"],
        property(&made, "Status")
    );
    let db = database_id(&made);
    let base = format!("/api/v1/databases/{db}");

    let read = t.dee.get(&base).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_eq!(read.body["page"]["access"], "edit");
    // The database is in its teamspace's tree.
    let general = general(&t.ada, ws).await;
    let tree = t
        .dee
        .get(&format!("/api/v1/teamspaces/{general}/pages"))
        .await;
    assert_eq!(tree.body["pages"][0]["kind"], "database");

    for body in [
        json!({"name": "Notes", "type": "text"}),
        json!({"name": "Budget", "type": "number", "number_format": "currency", "currency": "EUR"}),
        json!({"name": "Type", "type": "select", "options": [{"name": "Bug", "hue": 25}, {"name": "Feature"}]}),
        json!({"name": "Tags", "type": "multi_select", "options": [{"name": "UI"}]}),
        json!({"name": "Due", "type": "date"}),
        json!({"name": "Owner", "type": "person"}),
        json!({"name": "Shipped", "type": "checkbox"}),
        json!({"name": "Link", "type": "url"}),
        json!({"name": "ID", "type": "unique_id", "prefix": "nw"}),
        json!({"name": "Created", "type": "created_time"}),
        json!({"name": "Creator", "type": "created_by"}),
        json!({"name": "Edited", "type": "last_edited_time"}),
        json!({"name": "Editor", "type": "last_edited_by"}),
    ] {
        let added = t
            .ada
            .post(&format!("{base}/properties"), body.clone())
            .await;
        assert_eq!(added.status, StatusCode::CREATED, "{body}: {}", added.body);
        assert_eq!(added.body["name"], body["name"]);
    }
    let read = t.ada.get(&base).await.body;
    let budget = property(&read, "Budget");
    let id_property = read["properties"]
        .as_array()
        .expect("properties")
        .iter()
        .find(|p| p["name"] == "ID")
        .cloned()
        .expect("an id");
    assert_eq!(id_property["prefix"], "NW");
    assert_eq!(id_property.get("options"), None);

    for (body, status, code) in [
        (
            json!({"name": "Status", "type": "text"}),
            StatusCode::CONFLICT,
            "name_taken",
        ),
        (
            json!({"name": "Title 2", "type": "title"}),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (
            json!({"name": "P", "type": "text", "currency": "EUR"}),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (
            json!({"name": "P", "type": "nope"}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_json",
        ),
    ] {
        let refused = t
            .ada
            .post(&format!("{base}/properties"), body.clone())
            .await;
        assert_eq!(refused.status, status, "{body}: {}", refused.body);
        assert_eq!(refused.code(), code);
    }

    // Changing a property: its name, its format, its type.
    let renamed = t
        .ada
        .send(
            Method::PATCH,
            &format!("{base}/properties/{budget}"),
            Some(json!({"name": "Cost", "type": "number", "number_format": "percent"})),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    assert_eq!(renamed.body["name"], "Cost");
    assert_eq!(renamed.body["number_format"], "percent");
    let notes = property(&read, "Notes");
    let row = t
        .ada
        .post(
            &format!("{base}/rows"),
            json!({"values": {"Name": "Row", "Notes": "not a number"}}),
        )
        .await;
    assert_eq!(row.status, StatusCode::CREATED, "{}", row.body);
    let lossy = t
        .ada
        .send(
            Method::PATCH,
            &format!("{base}/properties/{notes}"),
            Some(json!({"type": "number"})),
        )
        .await;
    assert_eq!(lossy.status, StatusCode::CONFLICT, "{}", lossy.body);
    assert_eq!(lossy.code(), "lossy_change");
    let settings_alone = t
        .ada
        .send(
            Method::PATCH,
            &format!("{base}/properties/{notes}"),
            Some(json!({"number_format": "plain"})),
        )
        .await;
    assert_eq!(settings_alone.code(), "invalid_input");
    let confirmed = t
        .ada
        .send(
            Method::PATCH,
            &format!("{base}/properties/{notes}"),
            Some(json!({"type": "number", "confirm": true})),
        )
        .await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.body);
    let moved = t
        .ada
        .post(
            &format!("{base}/properties/{notes}/move"),
            json!({"placement": "first"}),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    let deleted = t
        .ada
        .send(Method::DELETE, &format!("{base}/properties/{notes}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    let title = property(&read, "Name");
    let kept = t
        .ada
        .send(Method::DELETE, &format!("{base}/properties/{title}"), None)
        .await;
    assert_eq!(kept.code(), "invalid_input");

    // Views.
    let view = t
        .ada
        .post(
            &format!("{base}/views"),
            json!({
                "name": "Open bugs",
                "layout": "list",
                "filter": {"and": [
                    {"property": "Type", "operator": "is", "value": "Bug"},
                    {"property": "Status", "operator": "is_not", "value": "Done"}
                ]},
                "sorts": [{"property": "Due", "direction": "ascending"}],
                "properties": [{"property": "Due", "visible": true, "width": 160}]
            }),
        )
        .await;
    assert_eq!(view.status, StatusCode::CREATED, "{}", view.body);
    assert_eq!(
        view.body["filter"]["and"][0]["value"],
        option(&read, "Type", "Bug")
    );
    assert_eq!(view.body["properties"][0]["width"], 160);
    let view_id = id(&view);
    let changed = t
        .ada
        .send(
            Method::PATCH,
            &format!("{base}/views/{view_id}"),
            Some(json!({"filter": null, "name": "Everything"})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.body["filter"], Value::Null);
    assert_eq!(changed.body["sorts"][0]["direction"], "ascending");
    let copied = t
        .ada
        .post(&format!("{base}/views/{view_id}/duplicate"), json!({}))
        .await;
    assert_eq!(copied.status, StatusCode::CREATED, "{}", copied.body);
    assert_eq!(copied.body["name"], "Everything (copy)");
    let moved = t
        .ada
        .post(
            &format!("{base}/views/{view_id}/move"),
            json!({"placement": "first"}),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    let views = t.ada.get(&base).await.body["views"].clone();
    assert_eq!(views[0]["name"], "Everything");
    let bad_board = t
        .ada
        .post(
            &format!("{base}/views"),
            json!({"name": "B", "layout": "board"}),
        )
        .await;
    assert_eq!(bad_board.code(), "invalid_input");
    let removed = t
        .ada
        .send(Method::DELETE, &format!("{base}/views/{view_id}"), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);

    // Nothing goes under a database but its rows.
    let under = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({"parent_id": db, "title": "Sub"}),
        )
        .await;
    assert_eq!(under.code(), "invalid_input");
    // A page that is no database is not found as one.
    let page = super::pages_tests::create(&t.ada, ws, json!({"teamspace_id": general}), "P").await;
    let not_one = t.ada.get(&format!("/api/v1/databases/{page}")).await;
    assert_eq!(not_one.status, StatusCode::NOT_FOUND, "{}", not_one.body);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_board_is_queried_with_a_filter_and_a_sort_and_cards_move_between_columns(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let made = database(&t.ada, &t.workspace).await;
    let db = database_id(&made);
    let base = format!("/api/v1/databases/{db}");
    for body in [
        json!({"name": "Points", "type": "number"}),
        json!({"name": "ID", "type": "unique_id", "prefix": "NW"}),
    ] {
        let added = t.ada.post(&format!("{base}/properties"), body).await;
        assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    }
    let made = t.ada.get(&base).await.body;
    let mut rows = Vec::new();
    for (title, status, points) in [
        ("Login", "Done", 3),
        ("Search", "In progress", 8),
        ("Export", "Not started", 5),
        ("Import", "Not started", 1),
    ] {
        let row = t
            .dee
            .post(
                &format!("{base}/rows"),
                json!({"values": {"Name": title, "Status": status, "Points": points}}),
            )
            .await;
        assert_eq!(row.status, StatusCode::CREATED, "{}", row.body);
        rows.push(row.body);
    }
    assert_eq!(rows[0]["unique_id"], "NW-1");
    assert_eq!(rows[3]["number"], 4);
    assert_eq!(rows[0]["access"], "edit");
    let board = made["views"][1]["id"].as_str().expect("a board").to_owned();
    let query = |body: Value| {
        let base = base.clone();
        let dee = &t.dee;
        async move { dee.post(&format!("{base}/query"), body).await }
    };
    let answer = query(json!({
        "view_id": board,
        "filter": {"property": "Points", "operator": "greater_than", "value": 2},
        "sorts": [{"property": "Points", "direction": "descending"}],
    }))
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(titles(&answer), ["Export", "Search", "Login"]);
    let groups: Vec<(Value, i64)> = answer.body["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .map(|group| {
            (
                group["key"].clone(),
                group["count"].as_i64().expect("a count"),
            )
        })
        .collect();
    let (todo, doing, done) = (
        option(&made, "Status", "Not started"),
        option(&made, "Status", "In progress"),
        option(&made, "Status", "Done"),
    );
    assert_eq!(
        groups,
        [
            (Value::Null, 0),
            (json!(todo), 1),
            (json!(doing), 1),
            (json!(done), 1)
        ]
    );
    assert_eq!(answer.body["rows"][0]["group"], todo.as_str());
    assert_eq!(answer.body["total"], 3);
    assert_eq!(answer.body["next"], Value::Null);
    let paged = query(json!({"limit": 3})).await;
    assert_eq!(titles(&paged), ["Login", "Search", "Export"]);
    let rest = query(json!({"limit": 3, "after": paged.body["next"]})).await;
    assert_eq!(titles(&rest), ["Import"]);
    let refused =
        query(json!({"filter": {"property": "Points", "operator": "contains", "value": "x"}}))
            .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert_eq!(refused.code(), "invalid_input");

    // The card moves to Done, first among the rows.
    let export = rows[2]["id"].as_str().expect("an id");
    let moved = t
        .dee
        .post(
            &format!("/api/v1/rows/{export}/move"),
            json!({
                "placement": "first",
                "group": {"view_id": board, "from": todo, "to": done},
            }),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    assert_eq!(
        moved.body["values"][property(&made, "Status")],
        done.as_str()
    );
    let all = query(json!({})).await;
    assert_eq!(titles(&all), ["Export", "Login", "Search", "Import"]);

    // Found by its unique id; changed; read.
    let found = t.dee.get(&format!("{base}/rows/nw-3")).await;
    assert_eq!(found.status, StatusCode::OK, "{}", found.body);
    assert_eq!(found.body["id"], export);
    assert_eq!(
        t.dee.get(&format!("{base}/rows/NW-9")).await.code(),
        "not_found"
    );
    assert_eq!(
        t.dee.get(&format!("{base}/rows/XX-3")).await.code(),
        "invalid_input"
    );
    let changed = t
        .ada
        .send(
            Method::PATCH,
            &format!("/api/v1/rows/{export}"),
            Some(json!({"values": {"Points": 13, "Name": "Export all"}})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.body["title"], "Export all");
    assert_eq!(changed.body["values"][property(&made, "Points")], 13.0);
    let read = t.cy.get(&format!("/api/v1/rows/{export}")).await;
    assert_eq!(read.body["last_edited_by"], changed.body["last_edited_by"]);
    let page = t.cy.get(&format!("/api/v1/pages/{export}")).await;
    assert_eq!(page.body["kind"], "row");
    assert_eq!(page.body["parent_id"], db.as_str());
    let nothing = t
        .ada
        .send(
            Method::PATCH,
            &format!("/api/v1/rows/{export}"),
            Some(json!({"values": {}})),
        )
        .await;
    assert_eq!(nothing.code(), "invalid_input");

    // Trashed with the pages' route, it leaves the query.
    let trashed = t
        .ada
        .post(&format!("/api/v1/pages/{export}/trash"), json!({}))
        .await;
    assert_eq!(trashed.status, StatusCode::OK, "{}", trashed.body);
    assert_eq!(
        titles(&query(json!({})).await),
        ["Login", "Search", "Import"]
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn queries_need_only_the_read_scope_and_changes_the_write_scope(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let made = database(&t.ada, &t.workspace).await;
    let db = database_id(&made);
    let read = t.ada.token(&["read"]).await;
    let write = t.ada.token(&["write"]).await;
    let send = |token: &str, method: Method, uri: String, body: Option<Value>| {
        let app = &app;
        let token = token.to_owned();
        async move { call(app, method, &uri, &[(AUTHORIZATION.as_str(), &token)], body).await }
    };
    let queried = send(
        &read,
        Method::POST,
        format!("/api/v1/databases/{db}/query"),
        Some(json!({})),
    )
    .await;
    assert_eq!(queried.status, StatusCode::OK, "{}", queried.body);
    let refused = send(
        &read,
        Method::POST,
        format!("/api/v1/databases/{db}/rows"),
        Some(json!({})),
    )
    .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert_eq!(refused.code(), "insufficient_scope");
    let made_row = send(
        &write,
        Method::POST,
        format!("/api/v1/databases/{db}/rows"),
        Some(json!({"values": {"Name": "By token"}})),
    )
    .await;
    assert_eq!(made_row.status, StatusCode::CREATED, "{}", made_row.body);
    let anonymous = call(
        &app,
        Method::POST,
        &format!("/api/v1/databases/{db}/query"),
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(
        anonymous.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        anonymous.body
    );
    // A query from another origin with the cookie is refused, as any POST.
    let cross = call(
        &app,
        Method::POST,
        &format!("/api/v1/databases/{db}/query"),
        &[
            (COOKIE.as_str(), t.ada.cookie()),
            (ORIGIN.as_str(), "https://evil.example"),
        ],
        Some(json!({})),
    )
    .await;
    assert_eq!(cross.status, StatusCode::FORBIDDEN, "{}", cross.body);

    // Someone outside the workspace is told nothing.
    user(&state, "eve@example.com").await;
    let eve = Browser::sign_in(&app, "eve@example.com").await;
    let hidden = eve.get(&format!("/api/v1/databases/{db}")).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{}", hidden.body);
    let hidden = eve
        .post(&format!("/api/v1/databases/{db}/query"), json!({}))
        .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{}", hidden.body);
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Opens a database's feed with `headers`, or answers the status the upgrade got instead.
async fn follow(
    addr: std::net::SocketAddr,
    database: &str,
    headers: &[(&str, &str)],
) -> Result<Socket, StatusCode> {
    let mut request = format!("ws://{addr}/ws/databases/{database}")
        .into_client_request()
        .expect("a request");
    for (name, value) in headers {
        request.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).expect("a header name"),
            HeaderValue::from_str(value).expect("a header value"),
        );
    }
    match timeout(PATIENCE, tokio_tungstenite::connect_async(request))
        .await
        .expect("the handshake ends")
    {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::Error::Http(response)) => Err(response.status()),
        Err(error) => panic!("the handshake failed: {error}"),
    }
}

/// A client following a database.
struct Follower {
    socket: Socket,
}

impl Follower {
    /// Opens the feed with a session cookie, and waits for `ready`.
    async fn join(addr: std::net::SocketAddr, database: &str, cookie: &str) -> Self {
        let socket = follow(addr, database, &[(COOKIE.as_str(), cookie)])
            .await
            .expect("the upgrade works");
        let mut follower = Self { socket };
        let ready = follower.next().await;
        assert_eq!(ready["type"], "ready", "{ready}");
        follower
    }

    /// The next message, skipping pings.
    async fn next(&mut self) -> Value {
        timeout(PATIENCE, async {
            loop {
                match self.socket.next().await {
                    Some(Ok(Frame::Text(text))) => {
                        return serde_json::from_str(&text).expect("JSON");
                    }
                    Some(Ok(Frame::Ping(_) | Frame::Pong(_))) => {}
                    other => panic!("no message came: {other:?}"),
                }
            }
        })
        .await
        .expect("a message came")
    }

    /// The next message of `kind`, the others before it read past.
    async fn next_of(&mut self, kind: &str) -> Value {
        loop {
            let message = self.next().await;
            if message["type"] == kind {
                return message;
            }
        }
    }

    /// The close code the server ends the feed with.
    async fn closed(&mut self) -> Option<u16> {
        timeout(PATIENCE, async {
            loop {
                match self.socket.next().await {
                    Some(Ok(Frame::Close(frame))) => return frame.map(|frame| frame.code.into()),
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return None,
                }
            }
        })
        .await
        .expect("the feed closed")
    }
}

/// Ends what a test started: the feeds, then the server.
async fn finish(state: &AppState, server: Server, followers: Vec<Follower>) {
    drop(followers);
    state.live.shutdown().await;
    state.sync.shutdown().await;
    server.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn every_client_following_a_database_hears_each_change(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let made = database(&t.ada, &t.workspace).await;
    let db = database_id(&made);
    let server = Server::start(app.clone()).await;
    let addr = server.addr;
    let mut ada = Follower::join(addr, &db, t.ada.cookie()).await;
    let mut dee = Follower::join(addr, &db, t.dee.cookie()).await;
    let base = format!("/api/v1/databases/{db}");

    let row = t
        .ada
        .post(
            &format!("{base}/rows"),
            json!({"values": {"Name": "Fix login"}}),
        )
        .await;
    assert_eq!(row.status, StatusCode::CREATED, "{}", row.body);
    let row_id = id(&row);
    for follower in [&mut ada, &mut dee] {
        let created = follower.next().await;
        assert_eq!(created["type"], "row_created", "{created}");
        assert_eq!(created["row"]["id"], row_id.as_str());
        // The row as the API has it, but for what each caller may do with it.
        let mut expected = row.body.clone();
        if let Some(fields) = expected.as_object_mut() {
            fields.remove("access");
        }
        assert_eq!(created["row"], expected);
    }
    let status = property(&made, "Status");
    let done = option(&made, "Status", "Done");
    let changed = t
        .ada
        .send(
            Method::PATCH,
            &format!("/api/v1/rows/{row_id}"),
            Some(json!({"values": {"Status": "Done"}})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    let updated = dee.next().await;
    assert_eq!(updated["type"], "row_updated");
    assert_eq!(updated["values"], json!({ status.clone(): done }));
    let board = made["views"][1]["id"].as_str().expect("a board").to_owned();
    let moved = t
        .dee
        .post(
            &format!("/api/v1/rows/{row_id}/move"),
            json!({"group": {"view_id": board, "to": null}}),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    let moved = ada.next_of("row_moved").await;
    assert_eq!(moved["values"], json!({ status: null }));
    let trashed = t
        .ada
        .post(&format!("/api/v1/pages/{row_id}/trash"), json!({}))
        .await;
    assert_eq!(trashed.status, StatusCode::OK, "{}", trashed.body);
    assert_eq!(dee.next_of("row_trashed").await["row_id"], row_id.as_str());
    t.ada
        .post(&format!("/api/v1/pages/{row_id}/restore"), json!({}))
        .await;
    assert_eq!(
        dee.next_of("row_restored").await["row"]["id"],
        row_id.as_str()
    );
    let added = t
        .ada
        .post(
            &format!("{base}/properties"),
            json!({"name": "Due", "type": "date"}),
        )
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    dee.next_of("schema_changed").await;
    let renamed = t
        .ada
        .send(
            Method::PATCH,
            &format!("{base}/views/{board}"),
            Some(json!({"name": "Kanban"})),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    let view = dee.next_of("view_changed").await;
    assert_eq!(view["view_id"], board.as_str());
    finish(&state, server, vec![ada, dee]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn feeds_are_checked_as_they_open_and_follow_access_changes(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let mine = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/databases"),
            json!({"private": true, "title": "Mine"}),
        )
        .await;
    assert_eq!(mine.status, StatusCode::CREATED, "{}", mine.body);
    let db = database_id(&mine.body);
    let server = Server::start(app.clone()).await;
    let addr = server.addr;

    // The upgrade's checks.
    assert_eq!(
        follow(addr, &db, &[]).await.err(),
        Some(StatusCode::UNAUTHORIZED)
    );
    assert_eq!(
        follow(
            addr,
            &db,
            &[
                (COOKIE.as_str(), t.ada.cookie()),
                (ORIGIN.as_str(), "https://evil.example")
            ]
        )
        .await
        .err(),
        Some(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        follow(addr, &db, &[(COOKIE.as_str(), t.dee.cookie())])
            .await
            .err(),
        Some(StatusCode::NOT_FOUND)
    );
    let plain = call(
        &app,
        Method::GET,
        &format!("/ws/databases/{db}"),
        &[(COOKIE.as_str(), t.ada.cookie())],
        None,
    )
    .await;
    assert_eq!(plain.status, StatusCode::UPGRADE_REQUIRED, "{}", plain.body);

    // A guest it is shared with follows it, and hears no people.
    user(&state, "gus@example.com").await;
    let gus = guest(&app, &t.ada, ws, "gus@example.com").await;
    let gus_id = gus.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let share = format!("/api/v1/pages/{db}/shares");
    let shared = t
        .ada
        .post(&share, json!({"user_id": gus_id, "access": "can_edit"}))
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    let socket = follow(addr, &db, &[(COOKIE.as_str(), gus.cookie())])
        .await
        .expect("the upgrade works");
    let mut follower = Follower { socket };
    let ready = follower.next().await;
    assert_eq!(
        ready,
        json!({"type": "ready", "database_id": db, "access": "edit"})
    );
    let row = t
        .ada
        .post(
            &format!("/api/v1/databases/{db}/rows"),
            json!({"values": {"Name": "Ada's"}}),
        )
        .await;
    assert_eq!(row.status, StatusCode::CREATED, "{}", row.body);
    let created = follower.next().await;
    assert_eq!(created["row"]["created_by"], Value::Null);
    assert_eq!(created["row"]["last_edited_by"], Value::Null);

    // Down to viewing: told at once.
    let changed = t
        .ada
        .send(
            Method::PATCH,
            &format!("{share}/{gus_id}"),
            Some(json!({"access": "can_view"})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(
        follower.next().await,
        json!({"type": "access", "database_id": db, "access": "view"})
    );
    // The share removed: the feed closes, and does not open again.
    let removed = t
        .ada
        .send(Method::DELETE, &format!("{share}/{gus_id}"), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    assert_eq!(follower.closed().await, Some(4404));
    assert_eq!(
        follow(addr, &db, &[(COOKIE.as_str(), gus.cookie())])
            .await
            .err(),
        Some(StatusCode::NOT_FOUND)
    );

    // Signing out closes the owner's own feed.
    let mut ada = Follower::join(addr, &db, t.ada.cookie()).await;
    let out = t.ada.post("/api/v1/auth/logout", json!({})).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT, "{}", out.body);
    assert_eq!(ada.closed().await, Some(4401));
    finish(&state, server, vec![follower, ada]).await;
}
