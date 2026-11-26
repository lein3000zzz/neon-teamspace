//! The MCP server over its Streamable HTTP transport, with the SDK's own client on a real
//! socket: initializing, the tools listed and each called, resources listed, read and
//! subscribed to with a notification after another client's change, the gate's refusals,
//! the bounds on sessions, a session ending when its agent is suspended or its token
//! revoked or expired, a person's token beside an agent's and one across the instance, concurrent
//! claims, and an edit merging with a page-sync client typing.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::header::{CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt as _;
use neon_app::ratelimit::{Bucket, Limited, MemoryLimiter, RateLimiter};
use neon_app::settings::Change;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    ProtocolVersion, ReadResourceRequestParams, ResourceContents, ResourceUpdatedNotificationParam,
    SubscribeRequestParams,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientHandler, RoleClient, ServiceExt as _};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, timeout};
use tower::ServiceExt as _;

use super::agents_tests::{Bearer, agent, agent_token};
use super::pages_tests::general;
use super::sync_tests::{Peer, Server};
use super::tests::{state, state_with_mcp};
use super::workspaces_tests::{Browser, Team, id, team};
use super::*;

/// Long enough for any step here; reaching it means something never came.
const PATIENCE: Duration = Duration::from_secs(10);

/// What the client hears of the server unasked: the resources that changed.
#[derive(Clone)]
struct Listener(mpsc::UnboundedSender<String>);

impl ClientHandler for Listener {
    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let _ = self.0.send(params.uri);
    }

    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("neon-tests", "1"),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
    }
}

/// An MCP client connected over the network, and the resources it heard changed.
struct Client {
    service: RunningService<RoleClient, Listener>,
    updates: mpsc::UnboundedReceiver<String>,
}

impl Client {
    /// Connects with `token`, a bearer token without its scheme, or fails to initialize.
    async fn connect(server: &Server, token: &str) -> Result<Self, String> {
        let config =
            StreamableHttpClientTransportConfig::with_uri(format!("http://{}/mcp", server.addr))
                .auth_header(token);
        let (sender, updates) = mpsc::unbounded_channel();
        let service = timeout(
            PATIENCE,
            Listener(sender).serve(StreamableHttpClientTransport::from_config(config)),
        )
        .await
        .map_err(|_| "initializing took too long".to_owned())?
        .map_err(|error| error.to_string())?;
        Ok(Self { service, updates })
    }

    async fn call(&self, tool: &str, arguments: Value) -> CallToolResult {
        let arguments = arguments.as_object().cloned().expect("an object");
        timeout(
            PATIENCE,
            self.service
                .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments)),
        )
        .await
        .expect("the call is answered")
        .unwrap_or_else(|error| panic!("{tool} failed: {error}"))
    }

    /// A tool's structured answer, which must be no error.
    async fn ok(&self, tool: &str, arguments: Value) -> Value {
        let result = self.call(tool, arguments).await;
        assert_ne!(result.is_error, Some(true), "{tool}: {:?}", result.content);
        result.structured_content.expect("structured content")
    }

    /// A tool's refusal: its code and message.
    async fn refused(&self, tool: &str, arguments: Value) -> (String, String) {
        let result = self.call(tool, arguments).await;
        assert_eq!(result.is_error, Some(true), "{tool} was not refused");
        let error = &result.structured_content.expect("structured content")["error"];
        (
            error["code"].as_str().expect("a code").to_owned(),
            error["message"].as_str().expect("a message").to_owned(),
        )
    }

    async fn read(&self, uri: &str) -> Vec<ResourceContents> {
        timeout(
            PATIENCE,
            self.service
                .read_resource(ReadResourceRequestParams::new(uri.to_owned())),
        )
        .await
        .expect("answered")
        .unwrap_or_else(|error| panic!("{uri}: {error}"))
        .contents
    }

    async fn subscribe(&self, uri: &str) {
        self.try_subscribe(uri)
            .await
            .unwrap_or_else(|error| panic!("subscribing to {uri}: {error}"));
    }

    #[expect(
        deprecated,
        reason = "resources/subscribe is how sessions of 2025-11-25 subscribe"
    )]
    async fn try_subscribe(&self, uri: &str) -> Result<(), rmcp::ServiceError> {
        timeout(
            PATIENCE,
            self.service
                .subscribe(SubscribeRequestParams::new(uri.to_owned())),
        )
        .await
        .expect("answered")
    }

    #[expect(
        deprecated,
        reason = "resources/unsubscribe is how sessions of 2025-11-25 unsubscribe"
    )]
    async fn unsubscribe(&self, uri: &str) {
        timeout(
            PATIENCE,
            self.service
                .unsubscribe(rmcp::model::UnsubscribeRequestParams::new(uri.to_owned())),
        )
        .await
        .expect("answered")
        .unwrap_or_else(|error| panic!("unsubscribing from {uri}: {error}"));
    }

    /// The next resource the server says changed.
    async fn updated(&mut self) -> String {
        timeout(PATIENCE, self.updates.recv())
            .await
            .expect("a notification comes")
            .expect("the client is open")
    }

    async fn close(self) {
        let _ = self.service.cancel().await;
    }
}

fn text(contents: &ResourceContents) -> &str {
    match contents {
        ResourceContents::TextResourceContents { text, .. } => text,
        other => panic!("not text: {other:?}"),
    }
}

fn mime(contents: &ResourceContents) -> Option<&str> {
    match contents {
        ResourceContents::TextResourceContents { mime_type, .. } => mime_type.as_deref(),
        ResourceContents::BlobResourceContents { mime_type, .. } => mime_type.as_deref(),
        other => panic!("unknown contents: {other:?}"),
    }
}

/// A token bound to the team's workspace that `by` makes, without its scheme.
async fn bound_token(by: &Browser<'_>, workspace: &str, scopes: &[&str]) -> String {
    let made = by
        .post(
            "/api/v1/account/tokens",
            json!({"name": "mcp", "scopes": scopes, "workspace_id": workspace}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    made.body["token"].as_str().expect("a token").to_owned()
}

/// An agent's token that `by` makes for a new agent, without its scheme, and the agent's id.
async fn agent_of(by: &Browser<'_>, workspace: &str, name: &str) -> (String, String) {
    let made = agent(by, workspace, name).await;
    let header = agent_token(by, workspace, &made, &["read", "write"]).await;
    let token = header.strip_prefix("Bearer ").expect("bearer").to_owned();
    (token, made)
}

/// The id of the account `by` is signed in as.
async fn account_id(by: &Browser<'_>) -> String {
    by.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

/// A page in General, with `markdown` as its content.
async fn page(team: &Team<'_>, title: &str, markdown: &str) -> String {
    let general = general(&team.ada, &team.workspace).await;
    let made = team
        .ada
        .post(
            &format!("/api/v1/workspaces/{}/pages", team.workspace),
            json!({"teamspace_id": general, "title": title, "markdown": markdown}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    id(&made)
}

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
fn plain(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| !line.trim().starts_with("<!-- block "))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn set(state: &AppState, key: &str, value: Value) {
    state
        .settings
        .update([(key.to_owned(), Change::Set(value))], None)
        .await
        .expect("a valid setting");
}

/// Waits until `check` holds.
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    timeout(PATIENCE, async {
        while !check().await {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("waited too long for {what}"));
}

/// Ends what a test started.
async fn finish(state: &AppState, server: Server) {
    state.mcp.shutdown().await;
    state.sync.shutdown().await;
    server.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn every_tool_acts_as_its_caller_sees(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let server = Server::start(app.clone()).await;
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();
    let general = general(&t.ada, &ws).await;
    let (scout_token, scout) = agent_of(&t.cy, &ws, "Scout").await;
    let scout_client = Client::connect(&server, &scout_token)
        .await
        .expect("connects");
    let ada_client = Client::connect(&server, &bound_token(&t.ada, &ws, &["read", "write"]).await)
        .await
        .expect("connects");

    let info = scout_client
        .service
        .peer_info()
        .expect("the server introduced itself");
    assert_eq!(info.protocol_version, ProtocolVersion::V_2025_11_25);
    assert_eq!(
        info.server_info.as_ref().map(|server| server.name.as_str()),
        Some("neon")
    );
    let tools: Vec<String> = scout_client
        .service
        .list_all_tools()
        .await
        .expect("listed")
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect();
    assert_eq!(
        tools,
        [
            "whoami",
            "list_members",
            "list_teamspaces",
            "get_page_tree",
            "read_page",
            "create_page",
            "edit_page",
            "move_page",
            "trash_page",
            "restore_page",
            "get_database",
            "query_database",
            "create_database",
            "create_row",
            "update_row",
            "claim_task",
        ]
    );

    // Orientation: an agent and its owner, a person, the workspace.
    let me = scout_client.ok("whoami", json!({})).await;
    assert_eq!(me["identity"]["id"], scout.as_str());
    assert_eq!(me["identity"]["kind"], "agent");
    assert_eq!(me["identity"]["owner"]["display_name"], "cy");
    assert_eq!(me["workspace"]["id"], ws.as_str());
    assert_eq!(me["workspace"]["role"], "member");
    assert_eq!(me["scopes"], json!(["read", "write"]));
    let me = ada_client.ok("whoami", json!({})).await;
    assert_eq!(me["identity"]["kind"], "human");
    assert_eq!(me["identity"]["display_name"], "ada");
    let members = scout_client.ok("list_members", json!({})).await;
    let names: Vec<&str> = members["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|member| member["user"]["display_name"].as_str().expect("a name"))
        .collect();
    assert!(
        names.contains(&"Scout") && names.contains(&"ada"),
        "{names:?}"
    );
    let teamspaces = scout_client.ok("list_teamspaces", json!({})).await;
    assert_eq!(teamspaces["teamspaces"][0]["id"], general.as_str());

    // Pages: made with markdown, read with block ids, edited by them, nested, moved,
    // trashed and restored.
    let made = scout_client
        .ok(
            "create_page",
            json!({"teamspace_id": general, "title": "Plan", "markdown": "# Goals\n\nShip"}),
        )
        .await;
    let plan = made["page"]["id"].as_str().expect("an id").to_owned();
    assert_eq!(made["page"]["created_by"], scout.as_str());
    let ids = block_ids(made["markdown"].as_str().expect("markdown"));
    assert_eq!(ids.len(), 2);
    let read = ada_client.ok("read_page", json!({"page_id": plan})).await;
    assert_eq!(
        plain(read["markdown"].as_str().expect("markdown")),
        "# Goals\n\nShip"
    );
    let edited = scout_client
        .ok(
            "edit_page",
            json!({"page_id": plan, "operations": [
                {"op": "insert_after", "block_id": ids[0], "markdown": "- one\n- two"},
                {"op": "find_replace", "find": "Ship", "replace": "Ship it"},
                {"op": "append", "markdown": "Done."},
            ]}),
        )
        .await;
    assert_eq!(
        edited["results"][0]["added"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(edited["results"][1]["replaced"], 1);
    let read = ada_client.ok("read_page", json!({"page_id": plan})).await;
    assert_eq!(
        plain(read["markdown"].as_str().expect("markdown")),
        "# Goals\n\n- one\n- two\n\nShip it\n\nDone."
    );
    let (code, message) = scout_client
        .refused(
            "edit_page",
            json!({"page_id": plan, "operations": [
                {"op": "replace_range", "from": "nope", "markdown": ""},
            ]}),
        )
        .await;
    assert_eq!(code, "edit_refused");
    assert_eq!(
        message,
        "edit 0: the page has no block nope; nothing was changed"
    );
    let child = scout_client
        .ok("create_page", json!({"parent_id": plan, "title": "Notes"}))
        .await["page"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let tree = scout_client
        .ok("get_page_tree", json!({"teamspace_id": general}))
        .await;
    let top = tree["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .find(|node| node["id"] == plan.as_str())
        .expect("the plan is at the top")
        .clone();
    assert_eq!(top["child_count"], 1);
    assert_eq!(top["children"][0]["id"], child.as_str());
    let shallow = scout_client
        .ok(
            "get_page_tree",
            json!({"teamspace_id": general, "depth": 1}),
        )
        .await;
    assert!(
        shallow["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .all(|node| node.get("children").is_none())
    );
    // An agent has no private pages, as its owner could not see them; a person does.
    let (code, _) = scout_client.refused("get_page_tree", json!({})).await;
    assert_eq!(code, "forbidden");
    let private = ada_client.ok("get_page_tree", json!({})).await;
    assert_eq!(private["private"], true);
    let moved = scout_client
        .ok(
            "move_page",
            json!({"page_id": child, "teamspace_id": general, "placement": "first"}),
        )
        .await;
    assert_eq!(moved["parent_id"], Value::Null);
    let trashed = scout_client
        .ok("trash_page", json!({"page_id": child}))
        .await;
    assert!(trashed["trashed_at"].is_string());
    let restored = scout_client
        .ok("restore_page", json!({"page_id": child}))
        .await;
    assert_eq!(restored["trashed_at"], Value::Null);

    // Databases: made, read, queried with a filter, their rows made, changed by id and by
    // unique id, and claimed.
    let database = scout_client
        .ok(
            "create_database",
            json!({"teamspace_id": general, "title": "Tasks"}),
        )
        .await;
    let db = database["page"]["id"].as_str().expect("an id").to_owned();
    for property in [
        json!({"name": "Assignee", "type": "person"}),
        json!({"name": "Key", "type": "unique_id", "prefix": "NW"}),
    ] {
        let added = t
            .ada
            .post(&format!("/api/v1/databases/{db}/properties"), property)
            .await;
        assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    }
    let read = scout_client
        .ok("get_database", json!({"database_id": db}))
        .await;
    assert_eq!(read["page"]["title"], "Tasks");
    let property = |name: &str| {
        read["properties"]
            .as_array()
            .expect("properties")
            .iter()
            .find(|property| property["name"] == name)
            .and_then(|property| property["id"].as_str())
            .unwrap_or_else(|| panic!("no {name}"))
            .to_owned()
    };
    let row = scout_client
        .ok(
            "create_row",
            json!({"database_id": db, "values": {"Name": "Fix login"}}),
        )
        .await;
    let row_id = row["id"].as_str().expect("an id").to_owned();
    assert_eq!(row["unique_id"], "NW-1");
    scout_client
        .ok(
            "create_row",
            json!({"database_id": db, "values": {"Name": "Write docs", "Status": "Done"}}),
        )
        .await;
    let changed = scout_client
        .ok(
            "update_row",
            json!({"database_id": db, "unique_id": "NW-2", "values": {"Name": "Write the docs"}}),
        )
        .await;
    assert_eq!(changed["title"], "Write the docs");
    let changed = scout_client
        .ok(
            "update_row",
            json!({"row_id": row_id, "values": {"Name": "Fix the login"}}),
        )
        .await;
    assert_eq!(changed["title"], "Fix the login");
    let (code, _) = scout_client
        .refused(
            "update_row",
            json!({"row_id": row_id, "unique_id": "NW-1", "values": {}}),
        )
        .await;
    assert_eq!(code, "invalid_arguments");
    let done = scout_client
        .ok(
            "query_database",
            json!({"database_id": db, "filter": {"property": "Status", "operator": "is", "value": "Done"}}),
        )
        .await;
    let titles: Vec<&str> = done["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["title"].as_str().expect("a title"))
        .collect();
    assert_eq!(titles, ["Write the docs"]);
    let claimed = scout_client
        .ok(
            "claim_task",
            json!({"database_id": db, "unique_id": "NW-1"}),
        )
        .await;
    assert_eq!(claimed["values"][property("Assignee")], json!([scout]));
    let (code, message) = ada_client
        .refused("claim_task", json!({"row_id": row_id}))
        .await;
    assert_eq!(code, "already_claimed");
    assert!(message.contains(&scout), "{message}");
    let held = ada_client
        .call("claim_task", json!({"row_id": row_id}))
        .await;
    assert_eq!(
        held.structured_content.expect("structured")["error"]["holders"][0]["id"],
        scout.as_str()
    );
    let task = ada_client.ok("read_page", json!({"page_id": row_id})).await;
    let assignee = task["properties"]
        .as_array()
        .expect("properties")
        .iter()
        .find(|property| property["name"] == "Assignee")
        .expect("the assignee")
        .clone();
    assert_eq!(assignee["text"], "Scout · via cy");

    // Arguments are checked; unknown tools are protocol errors.
    let (code, message) = scout_client
        .refused("read_page", json!({"page": plan}))
        .await;
    assert_eq!(code, "invalid_arguments");
    assert!(message.contains("unknown field `page`"), "{message}");
    let unknown = scout_client
        .service
        .call_tool(CallToolRequestParams::new("search"))
        .await;
    assert!(unknown.is_err(), "{unknown:?}");

    // Nothing of another workspace is seen, nor told to exist.
    let elsewhere = t
        .bob
        .post("/api/v1/workspaces", json!({"name": "Elsewhere"}))
        .await;
    let elsewhere = id(&elsewhere);
    let theirs = t
        .bob
        .post(
            &format!("/api/v1/workspaces/{elsewhere}/pages"),
            json!({"private": true, "title": "Secret"}),
        )
        .await;
    let theirs = id(&theirs);
    let (code, message) = scout_client
        .refused("read_page", json!({"page_id": theirs}))
        .await;
    assert_eq!(code, "not_found");
    let (_, missing) = scout_client
        .refused("read_page", json!({"page_id": uuid::Uuid::now_v7()}))
        .await;
    assert_eq!(
        message, missing,
        "a page elsewhere reads as one that is not there"
    );

    // A token with the read scope alone reads, and changes nothing.
    let reader = Client::connect(&server, &bound_token(&t.dee, &ws, &["read"]).await)
        .await
        .expect("connects");
    reader.ok("read_page", json!({"page_id": plan})).await;
    let (code, _) = reader.refused("trash_page", json!({"page_id": plan})).await;
    assert_eq!(code, "insufficient_scope");

    for client in [scout_client, ada_client, reader] {
        client.close().await;
    }
    finish(&state, server).await;
}

/// A raw request to `/mcp`, its answer's status, headers and body as text.
async fn raw(
    app: &Router,
    method: Method,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut request = Request::builder()
        .method(method)
        .uri("/mcp")
        .header("accept", "application/json, text/event-stream");
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        request = request.header("host", "neon.example.com");
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let body = match body {
        Some(json) => {
            request = request.header(CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).expect("a request"))
        .await
        .expect("a response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = timeout(PATIENCE, response.into_body().collect())
        .await
        .expect("the body ends")
        .expect("a body")
        .to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "raw", "version": "1"},
        },
    })
}

fn code_of(body: &str) -> String {
    let value: Value = serde_json::from_str(body).unwrap_or_else(|_| panic!("not JSON: {body}"));
    value["error"]["code"].as_str().expect("a code").to_owned()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_gate_wants_a_token_bound_to_a_workspace_on_every_request(pool: PgPool) {
    let web = WebConfig::new(
        Some(&url::Url::parse("https://neon.example.com").expect("a URL")),
        Vec::new(),
    );
    let state = state_with_mcp(
        pool,
        web,
        neon_app::sync::Limits::default(),
        Arc::new(MemoryLimiter::new()),
    )
    .await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();
    let challenge = "Bearer resource_metadata=\"https://neon.example.com/.well-known/oauth-protected-resource\"";

    // No token, a cookie alone, a token that does not work: 401, naming the metadata.
    for headers in [
        vec![],
        vec![("cookie", t.ada.cookie())],
        vec![("authorization", "Bearer nw_nothing")],
        vec![("authorization", "Basic abc")],
    ] {
        let (status, answered, body) = raw(&app, Method::POST, &headers, Some(initialize())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{headers:?}: {body}");
        assert_eq!(code_of(&body), "unauthenticated");
        assert_eq!(answered[WWW_AUTHENTICATE], challenge);
    }
    // A token across the instance: refused, saying why.
    let across = t.ada.token(&["read", "write"]).await;
    let (status, _, body) = raw(
        &app,
        Method::POST,
        &[("authorization", &across)],
        Some(initialize()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(code_of(&body), "token_not_bound");
    assert!(body.contains("one workspace"), "{body}");

    let token = format!("Bearer {}", bound_token(&t.ada, &ws, &["read"]).await);
    let auth = ("authorization", token.as_str());
    // Another origin is refused; the server's own is not.
    let (status, _, _) = raw(
        &app,
        Method::POST,
        &[auth, ("origin", "https://evil.example")],
        Some(initialize()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, opened, body) = raw(
        &app,
        Method::POST,
        &[auth, ("origin", "https://neon.example.com")],
        Some(initialize()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("\"protocolVersion\":\"2025-11-25\""),
        "{body}"
    );
    let session = opened["mcp-session-id"]
        .to_str()
        .expect("a session id")
        .to_owned();
    assert_eq!(state.mcp.session_count(), 1);

    // Outside a session only initialize is taken; a session is its identity's alone.
    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
    let (status, _, body) = raw(&app, Method::POST, &[auth], Some(list.clone())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(code_of(&body), "session_required");
    let dee = format!("Bearer {}", bound_token(&t.dee, &ws, &["read"]).await);
    let (status, _, body) = raw(
        &app,
        Method::POST,
        &[("authorization", &dee), ("mcp-session-id", &session)],
        Some(list.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // Another token of the same identity and workspace carries on the session.
    let again = format!("Bearer {}", bound_token(&t.ada, &ws, &["read"]).await);
    let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let (status, _, body) = raw(
        &app,
        Method::POST,
        &[("authorization", &again), ("mcp-session-id", &session)],
        Some(initialized),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let (status, _, body) = raw(
        &app,
        Method::POST,
        &[
            ("authorization", &again),
            ("mcp-session-id", &session),
            ("mcp-protocol-version", "2025-11-25"),
        ],
        Some(list),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("claim_task"), "{body}");
    // Ended by its client, it is gone.
    let (status, _, _) = raw(
        &app,
        Method::DELETE,
        &[auth, ("mcp-session-id", &session)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(state.mcp.session_count(), 0);

    // People may be kept out: then only agents connect.
    set(&state, "mcp.identities", json!(["agent"])).await;
    let (status, _, body) = raw(&app, Method::POST, &[auth], Some(initialize())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code_of(&body), "identity_not_allowed");
    state.mcp.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn without_a_public_url_only_an_address_or_localhost_stands_in_for_the_origin(pool: PgPool) {
    let state = state_with_mcp(
        pool,
        WebConfig::new(None, Vec::new()),
        neon_app::sync::Limits::default(),
        Arc::new(MemoryLimiter::new()),
    )
    .await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let token = format!(
        "Bearer {}",
        bound_token(&t.ada, &t.workspace, &["read"]).await
    );
    // A page that rebinds a name of its own to the server sends that name as Host too.
    for (host, origin, allowed) in [
        ("rebound.example:8080", "http://rebound.example:8080", false),
        ("127.0.0.1:8080", "http://evil.example", false),
        ("127.0.0.1:8080", "http://127.0.0.1:8080", true),
        ("[::1]:8080", "http://[::1]:8080", true),
        ("localhost:5173", "http://localhost:5173", true),
    ] {
        let (status, _, body) = raw(
            &app,
            Method::POST,
            &[
                ("authorization", &token),
                ("host", host),
                ("origin", origin),
            ],
            Some(initialize()),
        )
        .await;
        let expected = if allowed {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        };
        assert_eq!(status, expected, "{origin} to {host}: {body}");
    }
    // A client that is no browser sends no Origin, and needs no public URL.
    let (status, _, body) = raw(
        &app,
        Method::POST,
        &[("authorization", &token), ("host", "neon.example.com")],
        Some(initialize()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    state.mcp.shutdown().await;
}

/// A limiter that records the keys the MCP server's requests are counted by, and refuses
/// those of one key in one bucket.
#[derive(Default)]
struct Recording {
    refused: std::sync::Mutex<Option<(Bucket, String)>>,
    seen: std::sync::Mutex<Vec<(Bucket, String)>>,
}

impl Recording {
    fn refuse(&self, bucket: Bucket, key: &str) {
        *self.refused.lock().expect("not poisoned") = Some((bucket, key.to_owned()));
    }

    /// The keys seen since last asked, in order.
    fn seen(&self) -> Vec<(Bucket, String)> {
        std::mem::take(&mut *self.seen.lock().expect("not poisoned"))
    }
}

#[async_trait]
impl RateLimiter for Recording {
    async fn check(&self, bucket: Bucket, key: &str) -> Result<(), Limited> {
        if !matches!(bucket, Bucket::Mcp | Bucket::McpPerson) {
            return Ok(());
        }
        let counted = (bucket, key.to_owned());
        self.seen
            .lock()
            .expect("not poisoned")
            .push(counted.clone());
        if self.refused.lock().expect("not poisoned").as_ref() == Some(&counted) {
            return Err(Limited {
                retry_after: Duration::from_secs(3),
            });
        }
        Ok(())
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn requests_past_their_rate_are_refused_by_identity_and_by_person(pool: PgPool) {
    let limiter = Arc::new(Recording::default());
    let state = state_with_mcp(
        pool,
        WebConfig::new(None, Vec::new()),
        neon_app::sync::Limits::default(),
        Arc::clone(&limiter) as Arc<dyn RateLimiter>,
    )
    .await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();
    let (ada, dee) = (account_id(&t.ada).await, account_id(&t.dee).await);
    let ada_token = format!("Bearer {}", bound_token(&t.ada, &ws, &["read"]).await);
    let dee_token = format!("Bearer {}", bound_token(&t.dee, &ws, &["read"]).await);
    let (scout_token, scout) = agent_of(&t.ada, &ws, "Scout").await;
    let scout_token = format!("Bearer {scout_token}");
    let initialize_as = |token: &str| {
        let (app, token) = (app.clone(), token.to_owned());
        async move {
            raw(
                &app,
                Method::POST,
                &[("authorization", &token)],
                Some(initialize()),
            )
            .await
        }
    };

    // A person counts as themself twice; an agent as itself, then as its owner.
    let (status, _, body) = initialize_as(&ada_token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        limiter.seen(),
        [(Bucket::Mcp, ada.clone()), (Bucket::McpPerson, ada.clone())]
    );
    let (status, _, body) = initialize_as(&scout_token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        limiter.seen(),
        [
            (Bucket::Mcp, scout.clone()),
            (Bucket::McpPerson, ada.clone())
        ]
    );

    // Past one identity's rate, it is refused, and others are not.
    limiter.refuse(Bucket::Mcp, &ada);
    let (status, headers, body) = initialize_as(&ada_token).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(code_of(&body), "rate_limited");
    assert_eq!(headers["retry-after"], "3");
    for token in [&scout_token, &dee_token] {
        let (status, _, body) = initialize_as(token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    // Past one person's rate, their agents are refused too.
    limiter.refuse(Bucket::McpPerson, &ada);
    let (status, _, body) = initialize_as(&scout_token).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    let (status, _, body) = initialize_as(&dee_token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(limiter.seen().last(), Some(&(Bucket::McpPerson, dee)));
    state.mcp.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn sessions_are_bounded_and_end_when_idle(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let server = Server::start(app.clone()).await;
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();
    let token = bound_token(&t.ada, &ws, &["read"]).await;

    set(&state, "mcp.max_sessions_per_identity", json!(1)).await;
    let first = Client::connect(&server, &token).await.expect("connects");
    let refused = Client::connect(&server, &token)
        .await
        .err()
        .expect("a second session is refused");
    assert!(
        refused.contains("429") || refused.contains("Too Many"),
        "{refused}"
    );
    // Another identity has its own bound.
    let other = Client::connect(&server, &bound_token(&t.dee, &ws, &["read"]).await)
        .await
        .expect("connects");
    set(&state, "mcp.max_sessions", json!(2)).await;
    let full = Client::connect(&server, &bound_token(&t.cy, &ws, &["read"]).await)
        .await
        .err()
        .expect("the server is full");
    assert!(full.contains("429") || full.contains("Too Many"), "{full}");
    first.close().await;
    eventually("the first session to end", || async {
        state.mcp.session_count() == 1
    })
    .await;
    let again = Client::connect(&server, &token)
        .await
        .expect("connects again");
    again.close().await;
    other.close().await;
    eventually("every session to end", || async {
        state.mcp.session_count() == 0
    })
    .await;

    // A session with no request and no stream ends once idle; one listening stays.
    set(&state, "mcp.session_idle_timeout", json!("1s")).await;
    let auth = format!("Bearer {token}");
    let started = Instant::now();
    let (status, opened, body) = raw(
        &app,
        Method::POST,
        &[("authorization", &auth)],
        Some(initialize()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let idle = opened["mcp-session-id"].to_str().expect("an id").to_owned();
    let listening = Client::connect(&server, &bound_token(&t.dee, &ws, &["read"]).await)
        .await
        .expect("connects");
    eventually("the idle session to end", || async {
        state.mcp.session_count() == 1
    })
    .await;
    // It opened after `started`, and was idle from then on.
    assert!(started.elapsed() >= Duration::from_secs(1));
    let (status, _, _) = raw(
        &app,
        Method::POST,
        &[("authorization", &auth), ("mcp-session-id", &idle)],
        Some(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    sleep(Duration::from_millis(1500)).await;
    let me = listening.ok("whoami", json!({})).await;
    assert_eq!(me["identity"]["display_name"], "dee");
    listening.close().await;
    finish(&state, server).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_idle_session_ends_while_access_changes_keep_coming(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let plan = page(&t, "Plan", "Text").await;
    set(&state, "mcp.session_idle_timeout", json!("1s")).await;
    let token = format!(
        "Bearer {}",
        bound_token(&t.ada, &t.workspace, &["read"]).await
    );
    let (status, _, body) = raw(
        &app,
        Method::POST,
        &[("authorization", &token)],
        Some(initialize()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(state.mcp.session_count(), 1);

    // Trashing a page and restoring it are access changes, each of which has the watcher
    // look at the sessions again: they come many times between its looks for idle ones.
    timeout(PATIENCE, async {
        while state.mcp.session_count() > 0 {
            for step in ["trash", "restore"] {
                let changed = t
                    .ada
                    .post(&format!("/api/v1/pages/{plan}/{step}"), json!({}))
                    .await;
                assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
            }
        }
    })
    .await
    .expect("the idle session to end");
    state.mcp.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn one_person_s_agents_share_one_bound_and_leave_room_for_others(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();
    set(&state, "mcp.max_sessions_per_person", json!(2)).await;
    let mut agents = Vec::new();
    for name in ["A1", "A2", "A3"] {
        agents.push(format!("Bearer {}", agent_of(&t.ada, &ws, name).await.0));
    }
    let initialize_as = |token: &str| {
        let (app, token) = (app.clone(), token.to_owned());
        async move {
            raw(
                &app,
                Method::POST,
                &[("authorization", &token)],
                Some(initialize()),
            )
            .await
        }
    };

    let (status, opened, body) = initialize_as(&agents[0]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let first = opened["mcp-session-id"].to_str().expect("an id").to_owned();
    let (status, _, body) = initialize_as(&agents[1]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // A third agent of hers, and she herself, find her bound taken, each identity of hers
    // far from its own.
    let ada = format!("Bearer {}", bound_token(&t.ada, &ws, &["read"]).await);
    for token in [&agents[2], &ada] {
        let (status, _, body) = initialize_as(token).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
        assert_eq!(code_of(&body), "too_many_sessions");
        assert!(body.contains("with their agents"), "{body}");
    }
    // Others still connect.
    let dee = format!("Bearer {}", bound_token(&t.dee, &ws, &["read"]).await);
    let (status, _, body) = initialize_as(&dee).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(state.mcp.session_count(), 3);

    // One of hers ended, another of hers opens.
    let (status, _, _) = raw(
        &app,
        Method::DELETE,
        &[("authorization", &agents[0]), ("mcp-session-id", &first)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _, body) = initialize_as(&agents[2]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    state.mcp.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_session_ends_at_once_when_what_it_acts_as_stops_working(pool: PgPool) {
    let state = state(pool.clone()).await;
    let app = router(state.clone(), false);
    let server = Server::start(app.clone()).await;
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();

    // An agent whose owner leaves the workspace is suspended: its session ends unasked.
    let (token, _) = agent_of(&t.cy, &ws, "Scout").await;
    let scout = Client::connect(&server, &token).await.expect("connects");
    scout.ok("whoami", json!({})).await;
    let cy = t.cy.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    assert_eq!(state.mcp.session_count(), 1);
    let removed = t
        .ada
        .send(
            Method::DELETE,
            &format!("/api/v1/workspaces/{ws}/members/{cy}"),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    eventually("the suspended agent's session to end", || async {
        state.mcp.session_count() == 0
    })
    .await;
    let after = scout
        .service
        .call_tool(CallToolRequestParams::new("whoami"))
        .await;
    assert!(after.is_err(), "{after:?}");
    scout.close().await;

    // A token revoked ends its session too.
    let made = t
        .dee
        .post(
            "/api/v1/account/tokens",
            json!({"name": "mcp", "scopes": ["read"], "workspace_id": ws}),
        )
        .await;
    let token_id = made.body["api_token"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let dee = Client::connect(&server, made.body["token"].as_str().expect("a token"))
        .await
        .expect("connects");
    assert_eq!(state.mcp.session_count(), 1);
    let revoked = t
        .dee
        .send(
            Method::DELETE,
            &format!("/api/v1/account/tokens/{token_id}"),
            None,
        )
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    eventually("the revoked token's session to end", || async {
        state.mcp.session_count() == 0
    })
    .await;
    dee.close().await;

    // A person removed from the workspace: their bound token reaches nothing, and the
    // session ends.
    let bob = Client::connect(&server, &bound_token(&t.bob, &ws, &["read"]).await)
        .await
        .expect("connects");
    let bob_id = t.bob.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    t.ada
        .send(
            Method::DELETE,
            &format!("/api/v1/workspaces/{ws}/members/{bob_id}"),
            None,
        )
        .await;
    eventually("the removed person's session to end", || async {
        state.mcp.session_count() == 0
    })
    .await;
    bob.close().await;

    // A token that expires tells nobody: its next request is refused, and its session
    // ends then.
    let token = bound_token(&t.dee, &ws, &["read"]).await;
    let auth = format!("Bearer {token}");
    let (status, opened, body) = raw(
        &app,
        Method::POST,
        &[("authorization", &auth)],
        Some(initialize()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session = opened["mcp-session-id"].to_str().expect("an id").to_owned();
    assert_eq!(state.mcp.session_count(), 1);
    sqlx::query("UPDATE api_tokens SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .expect("expired");
    let (status, _, _) = raw(
        &app,
        Method::POST,
        &[("authorization", &auth), ("mcp-session-id", &session)],
        Some(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    eventually("the expired token's session to end", || async {
        state.mcp.session_count() == 0
    })
    .await;
    finish(&state, server).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn resources_are_listed_read_and_tell_subscribers_of_changes(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let server = Server::start(app.clone()).await;
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();
    let (token, scout) = agent_of(&t.ada, &ws, "Scout").await;
    let mut client = Client::connect(&server, &token).await.expect("connects");
    let plan = page(&t, "Plan", "# Goals\n\nShip").await;
    let favorite = Bearer::new(&app, format!("Bearer {token}"))
        .post("/api/v1/account/favorites", json!({"page_id": plan}))
        .await;
    assert_eq!(favorite.status, StatusCode::CREATED, "{}", favorite.body);
    let general = general(&t.ada, &ws).await;
    let db = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/databases"),
            json!({"teamspace_id": general, "title": "Tasks"}),
        )
        .await
        .body;
    let db_id = db["page"]["id"].as_str().expect("an id").to_owned();
    let view = db["views"][0]["id"].as_str().expect("a view").to_owned();
    t.ada
        .post(
            &format!("/api/v1/databases/{db_id}/properties"),
            json!({"name": "Assignee", "type": "person"}),
        )
        .await;
    let task = t
        .ada
        .post(
            &format!("/api/v1/databases/{db_id}/rows"),
            json!({"values": {"Name": "Fix login", "Assignee": [scout]}}),
        )
        .await;
    let task = id(&task);

    // The list: tasks first, then favorites, the pages changed last and each task.
    let templates: Vec<String> = client
        .service
        .list_all_resource_templates()
        .await
        .expect("listed")
        .into_iter()
        .map(|template| template.uri_template.clone())
        .collect();
    assert_eq!(
        templates,
        [
            "neon://page/{id}",
            "neon://database/{id}",
            "neon://database/{id}/view/{view}",
            "neon://me/tasks"
        ]
    );
    let listed: Vec<String> = client
        .service
        .list_all_resources()
        .await
        .expect("listed")
        .into_iter()
        .map(|resource| resource.uri.clone())
        .collect();
    assert_eq!(listed[0], "neon://me/tasks");
    assert_eq!(listed[1], format!("neon://page/{plan}"));
    assert!(
        listed.contains(&format!("neon://database/{db_id}")),
        "{listed:?}"
    );
    assert!(
        listed.contains(&format!("neon://page/{task}")),
        "{listed:?}"
    );
    let unique: std::collections::HashSet<&String> = listed.iter().collect();
    assert_eq!(unique.len(), listed.len(), "each once: {listed:?}");

    // Each template reads.
    let read = client.read(&format!("neon://page/{plan}")).await;
    assert_eq!(mime(&read[0]), Some("text/markdown"));
    assert_eq!(plain(text(&read[0])), "# Goals\n\nShip");
    let about: Value = serde_json::from_str(text(&read[1])).expect("JSON");
    assert_eq!(about["page"]["title"], "Plan");
    let read = client.read(&format!("neon://database/{db_id}")).await;
    let database: Value = serde_json::from_str(text(&read[0])).expect("JSON");
    assert_eq!(database["page"]["title"], "Tasks");
    let read = client
        .read(&format!("neon://database/{db_id}/view/{view}"))
        .await;
    let rows: Value = serde_json::from_str(text(&read[0])).expect("JSON");
    assert_eq!(rows["rows"][0]["title"], "Fix login");
    let read = client.read("neon://me/tasks").await;
    let tasks: Value = serde_json::from_str(text(&read[0])).expect("JSON");
    assert_eq!(tasks["tasks"][0]["row"]["id"], task.as_str());
    let missing = client
        .service
        .read_resource(ReadResourceRequestParams::new("neon://me/inbox".to_owned()))
        .await;
    assert!(missing.is_err());

    // A subscription hears of another client's change to the page.
    let uri = format!("neon://page/{plan}");
    client.subscribe(&uri).await;
    let edited = t
        .dee
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{plan}/markdown"),
            Some(find_replace("Ship", "Ship it")),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(client.updated().await, uri);

    // A view's rows, and the caller's tasks, hear of rows changing.
    let view_uri = format!("neon://database/{db_id}/view/{view}");
    client.subscribe(&view_uri).await;
    client.subscribe("neon://me/tasks").await;
    t.dee
        .post(
            &format!("/api/v1/databases/{db_id}/rows"),
            json!({"values": {"Name": "Another", "Assignee": [scout]}}),
        )
        .await;
    let mut heard = vec![client.updated().await, client.updated().await];
    heard.sort();
    let mut expected = vec![view_uri.clone(), "neon://me/tasks".to_owned()];
    expected.sort();
    assert_eq!(heard, expected);

    // Unsubscribed, or out of reach, nothing more comes.
    client.unsubscribe(&view_uri).await;
    assert_eq!(state.mcp.subscription_count(), 2);

    // A page moved where the agent cannot see it: its subscription drops.
    let hidden = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces"),
            json!({"name": "Hidden", "access": "private"}),
        )
        .await;
    assert_eq!(hidden.status, StatusCode::CREATED, "{}", hidden.body);
    let moved = t
        .ada
        .post(
            &format!("/api/v1/pages/{plan}/move"),
            json!({"teamspace_id": id(&hidden)}),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    eventually(
        "the subscription to the page out of reach to drop",
        || async { state.mcp.subscription_count() == 1 },
    )
    .await;
    assert_eq!(state.mcp.session_count(), 1, "the session stays");
    let missing = client
        .try_subscribe(&format!("neon://page/{}", uuid::Uuid::now_v7()))
        .await;
    assert!(missing.is_err());
    client.close().await;
    finish(&state, server).await;
}

/// A database in teamspace `teamspace` with a person property `Assignee`, and its id.
async fn database_with_assignee(team: &Team<'_>, teamspace: &str, title: &str) -> String {
    let made = team
        .ada
        .post(
            &format!("/api/v1/workspaces/{}/databases", team.workspace),
            json!({"teamspace_id": teamspace, "title": title}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let id = made.body["page"]["id"].as_str().expect("an id").to_owned();
    let property = team
        .ada
        .post(
            &format!("/api/v1/databases/{id}/properties"),
            json!({"name": "Assignee", "type": "person"}),
        )
        .await;
    assert_eq!(property.status, StatusCode::CREATED, "{}", property.body);
    id
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_tasks_subscription_hears_only_of_what_its_caller_sees(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let server = Server::start(app.clone()).await;
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();
    let general = general(&t.ada, &ws).await;
    let (token, scout) = agent_of(&t.cy, &ws, "Scout").await;
    let mut client = Client::connect(&server, &token).await.expect("connects");
    let visible = database_with_assignee(&t, &general, "Visible").await;
    let plan = page(&t, "Plan", "Ship").await;
    let page_uri = format!("neon://page/{plan}");
    client.subscribe("neon://me/tasks").await;
    client.subscribe(&page_uri).await;

    // In a teamspace the agent cannot see, a database changes and a row names the agent.
    let hidden = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces"),
            json!({"name": "Hidden", "access": "private"}),
        )
        .await;
    assert_eq!(hidden.status, StatusCode::CREATED, "{}", hidden.body);
    let secret = database_with_assignee(&t, &id(&hidden), "Secret").await;
    let row = t
        .ada
        .post(
            &format!("/api/v1/databases/{secret}/rows"),
            json!({"values": {"Name": "Secret task", "Assignee": [scout]}}),
        )
        .await;
    assert_eq!(row.status, StatusCode::CREATED, "{}", row.body);

    // The next the agent hears of is the page it sees changing, not its tasks: nothing of
    // them it sees changed.
    let edited = t
        .ada
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{plan}/markdown"),
            Some(json!({"operations": [{"op": "append", "markdown": "More"}]})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(client.updated().await, page_uri);

    // A row it sees naming it is told.
    let row = t
        .ada
        .post(
            &format!("/api/v1/databases/{visible}/rows"),
            json!({"values": {"Name": "Open task", "Assignee": [scout]}}),
        )
        .await;
    assert_eq!(row.status, StatusCode::CREATED, "{}", row.body);
    assert_eq!(client.updated().await, "neon://me/tasks");
    let read = client.read("neon://me/tasks").await;
    let tasks: Value = serde_json::from_str(text(&read[0])).expect("JSON");
    let titles: Vec<&str> = tasks["tasks"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|task| task["row"]["title"].as_str().expect("a title"))
        .collect();
    assert_eq!(titles, ["Open task"]);
    client.close().await;
    finish(&state, server).await;
}

/// An edit replacing `find` with `replace`, as the markdown route takes it.
fn find_replace(find: &str, replace: &str) -> Value {
    json!({"operations": [{"op": "find_replace", "find": find, "replace": replace}]})
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn changes_within_the_interval_are_told_together_once_it_ends(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let server = Server::start(app.clone()).await;
    let t = team(&state, &app).await;
    // Long enough that the changes after the first are sure to come within it.
    set(&state, "mcp.notification_interval", json!("30s")).await;
    let (token, _) = agent_of(&t.ada, &t.workspace, "Scout").await;
    let mut client = Client::connect(&server, &token).await.expect("connects");
    let plan = page(&t, "Plan", "# Goals\n\nShip").await;
    let uri = format!("neon://page/{plan}");
    client.subscribe(&uri).await;
    let edit = |find: &str, replace: &str| {
        let body = find_replace(find, replace);
        let path = format!("/api/v1/pages/{plan}/markdown");
        let dee = &t.dee;
        async move {
            let edited = dee.send(Method::PATCH, &path, Some(body)).await;
            assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
        }
    };

    // The first change is told at once.
    edit("Ship", "Ship it").await;
    assert_eq!(client.updated().await, uri);
    let first = Instant::now();
    // The two after it are told together, once the interval since the first is over.
    edit("Ship it", "Ship it now").await;
    edit("Goals", "Aims").await;
    let next = timeout(Duration::from_secs(60), client.updates.recv())
        .await
        .expect("the changes are told")
        .expect("the client is open");
    assert_eq!(next, uri);
    assert!(
        first.elapsed() >= Duration::from_secs(15),
        "told after {:?}, within the interval",
        first.elapsed()
    );
    let read = client.read(&uri).await;
    assert_eq!(plain(text(&read[0])), "# Aims\n\nShip it now");
    client.close().await;
    finish(&state, server).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn concurrent_claims_have_exactly_one_winner(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let server = Server::start(app.clone()).await;
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();
    let general = general(&t.ada, &ws).await;
    let db = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/databases"),
            json!({"teamspace_id": general, "title": "Tasks"}),
        )
        .await
        .body;
    let db_id = db["page"]["id"].as_str().expect("an id").to_owned();
    t.ada
        .post(
            &format!("/api/v1/databases/{db_id}/properties"),
            json!({"name": "Assignee", "type": "person"}),
        )
        .await;
    let row = t
        .ada
        .post(
            &format!("/api/v1/databases/{db_id}/rows"),
            json!({"values": {"Name": "Popular"}}),
        )
        .await;
    let row = id(&row);
    let mut clients = Vec::new();
    for (owner, name) in [
        (&t.ada, "A1"),
        (&t.bob, "B1"),
        (&t.cy, "C1"),
        (&t.dee, "D1"),
        (&t.ada, "A2"),
        (&t.cy, "C2"),
    ] {
        let (token, agent) = agent_of(owner, &ws, name).await;
        let client = Client::connect(&server, &token).await.expect("connects");
        clients.push((agent, Arc::new(client)));
    }
    let claims: Vec<_> = clients
        .iter()
        .map(|(agent, client)| {
            let client = Arc::clone(client);
            let agent = agent.clone();
            let row = row.clone();
            tokio::spawn(async move {
                (
                    agent,
                    client.call("claim_task", json!({"row_id": row})).await,
                )
            })
        })
        .collect();
    let mut winners = Vec::new();
    let mut holders = Vec::new();
    for claim in claims {
        let (agent, result) = claim.await.expect("the claim ran");
        let structured = result.structured_content.expect("structured content");
        if result.is_error == Some(true) {
            assert_eq!(
                structured["error"]["code"], "already_claimed",
                "{structured}"
            );
            holders.push(structured["error"]["holders"][0]["id"].clone());
        } else {
            winners.push(agent);
        }
    }
    assert_eq!(winners.len(), 1, "{winners:?}");
    assert_eq!(holders.len(), clients.len() - 1);
    assert!(holders.iter().all(|holder| holder == winners[0].as_str()));
    for (_, client) in clients {
        if let Ok(client) = Arc::try_unwrap(client) {
            client.close().await;
        }
    }
    finish(&state, server).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_edit_merges_with_a_page_sync_client_typing(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let server = Server::start(app.clone()).await;
    let t = team(&state, &app).await;
    let ws = t.workspace.clone();
    let plan = page(&t, "Plan", "Typed here\n\nOther text").await;
    let (token, _) = agent_of(&t.ada, &ws, "Scout").await;
    let client = Client::connect(&server, &token).await.expect("connects");
    let mut peer = Peer::join(server.addr, &plan, t.dee.cookie(), "dee").await;
    assert_eq!(peer.text(), "Typed here\nOther text");

    // The person types key after key while the agent edits, neither waiting for the other.
    let edit = client.ok(
        "edit_page",
        json!({"page_id": plan, "operations": [
            {"op": "find_replace", "find": "Other text", "replace": "Changed by the agent"},
            {"op": "append", "markdown": "The agent's line"},
        ]}),
    );
    let typing = peer.type_into_first(" and more");
    let (edited, ()) = tokio::join!(edit, typing);
    assert_eq!(edited["results"][0]["replaced"], 1);
    let both = "Typed here and more\nChanged by the agent\nThe agent's line";
    peer.until_text(both).await;
    eventually("the server to hold both", || async {
        let read = client.ok("read_page", json!({"page_id": plan})).await;
        plain(read["markdown"].as_str().unwrap_or_default())
            == "Typed here and more\n\nChanged by the agent\n\nThe agent's line"
    })
    .await;

    // Typing in the very block an edit changes, elsewhere in its text, stays too.
    let edit = client.ok(
        "edit_page",
        json!({"page_id": plan, "operations": [
            {"op": "find_replace", "find": "Typed", "replace": "Written"},
        ]}),
    );
    let typing = peer.type_into_first("!");
    tokio::join!(edit, typing);
    let both = "Written here and more!\nChanged by the agent\nThe agent's line";
    peer.until_text(both).await;
    eventually("the server to hold both", || async {
        let read = client.ok("read_page", json!({"page_id": plan})).await;
        plain(read["markdown"].as_str().unwrap_or_default()).starts_with("Written here and more!")
    })
    .await;
    drop(peer);
    client.close().await;
    finish(&state, server).await;
}

/// `neon_mcp_operations_total` by operation and status.
fn mcp_counts(
    snapshotter: &metrics_util::debugging::Snapshotter,
) -> std::collections::HashMap<(String, String), u64> {
    use metrics_util::MetricKind;
    use metrics_util::debugging::DebugValue;

    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            let (kind, key) = key.into_parts();
            let (MetricKind::Counter, DebugValue::Counter(count)) = (kind, value) else {
                return None;
            };
            if key.name() != "neon_mcp_operations_total" {
                return None;
            }
            let label = |name: &str| {
                key.labels()
                    .find(|label| label.key() == name)
                    .map(|label| label.value().to_owned())
            };
            Some(((label("operation")?, label("status")?), count))
        })
        .collect()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn each_tool_call_is_a_red_operation_of_its_own(pool: PgPool) {
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    // Tests run on a single-threaded runtime, so the recorder sees every call, the server's
    // tasks' included.
    let _recording = metrics::set_default_local_recorder(&recorder);
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let server = Server::start(app.clone()).await;
    let t = team(&state, &app).await;
    let client = Client::connect(&server, &bound_token(&t.ada, &t.workspace, &["read"]).await)
        .await
        .expect("connects");

    client.ok("whoami", json!({})).await;
    client.ok("whoami", json!({})).await;
    client
        .refused("read_page", json!({"page_id": uuid::Uuid::now_v7()}))
        .await;
    client.service.list_all_resources().await.expect("listed");

    let counts = mcp_counts(&snapshotter);
    let count = |operation: &str, status: &str| {
        counts
            .get(&(operation.to_owned(), status.to_owned()))
            .copied()
            .unwrap_or(0)
    };
    assert_eq!(count("whoami", "ok"), 2);
    // A refusal is the caller's, and counts as ok.
    assert_eq!(count("read_page", "ok"), 1);
    assert_eq!(count("read_page", "error"), 0);
    assert_eq!(count("list_resources", "ok"), 1);
    client.close().await;
    finish(&state, server).await;
}
