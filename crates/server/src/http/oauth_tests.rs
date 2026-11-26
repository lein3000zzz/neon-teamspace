//! OAuth over HTTP: the metadata and CORS, registration, the authorization endpoint and the
//! consent page's requests, the token endpoint with PKCE, code reuse, refresh rotation and
//! reuse detection, revocation, connected apps, and the tokens stopping as access ends.

use std::collections::HashMap;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_HEADERS,
    ACCESS_CONTROL_REQUEST_METHOD, CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN,
    RETRY_AFTER,
};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt as _;
use neon_app::clock::{Clock, ManualTime};
use neon_app::crypto::pkce_challenge;
use neon_app::settings::Change;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tower::ServiceExt as _;
use url::Url;

use super::agents_tests::{Bearer, agent};
use super::auth_tests::{Answer, admin, call};
use super::pages_tests::{create, general, guest};
use super::tests::{PUBLIC_URL, state, state_at};
use super::workspaces_tests::{Browser, Team, id, team};
use super::*;

/// Where the clients of these tests listen: this machine, as a CLI's would.
const REDIRECT: &str = "http://127.0.0.1:43110/callback";

/// A PKCE verifier, low in entropy: tests never hold secrets that look real.
fn verifier(letter: char) -> String {
    letter.to_string().repeat(48)
}

/// The state with `server.public_url` set, its OAuth and auth on `clock`.
async fn public(pool: PgPool, clock: Clock) -> AppState {
    let url = Url::parse(PUBLIC_URL).expect("a URL");
    state_at(pool, WebConfig::new(Some(&url), Vec::new()), clock).await
}

async fn setting(state: &AppState, key: &str, value: Value) {
    state
        .settings
        .update([(key.to_owned(), Change::Set(value))], None)
        .await
        .expect("saved");
}

/// Sends a request with `headers` and a raw body, and reads the answer, JSON or empty.
async fn raw(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: String,
) -> Answer {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body)).expect("a request"))
        .await
        .expect("a response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&bytes)))
    };
    Answer {
        status,
        headers,
        body,
    }
}

/// Posts a form, as OAuth clients do.
async fn form(app: &Router, uri: &str, pairs: &[(&str, &str)]) -> Answer {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    raw(
        app,
        Method::POST,
        uri,
        &[(CONTENT_TYPE.as_str(), "application/x-www-form-urlencoded")],
        body,
    )
    .await
}

/// The OAuth error code of an answer in OAuth's shape.
fn oauth_error(answer: &Answer) -> &str {
    answer.body["error"]
        .as_str()
        .unwrap_or_else(|| panic!("not OAuth's error shape: {}", answer.body))
}

/// Registers a client that comes back to `redirects`: its id.
async fn register(app: &Router, redirects: &[&str]) -> String {
    let made = call(
        app,
        Method::POST,
        "/api/v1/oauth/register",
        &[],
        Some(json!({"client_name": "Claude Code", "redirect_uris": redirects})),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    made.body["client_id"]
        .as_str()
        .expect("a client id")
        .to_owned()
}

/// An authorization request of `client` with the challenge of `verifier`, as the consent
/// page sends it.
fn request(client: &str, verifier: &str, scope: Option<&str>) -> Value {
    let mut request = json!({
        "response_type": "code",
        "client_id": client,
        "redirect_uri": REDIRECT,
        "code_challenge": pkce_challenge(verifier),
        "code_challenge_method": "S256",
        "state": "opaque-state",
    });
    if let Some(scope) = scope {
        request["scope"] = json!(scope);
    }
    request
}

/// The query of an authorization request.
fn query(request: &Value) -> String {
    let pairs: Vec<(String, String)> = request
        .as_object()
        .expect("an object")
        .iter()
        .map(|(key, value)| (key.clone(), value.as_str().expect("text").to_owned()))
        .collect();
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// The parameters of an address's query.
fn params(address: &str) -> HashMap<String, String> {
    let url = Url::parse(address)
        .or_else(|_| Url::parse(&format!("https://neon.example.com{address}")))
        .expect("an address");
    url.query_pairs().into_owned().collect()
}

/// The person's approval of `request`, acting as `agent` or themselves: where the browser
/// goes next.
async fn approve(
    by: &Browser<'_>,
    request: &Value,
    workspace: &str,
    agent: Option<&str>,
    scopes: &[&str],
) -> Answer {
    let mut body = json!({"request": request, "workspace_id": workspace, "scopes": scopes});
    if let Some(agent) = agent {
        body["agent_id"] = json!(agent);
    }
    by.post("/api/v1/oauth/consent/approve", body).await
}

/// The code an approval sends the browser back with.
async fn code(
    by: &Browser<'_>,
    request: &Value,
    workspace: &str,
    agent: Option<&str>,
    scopes: &[&str],
) -> String {
    let approved = approve(by, request, workspace, agent, scopes).await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    let back = approved.body["redirect_to"].as_str().expect("an address");
    assert!(back.starts_with(REDIRECT), "{back}");
    params(back)["code"].clone()
}

async fn exchange(app: &Router, client: &str, code: &str, verifier: &str) -> Answer {
    form(
        app,
        "/api/v1/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier),
        ],
    )
    .await
}

async fn refresh(app: &Router, client: &str, token: &str, scope: Option<&str>) -> Answer {
    let mut pairs = vec![
        ("grant_type", "refresh_token"),
        ("client_id", client),
        ("refresh_token", token),
    ];
    if let Some(scope) = scope {
        pairs.push(("scope", scope));
    }
    form(app, "/api/v1/oauth/token", &pairs).await
}

/// A client connected end to end, and its tokens.
struct Connected {
    client: String,
    access: String,
    refresh: String,
}

impl Connected {
    fn bearer<'a>(&self, app: &'a Router) -> Bearer<'a> {
        Bearer::new(app, format!("Bearer {}", self.access))
    }
}

fn tokens(client: &str, answer: &Answer) -> Connected {
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    Connected {
        client: client.to_owned(),
        access: answer.body["access_token"]
            .as_str()
            .expect("an access token")
            .to_owned(),
        refresh: answer.body["refresh_token"]
            .as_str()
            .expect("a refresh token")
            .to_owned(),
    }
}

/// Registers a client, has `by` approve it in `workspace` as `agent` or themselves, and
/// exchanges the code.
async fn connect(
    app: &Router,
    by: &Browser<'_>,
    workspace: &str,
    agent: Option<&str>,
    scopes: &[&str],
) -> Connected {
    let client = register(app, &[REDIRECT]).await;
    let verifier = verifier('v');
    let request = request(&client, &verifier, None);
    let code = code(by, &request, workspace, agent, scopes).await;
    tokens(&client, &exchange(app, &client, &code, &verifier).await)
}

/// A user's id, as their `/account/me` says.
async fn me(by: &Browser<'_>) -> String {
    id(&by.get("/api/v1/account/me").await)
}

async fn works(app: &Router, connected: &Connected, workspace: &str) -> StatusCode {
    connected
        .bearer(app)
        .get(&format!("/api/v1/workspaces/{workspace}/members"))
        .await
        .status
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_metadata_names_the_endpoints_below_the_public_url(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);

    let server = call(
        &app,
        Method::GET,
        "/.well-known/oauth-authorization-server",
        &[],
        None,
    )
    .await;
    assert_eq!(server.status, StatusCode::OK, "{}", server.body);
    assert_eq!(
        server.body,
        json!({
            "issuer": "https://neon.example.com",
            "authorization_endpoint": "https://neon.example.com/api/v1/oauth/authorize",
            "token_endpoint": "https://neon.example.com/api/v1/oauth/token",
            "registration_endpoint": "https://neon.example.com/api/v1/oauth/register",
            "revocation_endpoint": "https://neon.example.com/api/v1/oauth/revoke",
            "scopes_supported": ["read", "write", "admin"],
            "response_types_supported": ["code"],
            "response_modes_supported": ["query"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "revocation_endpoint_auth_methods_supported": ["none"],
            "authorization_response_iss_parameter_supported": true,
            "service_documentation": "https://neon.example.com/docs/guides/oauth/",
        })
    );
    assert_eq!(server.headers[CACHE_CONTROL], "public, max-age=3600");

    for (path, resource, name) in [
        (
            "/.well-known/oauth-protected-resource",
            "https://neon.example.com/mcp",
            "Neon Teamspace MCP",
        ),
        (
            "/.well-known/oauth-protected-resource/mcp",
            "https://neon.example.com/mcp",
            "Neon Teamspace MCP",
        ),
        (
            "/.well-known/oauth-protected-resource/api/v1",
            "https://neon.example.com/api/v1",
            "Neon Teamspace API",
        ),
    ] {
        let resource_metadata = call(&app, Method::GET, path, &[], None).await;
        assert_eq!(resource_metadata.status, StatusCode::OK, "{path}");
        assert_eq!(
            resource_metadata.body,
            json!({
                "resource": resource,
                "authorization_servers": ["https://neon.example.com"],
                "scopes_supported": ["read", "write", "admin"],
                "bearer_methods_supported": ["header"],
                "resource_name": name,
                "resource_documentation": "https://neon.example.com/docs/guides/oauth/",
            }),
            "{path}"
        );
    }

    // With registration off, the metadata names no registration endpoint.
    setting(&state, "oauth.registration", json!(false)).await;
    let closed = call(
        &app,
        Method::GET,
        "/.well-known/oauth-authorization-server",
        &[],
        None,
    )
    .await;
    assert_eq!(closed.body.get("registration_endpoint"), None);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn without_a_public_url_the_metadata_names_the_host_asked(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state, false);

    let server = call(
        &app,
        Method::GET,
        "/.well-known/oauth-authorization-server",
        &[(HOST.as_str(), "127.0.0.1:3000")],
        None,
    )
    .await;
    assert_eq!(server.body["issuer"], "http://127.0.0.1:3000");
    assert_eq!(
        server.body["token_endpoint"],
        "http://127.0.0.1:3000/api/v1/oauth/token"
    );
    let resource = call(
        &app,
        Method::GET,
        "/.well-known/oauth-protected-resource",
        &[(HOST.as_str(), "127.0.0.1:3000")],
        None,
    )
    .await;
    assert_eq!(resource.body["resource"], "http://127.0.0.1:3000/mcp");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn cors_opens_the_clients_endpoints_alone(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state, false);
    let elsewhere = (ORIGIN.as_str(), "https://inspector.example");

    for path in [
        "/.well-known/oauth-authorization-server",
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let answer = call(&app, Method::GET, path, &[elsewhere], None).await;
        assert_eq!(answer.status, StatusCode::OK, "{path}");
        assert_eq!(answer.headers[ACCESS_CONTROL_ALLOW_ORIGIN], "*", "{path}");
    }
    for path in [
        "/api/v1/oauth/register",
        "/api/v1/oauth/token",
        "/api/v1/oauth/revoke",
        "/.well-known/oauth-protected-resource",
    ] {
        let preflight = raw(
            &app,
            Method::OPTIONS,
            path,
            &[
                elsewhere,
                (ACCESS_CONTROL_REQUEST_METHOD.as_str(), "POST"),
                (
                    ACCESS_CONTROL_REQUEST_HEADERS.as_str(),
                    "content-type, mcp-protocol-version",
                ),
            ],
            String::new(),
        )
        .await;
        assert_eq!(preflight.status, StatusCode::NO_CONTENT, "{path}");
        assert_eq!(
            preflight.headers[ACCESS_CONTROL_ALLOW_ORIGIN], "*",
            "{path}"
        );
        let allowed = preflight.headers[ACCESS_CONTROL_ALLOW_HEADERS]
            .to_str()
            .expect("text");
        assert!(
            allowed.contains("mcp-protocol-version"),
            "{path}: {allowed}"
        );
    }

    // Everything else stays closed to other origins.
    for (method, path) in [
        (Method::GET, "/api/v1/meta"),
        (Method::GET, "/api/v1/oauth/authorize"),
        (Method::OPTIONS, "/api/v1/workspaces"),
        (Method::OPTIONS, "/api/v1/oauth/consent"),
        (Method::OPTIONS, "/api/v1/account/connected-apps"),
    ] {
        let answer = raw(
            &app,
            method.clone(),
            path,
            &[elsewhere, (ACCESS_CONTROL_REQUEST_METHOD.as_str(), "POST")],
            String::new(),
        )
        .await;
        assert_eq!(
            answer.headers.get(ACCESS_CONTROL_ALLOW_ORIGIN),
            None,
            "{method} {path}"
        );
    }
    // A cross-origin post carrying a cookie to an API route is still refused.
    let refused = call(
        &app,
        Method::POST,
        "/api/v1/oauth/consent",
        &[elsewhere, (COOKIE.as_str(), "neon_session=x")],
        Some(json!({})),
    )
    .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn clients_register_with_strict_redirect_uris(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let app = &app;
    let registration = |body: Value| async move {
        call(
            app,
            Method::POST,
            "/api/v1/oauth/register",
            &[(ORIGIN.as_str(), "https://inspector.example")],
            Some(body),
        )
        .await
    };

    let made = registration(json!({
        "client_name": "  Claude\u{202e} Code ",
        "redirect_uris": [REDIRECT, "https://app.example.com/cb"],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "client_secret_basic",
        "logo_uri": "https://app.example.com/logo.png",
    }))
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["client_name"], "Claude Code");
    assert_eq!(
        made.body["redirect_uris"],
        json!([REDIRECT, "https://app.example.com/cb"])
    );
    assert_eq!(made.body["token_endpoint_auth_method"], "none");
    assert_eq!(
        made.body["grant_types"],
        json!(["authorization_code", "refresh_token"])
    );
    assert!(made.body["client_id"].as_str().is_some());
    assert!(made.body["client_id_issued_at"].as_i64().is_some());
    assert_eq!(made.body.get("client_secret"), None);

    for (body, code) in [
        (json!({"redirect_uris": []}), "invalid_redirect_uri"),
        (json!({}), "invalid_redirect_uri"),
        (
            json!({"redirect_uris": ["http://app.example.com/cb"]}),
            "invalid_redirect_uri",
        ),
        (
            json!({"redirect_uris": ["https://app.example.com/cb#x"]}),
            "invalid_redirect_uri",
        ),
        (
            json!({"redirect_uris": ["javascript:alert(1)"]}),
            "invalid_redirect_uri",
        ),
        (
            json!({"redirect_uris": [REDIRECT], "grant_types": ["client_credentials"]}),
            "invalid_client_metadata",
        ),
        (
            json!({"redirect_uris": [REDIRECT], "response_types": ["token"]}),
            "invalid_client_metadata",
        ),
        (json!({"redirect_uris": "nope"}), "invalid_client_metadata"),
    ] {
        let refused = registration(body.clone()).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(oauth_error(&refused), code, "{body}");
    }
    let too_big = raw(
        app,
        Method::POST,
        "/api/v1/oauth/register",
        &[(CONTENT_TYPE.as_str(), "application/json")],
        format!(r#"{{"client_name": "{}"}}"#, "n".repeat(70 * 1024)),
    )
    .await;
    assert_eq!(too_big.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(oauth_error(&too_big), "invalid_client_metadata");

    // A client registers a few at a time from one address.
    let limited = loop {
        let answer = registration(json!({"redirect_uris": [REDIRECT]})).await;
        if answer.status != StatusCode::CREATED {
            break answer;
        }
    };
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(oauth_error(&limited), "rate_limited");
    assert!(limited.headers.contains_key(RETRY_AFTER));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn registration_switches_off_and_waits_while_too_many_are_unconnected(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);

    setting(&state, "oauth.max_unused_clients_per_source", json!(1)).await;
    register(&app, &[REDIRECT]).await;
    let theirs_full = call(
        &app,
        Method::POST,
        "/api/v1/oauth/register",
        &[],
        Some(json!({"redirect_uris": [REDIRECT]})),
    )
    .await;
    assert_eq!(
        theirs_full.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        theirs_full.body
    );
    assert_eq!(oauth_error(&theirs_full), "too_many_clients");

    setting(&state, "oauth.max_unused_clients_per_source", json!(10)).await;
    setting(&state, "oauth.max_unused_clients", json!(1)).await;
    let full = call(
        &app,
        Method::POST,
        "/api/v1/oauth/register",
        &[],
        Some(json!({"redirect_uris": [REDIRECT]})),
    )
    .await;
    assert_eq!(
        full.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        full.body
    );
    assert_eq!(oauth_error(&full), "temporarily_unavailable");

    setting(&state, "oauth.registration", json!(false)).await;
    let off = call(
        &app,
        Method::POST,
        "/api/v1/oauth/register",
        &[],
        Some(json!({"redirect_uris": [REDIRECT]})),
    )
    .await;
    assert_eq!(off.status, StatusCode::FORBIDDEN);
    assert_eq!(oauth_error(&off), "registration_off");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_authorization_endpoint_sends_the_browser_to_consent(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state, false);
    let client = register(&app, &[REDIRECT, "https://app.example.com/cb"]).await;
    let verifier = verifier('v');
    let good = request(&client, &verifier, Some("read write"));

    let sent = call(
        &app,
        Method::GET,
        &format!("/api/v1/oauth/authorize?{}&prompt=consent", query(&good)),
        &[],
        None,
    )
    .await;
    assert_eq!(sent.status, StatusCode::FOUND);
    let location = sent.headers[LOCATION].to_str().expect("text");
    assert!(location.starts_with("/oauth/consent?"), "{location}");
    let passed = params(location);
    assert_eq!(passed["client_id"], client);
    assert_eq!(passed["redirect_uri"], REDIRECT);
    assert_eq!(passed["state"], "opaque-state");
    assert_eq!(passed["scope"], "read write");
    assert_eq!(passed.get("prompt"), None);

    // A loopback redirect on another port is the one registered.
    let mut other_port = good.clone();
    other_port["redirect_uri"] = json!("http://127.0.0.1:51004/callback");
    let mut duplicated = query(&good);
    duplicated.push_str("&scope=admin");
    let mut problems = vec![(duplicated, "duplicate_parameter")];
    let mut with = |key: &str, value: Option<&str>, problem: &'static str| {
        let mut changed = good.clone();
        match value {
            Some(value) => changed[key] = json!(value),
            None => {
                changed.as_object_mut().expect("an object").remove(key);
            }
        }
        problems.push((query(&changed), problem));
    };
    with("client_id", None, "missing_client_id");
    with("client_id", Some("claude"), "unknown_client");
    with(
        "client_id",
        Some("00000000-0000-7000-8000-000000000001"),
        "unknown_client",
    );
    with("redirect_uri", None, "missing_redirect_uri");
    with(
        "redirect_uri",
        Some("https://evil.example/cb"),
        "redirect_uri_mismatch",
    );
    with(
        "redirect_uri",
        Some("https://app.example.com/cb/"),
        "redirect_uri_mismatch",
    );
    with("response_type", Some("token"), "unsupported_response_type");
    with("response_type", None, "unsupported_response_type");
    with("code_challenge_method", Some("plain"), "pkce_required");
    with("code_challenge_method", None, "pkce_required");
    with("code_challenge", None, "pkce_required");
    with("code_challenge", Some("short"), "invalid_code_challenge");
    with("scope", Some("read sudo"), "invalid_scope");
    with("state", Some("tab\tbreaks"), "invalid_state");
    with(
        "resource",
        Some("https://evil.example/mcp"),
        "invalid_target",
    );
    for (asked, problem) in problems {
        let refused = call(
            &app,
            Method::GET,
            &format!("/api/v1/oauth/authorize?{asked}"),
            &[],
            None,
        )
        .await;
        assert_eq!(refused.status, StatusCode::FOUND, "{problem}");
        let location = refused.headers[LOCATION].to_str().expect("text");
        assert_eq!(
            location,
            format!("/oauth/consent?error={problem}"),
            "the browser never goes to the client with a problem"
        );
    }
    let fine = call(
        &app,
        Method::GET,
        &format!("/api/v1/oauth/authorize?{}", query(&other_port)),
        &[],
        None,
    )
    .await;
    assert!(
        fine.headers[LOCATION]
            .to_str()
            .expect("text")
            .contains("client_id=")
    );
    let api = call(
        &app,
        Method::GET,
        &format!(
            "/api/v1/oauth/authorize?{}&resource={}",
            query(&good),
            "https%3A%2F%2Fneon.example.com%2Fapi%2Fv1"
        ),
        &[],
        None,
    )
    .await;
    assert!(
        api.headers[LOCATION]
            .to_str()
            .expect("text")
            .contains("resource=")
    );
}

/// The team, Cy with an agent, Scout, and a registered client's request.
async fn setting_up<'a>(state: &AppState, app: &'a Router) -> (Team<'a>, String) {
    let team = team(state, app).await;
    let scout = agent(&team.cy, &team.workspace, "Scout").await;
    (team, scout)
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn consent_offers_identities_as_the_settings_allow_and_scopes_as_the_role_does(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let (team, scout) = setting_up(&state, &app).await;
    let ws = &team.workspace;
    let client = register(&app, &[REDIRECT]).await;
    let asked = request(&client, &verifier('v'), Some("read write"));

    let cy = team.cy.post("/api/v1/oauth/consent", asked.clone()).await;
    assert_eq!(cy.status, StatusCode::OK, "{}", cy.body);
    assert_eq!(cy.body["client"]["name"], "Claude Code");
    assert_eq!(cy.body["redirect_host"], "http://127.0.0.1:43110");
    assert_eq!(cy.body["requested_scopes"], json!(["read", "write"]));
    assert_eq!(cy.body["resource"], "mcp");
    let design = &cy.body["workspaces"][0];
    assert_eq!(design["id"], ws.as_str());
    assert_eq!(design["role"], "member");
    assert_eq!(design["person_scopes"], json!(["read", "write"]));
    assert_eq!(design["agents_on"], true);
    assert_eq!(design["agents"][0]["id"], scout.as_str());
    assert_eq!(design["agents"][0]["status"], "active");
    assert_eq!(design["agent_scopes"], json!(["read", "write"]));

    // Admin only for owners and admins acting as themselves; nobody else's agents.
    for (who, scopes) in [
        (&team.ada, json!(["read", "write", "admin"])),
        (&team.bob, json!(["read", "write", "admin"])),
        (&team.dee, json!(["read", "write"])),
    ] {
        let offered = who.post("/api/v1/oauth/consent", asked.clone()).await;
        assert_eq!(offered.body["workspaces"][0]["person_scopes"], scopes);
        assert_eq!(offered.body["workspaces"][0]["agents"], json!([]));
    }

    // A guest connects as themselves, never as an agent.
    user(&state, "gus@example.com").await;
    let gus = guest(&app, &team.ada, ws, "gus@example.com").await;
    let offered = gus.post("/api/v1/oauth/consent", asked.clone()).await;
    assert_eq!(offered.body["workspaces"][0]["role"], "guest");
    assert_eq!(
        offered.body["workspaces"][0]["person_scopes"],
        json!(["read", "write"])
    );
    assert_eq!(offered.body["workspaces"][0]["agents_on"], false);

    // mcp.identities and the workspace's switch decide the identities.
    setting(&state, "mcp.identities", json!(["agent"])).await;
    let agents_only = team.cy.post("/api/v1/oauth/consent", asked.clone()).await;
    assert_eq!(
        agents_only.body["workspaces"][0]["person_scopes"],
        Value::Null
    );
    assert_eq!(agents_only.body["workspaces"][0]["agents_on"], true);
    let refused = approve(&team.cy, &asked, ws, None, &["read"]).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.code(), "identity_unavailable");
    setting(&state, "mcp.identities", json!(["user"])).await;
    let people_only = team.cy.post("/api/v1/oauth/consent", asked.clone()).await;
    assert_eq!(people_only.body["workspaces"][0]["agents_on"], false);
    assert_eq!(people_only.body["workspaces"][0]["agents"], json!([]));
    let refused = approve(&team.cy, &asked, ws, Some(&scout), &["read"]).await;
    assert_eq!(refused.code(), "identity_unavailable");
    setting(&state, "mcp.identities", json!(["user", "agent"])).await;
    let off = team
        .bob
        .send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{ws}"),
            Some(json!({"agents_enabled": false})),
        )
        .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.body);
    let switched = team.cy.post("/api/v1/oauth/consent", asked.clone()).await;
    assert_eq!(switched.body["workspaces"][0]["agents_on"], false);
    assert_eq!(
        switched.body["workspaces"][0]["person_scopes"],
        json!(["read", "write"])
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn approving_takes_a_session_and_only_what_is_offered(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let (team, scout) = setting_up(&state, &app).await;
    let ws = &team.workspace;
    let client = register(&app, &[REDIRECT]).await;
    let asked = request(&client, &verifier('v'), None);

    // Signed out, with a token, or from another origin: refused.
    let signed_out = call(
        &app,
        Method::POST,
        "/api/v1/oauth/consent",
        &[],
        Some(asked.clone()),
    )
    .await;
    assert_eq!(signed_out.status, StatusCode::UNAUTHORIZED);
    let token = team.cy.token(&["read", "write"]).await;
    let bearer = Bearer::new(&app, token);
    for (path, body) in [
        ("/api/v1/oauth/consent", asked.clone()),
        ("/api/v1/oauth/consent/deny", json!({"request": asked})),
        (
            "/api/v1/oauth/consent/approve",
            json!({"request": asked, "workspace_id": ws, "scopes": ["read"]}),
        ),
    ] {
        let by_token = bearer.post(path, body).await;
        assert_eq!(
            by_token.status,
            StatusCode::FORBIDDEN,
            "{path}: {}",
            by_token.body
        );
    }
    let forged = call(
        &app,
        Method::POST,
        "/api/v1/oauth/consent/approve",
        &[
            (COOKIE.as_str(), team.cy.cookie()),
            (ORIGIN.as_str(), "https://evil.example"),
        ],
        Some(json!({"request": asked, "workspace_id": ws, "scopes": ["read"]})),
    )
    .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);

    // Scopes among those offered, of a workspace they are in, as an agent of theirs.
    for (agent, scopes, status, code) in [
        (None, vec![], StatusCode::BAD_REQUEST, "invalid_scope"),
        (
            None,
            vec!["admin"],
            StatusCode::BAD_REQUEST,
            "invalid_scope",
        ),
        (None, vec!["sudo"], StatusCode::BAD_REQUEST, "invalid_scope"),
        (
            Some(scout.as_str()),
            vec!["read", "admin"],
            StatusCode::BAD_REQUEST,
            "invalid_scope",
        ),
    ] {
        let refused = approve(&team.cy, &asked, ws, agent, &scopes).await;
        assert_eq!(refused.status, status, "{scopes:?}: {}", refused.body);
        assert_eq!(refused.code(), code);
    }
    let not_theirs = approve(&team.dee, &asked, ws, Some(&scout), &["read"]).await;
    assert_eq!(
        not_theirs.status,
        StatusCode::FORBIDDEN,
        "{}",
        not_theirs.body
    );
    let elsewhere = approve(
        &team.cy,
        &asked,
        "00000000-0000-7000-8000-000000000001",
        None,
        &["read"],
    )
    .await;
    assert_eq!(elsewhere.status, StatusCode::NOT_FOUND);
    let no_agent = approve(
        &team.cy,
        &asked,
        ws,
        Some("00000000-0000-7000-8000-000000000001"),
        &["read"],
    )
    .await;
    assert_eq!(no_agent.status, StatusCode::NOT_FOUND);
    let mut tampered = asked.clone();
    tampered["redirect_uri"] = json!("https://evil.example/cb");
    let refused = approve(&team.cy, &tampered, ws, None, &["read"]).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(refused.code(), "redirect_uri_mismatch");

    // Denying sends the browser back with access_denied.
    let denied = team
        .cy
        .post("/api/v1/oauth/consent/deny", json!({"request": asked}))
        .await;
    assert_eq!(denied.status, StatusCode::OK, "{}", denied.body);
    let back = denied.body["redirect_to"].as_str().expect("an address");
    assert!(back.starts_with(REDIRECT), "{back}");
    let answer = params(back);
    assert_eq!(answer["error"], "access_denied");
    assert_eq!(answer["state"], "opaque-state");
    assert_eq!(answer["iss"], "https://neon.example.com");
    assert_eq!(answer.get("code"), None);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_person_connects_and_the_token_works_in_their_workspace_alone(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    admin(&state).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let elsewhere = id(&team
        .dee
        .post("/api/v1/workspaces", json!({"name": "Elsewhere"}))
        .await);
    let client = register(&app, &[REDIRECT]).await;
    let verifier = verifier('v');
    let asked = request(&client, &verifier, Some("read write"));

    let approved = approve(&team.cy, &asked, ws, None, &["write", "read"]).await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    let back = params(approved.body["redirect_to"].as_str().expect("an address"));
    assert_eq!(back["state"], "opaque-state");
    assert_eq!(back["iss"], "https://neon.example.com");
    let issued = exchange(&app, &client, &back["code"], &verifier).await;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.body);
    assert_eq!(issued.headers[CACHE_CONTROL], "no-store");
    assert_eq!(issued.body["token_type"], "Bearer");
    assert_eq!(issued.body["expires_in"], 3600);
    assert_eq!(issued.body["scope"], "read write");
    let connected = tokens(&client, &issued);
    assert!(connected.access.starts_with("nwo_"));
    assert!(connected.refresh.starts_with("nwr_"));
    assert_ne!(connected.access, connected.refresh);

    // It acts as Cy, in Design alone, with its scopes.
    let bearer = connected.bearer(&app);
    let cy_id = me(&team.cy).await;
    let page = create_with(&bearer, ws, &general(&team.cy, ws).await, "From the client").await;
    let read = team.cy.get(&format!("/api/v1/pages/{page}")).await;
    assert_eq!(read.body["created_by"], cy_id.as_str(), "{}", read.body);
    assert_eq!(works(&app, &connected, ws).await, StatusCode::OK);
    assert_eq!(
        bearer
            .get(&format!("/api/v1/workspaces/{elsewhere}"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    for path in [
        "/api/v1/account/me",
        "/api/v1/account/tokens",
        "/api/v1/account/connected-apps",
        "/api/v1/admin/users",
    ] {
        let refused = bearer.get(path).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(refused.code(), "token_bound", "{path}");
    }
    // The refresh token is no bearer credential, nor is anything with another prefix.
    for header in [
        format!("Bearer {}", connected.refresh),
        format!("Bearer nwo_{}", "x".repeat(43)),
    ] {
        let refused = Bearer::new(&app, header)
            .get(&format!("/api/v1/workspaces/{ws}/members"))
            .await;
        assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    }

    // Read alone writes nothing.
    let reader = connect(&app, &team.cy, ws, None, &["read"]).await;
    let refused = reader
        .bearer(&app)
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({"title": "No", "teamspace_id": general(&team.cy, ws).await}),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.code(), "insufficient_scope");

    // An admin's admin scope manages the workspace; without it, they manage nothing.
    let admin_app = connect(&app, &team.bob, ws, None, &["read", "write", "admin"]).await;
    let writer_app = connect(&app, &team.bob, ws, None, &["read", "write"]).await;
    let rename = json!({"name": "Design studio"});
    let path = format!("/api/v1/workspaces/{ws}");
    let renamed = admin_app
        .bearer(&app)
        .send(Method::PATCH, &path, Some(rename.clone()))
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    let refused = writer_app
        .bearer(&app)
        .send(Method::PATCH, &path, Some(rename))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

/// Makes a page in a teamspace through a bearer client.
async fn create_with(by: &Bearer<'_>, workspace: &str, teamspace: &str, title: &str) -> String {
    let made = by
        .post(
            &format!("/api/v1/workspaces/{workspace}/pages"),
            json!({"title": title, "teamspace_id": teamspace}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    id(&made)
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_connects_and_acts_as_itself_within_its_owners_reach(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let (team, scout) = setting_up(&state, &app).await;
    let ws = &team.workspace;

    let connected = connect(&app, &team.cy, ws, Some(&scout), &["read", "write"]).await;
    let bearer = connected.bearer(&app);
    let general = general(&team.cy, ws).await;
    let page = create_with(&bearer, ws, &general, "Scout's notes").await;
    let read = team.cy.get(&format!("/api/v1/pages/{page}")).await;
    assert_eq!(read.body["created_by"], scout.as_str(), "{}", read.body);

    // It manages nothing, its owner's admin scope or not.
    let refused = bearer
        .send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{ws}"),
            Some(json!({"name": "Mine"})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    // An agent's page shared with nobody else stays out of its reach when its owner's is.
    let private = create(&team.ada, ws, json!({"private": true}), "Ada's").await;
    assert_eq!(
        bearer.get(&format!("/api/v1/pages/{private}")).await.status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn codes_work_once_and_a_second_use_revokes_everything_issued(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let client = register(&app, &[REDIRECT]).await;
    let verifier = verifier('v');
    let asked = request(&client, &verifier, None);
    let code = code(&team.cy, &asked, ws, None, &["read", "write"]).await;

    let first = tokens(&client, &exchange(&app, &client, &code, &verifier).await);
    assert_eq!(works(&app, &first, ws).await, StatusCode::OK);
    let second = exchange(&app, &client, &code, &verifier).await;
    assert_eq!(second.status, StatusCode::BAD_REQUEST);
    assert_eq!(oauth_error(&second), "invalid_grant");
    assert!(
        second.body["error_description"]
            .as_str()
            .expect("text")
            .contains("used already")
    );
    assert_eq!(works(&app, &first, ws).await, StatusCode::UNAUTHORIZED);
    let refreshed = refresh(&app, &client, &first.refresh, None).await;
    assert_eq!(oauth_error(&refreshed), "invalid_grant");
    let apps = team.cy.get("/api/v1/account/connected-apps").await;
    assert_eq!(apps.body["apps"], json!([]), "the grant is gone");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_code_needs_its_verifier_redirect_and_client_and_a_mistake_spends_nothing(pool: PgPool) {
    let time = ManualTime::new(OffsetDateTime::now_utc());
    let state = public(pool, time.clock()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let client = register(&app, &[REDIRECT, "http://127.0.0.1/other"]).await;
    let other = register(&app, &[REDIRECT]).await;
    let verifier = verifier('v');
    let asked = request(&client, &verifier, None);
    let code = code(&team.cy, &asked, ws, None, &["read"]).await;

    for (pairs, error) in [
        (
            vec![
                ("grant_type", "authorization_code"),
                ("client_id", client.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
                ("code_verifier", &self::verifier('w')),
            ],
            "invalid_grant",
        ),
        (
            vec![
                ("grant_type", "authorization_code"),
                ("client_id", client.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
                ("code_verifier", "too-short"),
            ],
            "invalid_grant",
        ),
        (
            vec![
                ("grant_type", "authorization_code"),
                ("client_id", client.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", "http://127.0.0.1/other"),
                ("code_verifier", verifier.as_str()),
            ],
            "invalid_grant",
        ),
        (
            vec![
                ("grant_type", "authorization_code"),
                ("client_id", other.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
                ("code_verifier", verifier.as_str()),
            ],
            "invalid_grant",
        ),
        (
            vec![
                ("grant_type", "authorization_code"),
                ("client_id", client.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
                ("code_verifier", verifier.as_str()),
                ("resource", "https://neon.example.com/api/v1"),
            ],
            "invalid_target",
        ),
        (
            vec![
                ("grant_type", "authorization_code"),
                ("client_id", client.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
            ],
            "invalid_request",
        ),
        (
            vec![("grant_type", "password"), ("client_id", client.as_str())],
            "unsupported_grant_type",
        ),
        (vec![("client_id", client.as_str())], "invalid_request"),
        (
            vec![
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
            ],
            "invalid_request",
        ),
        (
            vec![
                ("grant_type", "authorization_code"),
                ("client_id", "00000000-0000-7000-8000-000000000001"),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
                ("code_verifier", verifier.as_str()),
            ],
            "invalid_client",
        ),
        (
            vec![
                ("grant_type", "authorization_code"),
                ("client_id", client.as_str()),
                ("code", "not-a-code"),
                ("redirect_uri", REDIRECT),
                ("code_verifier", verifier.as_str()),
            ],
            "invalid_grant",
        ),
    ] {
        let refused = form(&app, "/api/v1/oauth/token", &pairs).await;
        assert!(refused.status.is_client_error(), "{pairs:?}");
        assert_eq!(oauth_error(&refused), error, "{pairs:?}: {}", refused.body);
    }
    let unauthorized = form(
        &app,
        "/api/v1/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("client_id", "00000000-0000-7000-8000-000000000001"),
        ],
    )
    .await;
    assert_eq!(unauthorized.status, StatusCode::UNAUTHORIZED);
    let json = call(
        &app,
        Method::POST,
        "/api/v1/oauth/token",
        &[],
        Some(json!({"grant_type": "authorization_code"})),
    )
    .await;
    assert_eq!(oauth_error(&json), "invalid_request");

    // None of those spent it.
    let issued = exchange(&app, &client, &code, &verifier).await;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.body);

    // A code works for a minute.
    let late = self::code(&team.cy, &asked, ws, None, &["read"]).await;
    time.advance(Duration::from_secs(60));
    let expired = exchange(&app, &client, &late, &verifier).await;
    assert_eq!(oauth_error(&expired), "invalid_grant");
    assert!(
        expired.body["error_description"]
            .as_str()
            .expect("text")
            .contains("expired")
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_code_keeps_to_its_own_resource_and_a_refresh_to_the_grants(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let client = register(&app, &[REDIRECT]).await;
    let verifier = verifier('v');
    const API: &str = "https://neon.example.com/api/v1";
    const MCP: &str = "https://neon.example.com/mcp";
    let mut for_api = request(&client, &verifier, None);
    for_api["resource"] = json!(API);
    let api_code = code(&team.cy, &for_api, ws, None, &["read"]).await;
    // Allowed again for the MCP endpoint, the same client, workspace and identity.
    let mcp_code = code(
        &team.cy,
        &request(&client, &verifier, None),
        ws,
        None,
        &["read"],
    )
    .await;
    let exchange_for = |code: String, resource: &'static str| {
        let (app, client, verifier) = (&app, &client, &verifier);
        async move {
            form(
                app,
                "/api/v1/oauth/token",
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", client),
                    ("code", &code),
                    ("redirect_uri", REDIRECT),
                    ("code_verifier", verifier),
                    ("resource", resource),
                ],
            )
            .await
        }
    };

    let elsewhere = exchange_for(api_code.clone(), MCP).await;
    assert_eq!(
        oauth_error(&elsewhere),
        "invalid_target",
        "{}",
        elsewhere.body
    );
    let elsewhere = exchange_for(mcp_code.clone(), API).await;
    assert_eq!(
        oauth_error(&elsewhere),
        "invalid_target",
        "{}",
        elsewhere.body
    );
    let api = tokens(&client, &exchange_for(api_code, API).await);
    tokens(&client, &exchange_for(mcp_code, MCP).await);

    // A refresh keeps to the resource the grant was last allowed for.
    let refresh_for = |resource: &'static str| {
        let (app, client, token) = (&app, &client, &api.refresh);
        async move {
            form(
                app,
                "/api/v1/oauth/token",
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", client),
                    ("refresh_token", token),
                    ("resource", resource),
                ],
            )
            .await
        }
    };
    assert_eq!(oauth_error(&refresh_for(API).await), "invalid_target");
    let refreshed = refresh_for(MCP).await;
    assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.body);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn refresh_tokens_rotate_and_a_reused_one_revokes_the_whole_grant(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let first = connect(&app, &team.cy, ws, None, &["read", "write"]).await;

    let rotated = refresh(&app, &first.client, &first.refresh, None).await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.body);
    assert_eq!(rotated.body["scope"], "read write");
    let second = tokens(&first.client, &rotated);
    assert_ne!(second.refresh, first.refresh);
    assert_ne!(second.access, first.access);
    assert_eq!(works(&app, &second, ws).await, StatusCode::OK);
    let third = tokens(
        &first.client,
        &refresh(&app, &first.client, &second.refresh, None).await,
    );
    assert_eq!(works(&app, &third, ws).await, StatusCode::OK);

    // The first refresh token again: a copy is about, and the whole family ends.
    let reused = refresh(&app, &first.client, &first.refresh, None).await;
    assert_eq!(reused.status, StatusCode::BAD_REQUEST);
    assert_eq!(oauth_error(&reused), "invalid_grant");
    for tokens in [&first, &second, &third] {
        assert_eq!(works(&app, tokens, ws).await, StatusCode::UNAUTHORIZED);
    }
    let after = refresh(&app, &first.client, &third.refresh, None).await;
    assert_eq!(oauth_error(&after), "invalid_grant");
}

/// A form of `count` parameters with distinct names and empty values.
fn distinct_keys(count: usize) -> String {
    (0..count)
        .map(|n| format!("k{n}="))
        .collect::<Vec<_>>()
        .join("&")
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn forms_are_read_only_as_large_and_as_long_as_a_request_needs(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let connected = connect(&app, &team.cy, &team.workspace, None, &["read"]).await;
    let urlencoded = [(CONTENT_TYPE.as_str(), "application/x-www-form-urlencoded")];

    // A refresh with more parameters than any request sends is refused before anything is
    // looked up, and its refresh token still works.
    let crowded = format!(
        "grant_type=refresh_token&client_id={}&refresh_token={}&{}",
        connected.client,
        connected.refresh,
        distinct_keys(14)
    );
    let refused = raw(
        &app,
        Method::POST,
        "/api/v1/oauth/token",
        &urlencoded,
        crowded,
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert_eq!(oauth_error(&refused), "invalid_request");
    let rotated = refresh(&app, &connected.client, &connected.refresh, None).await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.body);

    // A form past 16 KiB is refused unread, however its parameters are named.
    for uri in ["/api/v1/oauth/token", "/api/v1/oauth/revoke"] {
        let large = raw(&app, Method::POST, uri, &urlencoded, distinct_keys(8000)).await;
        assert_eq!(large.status, StatusCode::PAYLOAD_TOO_LARGE, "{uri}");
        assert_eq!(oauth_error(&large), "invalid_request", "{uri}");
        assert_eq!(large.headers[CACHE_CONTROL], "no-store", "{uri}");
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_grant_exchanges_a_few_at_a_time(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let mut current = connect(&app, &team.cy, ws, None, &["read"]).await;

    // The exchange of its code and nine refreshes, then a wait.
    let mut refreshes = 0;
    let limited = loop {
        let answer = refresh(&app, &current.client, &current.refresh, None).await;
        if answer.status != StatusCode::OK {
            break answer;
        }
        current = tokens(&current.client, &answer);
        refreshes += 1;
    };
    assert_eq!(refreshes, 9, "{}", limited.body);
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(oauth_error(&limited), "rate_limited");
    assert!(limited.headers.contains_key(RETRY_AFTER));
    // Nothing was spent or ended: the last tokens work, and another grant exchanges.
    assert_eq!(works(&app, &current, ws).await, StatusCode::OK);
    let other = connect(&app, &team.cy, ws, None, &["read"]).await;
    let rotated = refresh(&app, &other.client, &other.refresh, None).await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.body);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn refreshing_never_widens_and_consenting_again_narrows_at_once(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let teamspace = general(&team.cy, ws).await;
    let writer = connect(&app, &team.cy, ws, None, &["read", "write"]).await;

    let narrower = refresh(&app, &writer.client, &writer.refresh, Some("read")).await;
    assert_eq!(narrower.status, StatusCode::OK, "{}", narrower.body);
    assert_eq!(narrower.body["scope"], "read");
    let reader = tokens(&writer.client, &narrower);
    let wider = refresh(&app, &writer.client, &reader.refresh, Some("read write")).await;
    assert_eq!(oauth_error(&wider), "invalid_scope");
    let admin = refresh(&app, &writer.client, &reader.refresh, Some("admin")).await;
    assert_eq!(oauth_error(&admin), "invalid_scope");
    let refused = reader
        .bearer(&app)
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({"title": "No", "teamspace_id": teamspace}),
        )
        .await;
    assert_eq!(refused.code(), "insufficient_scope");

    // Consenting again to the same client, workspace and identity with fewer scopes
    // narrows its tokens at once.
    let client = register(&app, &[REDIRECT]).await;
    let verifier = verifier('v');
    let asked = request(&client, &verifier, None);
    let code = code(&team.cy, &asked, ws, None, &["read", "write"]).await;
    let both = tokens(&client, &exchange(&app, &client, &code, &verifier).await);
    create_with(&both.bearer(&app), ws, &teamspace, "Before").await;
    self::code(&team.cy, &asked, ws, None, &["read"]).await;
    let narrowed = both
        .bearer(&app)
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({"title": "After", "teamspace_id": teamspace}),
        )
        .await;
    assert_eq!(narrowed.code(), "insufficient_scope");
    let apps = team.cy.get("/api/v1/account/connected-apps").await;
    let grants: Vec<&Value> = apps.body["apps"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|app| app["client"]["id"] == client.as_str())
        .collect();
    assert_eq!(grants.len(), 1, "one grant, replaced");
    assert_eq!(grants[0]["scopes"], json!(["read"]));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn tokens_expire_and_refresh_tokens_last_as_set(pool: PgPool) {
    let time = ManualTime::new(OffsetDateTime::now_utc());
    let state = public(pool, time.clock()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    setting(&state, "oauth.access_token_ttl", json!("10m")).await;
    setting(&state, "oauth.refresh_token_ttl", json!("2h")).await;
    let connected = connect(&app, &team.cy, ws, None, &["read"]).await;

    time.advance(Duration::from_secs(9 * 60));
    assert_eq!(works(&app, &connected, ws).await, StatusCode::OK);
    time.advance(Duration::from_secs(60));
    assert_eq!(works(&app, &connected, ws).await, StatusCode::UNAUTHORIZED);
    let renewed = tokens(
        &connected.client,
        &refresh(&app, &connected.client, &connected.refresh, None).await,
    );
    assert_eq!(works(&app, &renewed, ws).await, StatusCode::OK);

    time.advance(Duration::from_secs(2 * 3600));
    let expired = refresh(&app, &connected.client, &renewed.refresh, None).await;
    assert_eq!(oauth_error(&expired), "invalid_grant");
    assert!(
        expired.body["error_description"]
            .as_str()
            .expect("text")
            .contains("expired")
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn revocation_ends_the_grant_of_the_clients_own_tokens_alone(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let mine = connect(&app, &team.cy, ws, None, &["read"]).await;
    let theirs = connect(&app, &team.cy, ws, None, &["read"]).await;

    // Another client's token, an unknown one: the same answer, and nothing changes.
    for (token, client) in [
        (theirs.access.as_str(), mine.client.as_str()),
        ("nwo_unknown", mine.client.as_str()),
        ("something", mine.client.as_str()),
    ] {
        let answer = form(
            &app,
            "/api/v1/oauth/revoke",
            &[("token", token), ("client_id", client)],
        )
        .await;
        assert_eq!(answer.status, StatusCode::OK, "{token}");
    }
    assert_eq!(works(&app, &theirs, ws).await, StatusCode::OK);
    let missing = form(&app, "/api/v1/oauth/revoke", &[("client_id", &mine.client)]).await;
    assert_eq!(oauth_error(&missing), "invalid_request");
    let unknown = form(
        &app,
        "/api/v1/oauth/revoke",
        &[("token", &mine.access), ("client_id", "nobody")],
    )
    .await;
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(oauth_error(&unknown), "invalid_client");

    // Its own refresh token: the access token stops too.
    let revoked = form(
        &app,
        "/api/v1/oauth/revoke",
        &[
            ("token", &mine.refresh),
            ("token_type_hint", "refresh_token"),
            ("client_id", &mine.client),
        ],
    )
    .await;
    assert_eq!(revoked.status, StatusCode::OK);
    assert_eq!(works(&app, &mine, ws).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        oauth_error(&refresh(&app, &mine.client, &mine.refresh, None).await),
        "invalid_grant"
    );
    assert_eq!(works(&app, &theirs, ws).await, StatusCode::OK);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agents_tokens_stop_while_it_is_suspended_and_resume_after(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let (team, scout) = setting_up(&state, &app).await;
    let ws = &team.workspace;
    let connected = connect(&app, &team.cy, ws, Some(&scout), &["read", "write"]).await;
    let cy = me(&team.cy).await;
    let role = |role: &str| {
        let path = format!("/api/v1/workspaces/{ws}/members/{cy}");
        let body = json!({"role": role});
        let ada = &team.ada;
        async move { ada.send(Method::PATCH, &path, Some(body)).await }
    };

    // Its owner a guest: suspended, and a refresh spends nothing meanwhile.
    assert_eq!(role("guest").await.status, StatusCode::OK);
    assert_eq!(works(&app, &connected, ws).await, StatusCode::UNAUTHORIZED);
    let meanwhile = refresh(&app, &connected.client, &connected.refresh, None).await;
    assert_eq!(oauth_error(&meanwhile), "invalid_grant");
    assert_eq!(role("member").await.status, StatusCode::OK);
    assert_eq!(works(&app, &connected, ws).await, StatusCode::OK);
    let resumed = refresh(&app, &connected.client, &connected.refresh, None).await;
    assert_eq!(resumed.status, StatusCode::OK, "{}", resumed.body);
    let connected = tokens(&connected.client, &resumed);

    // Agents switched off, in the workspace or on the instance.
    let switch = |on: bool| {
        let path = format!("/api/v1/workspaces/{ws}");
        let ada = &team.ada;
        async move {
            ada.send(Method::PATCH, &path, Some(json!({"agents_enabled": on})))
                .await
        }
    };
    assert_eq!(switch(false).await.status, StatusCode::OK);
    assert_eq!(works(&app, &connected, ws).await, StatusCode::UNAUTHORIZED);
    assert_eq!(switch(true).await.status, StatusCode::OK);
    assert_eq!(works(&app, &connected, ws).await, StatusCode::OK);
    setting(&state, "mcp.identities", json!(["user"])).await;
    assert_eq!(works(&app, &connected, ws).await, StatusCode::UNAUTHORIZED);
    setting(&state, "mcp.identities", json!(["user", "agent"])).await;
    assert_eq!(works(&app, &connected, ws).await, StatusCode::OK);

    // Handed to someone else: its grants end with its tokens.
    let dee = me(&team.dee).await;
    let handed = team
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/agents/{scout}/transfer"),
            json!({"owner_id": dee}),
        )
        .await;
    assert_eq!(handed.status, StatusCode::OK, "{}", handed.body);
    assert_eq!(works(&app, &connected, ws).await, StatusCode::UNAUTHORIZED);
    let apps = team.cy.get("/api/v1/account/connected-apps").await;
    assert_eq!(apps.body["apps"], json!([]));

    // Deleted: the same.
    let again = connect(&app, &team.dee, ws, Some(&scout), &["read"]).await;
    assert_eq!(works(&app, &again, ws).await, StatusCode::OK);
    let deleted = team
        .dee
        .send(
            Method::DELETE,
            &format!("/api/v1/workspaces/{ws}/agents/{scout}"),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(works(&app, &again, ws).await, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_persons_tokens_stop_once_they_lose_access(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    admin(&state).await;
    let app = router(state.clone(), false);
    let (team, scout) = setting_up(&state, &app).await;
    let ws = &team.workspace;
    let admin = Browser::sign_in(&app, "admin@example.com").await;
    let cy = me(&team.cy).await;
    let as_cy = connect(&app, &team.cy, ws, None, &["read"]).await;
    let as_scout = connect(&app, &team.cy, ws, Some(&scout), &["read"]).await;

    // Acting as people switched off.
    setting(&state, "mcp.identities", json!(["agent"])).await;
    assert_eq!(works(&app, &as_cy, ws).await, StatusCode::UNAUTHORIZED);
    assert_eq!(works(&app, &as_scout, ws).await, StatusCode::OK);
    setting(&state, "mcp.identities", json!(["user", "agent"])).await;

    // Disabled: both stop.
    let disable = |disabled: bool| {
        let path = format!("/api/v1/admin/users/{cy}");
        let admin = &admin;
        async move {
            admin
                .send(Method::PATCH, &path, Some(json!({"disabled": disabled})))
                .await
        }
    };
    assert_eq!(disable(true).await.status, StatusCode::OK);
    assert_eq!(works(&app, &as_cy, ws).await, StatusCode::UNAUTHORIZED);
    assert_eq!(works(&app, &as_scout, ws).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        oauth_error(&refresh(&app, &as_cy.client, &as_cy.refresh, None).await),
        "invalid_grant"
    );
    assert_eq!(disable(false).await.status, StatusCode::OK);
    assert_eq!(works(&app, &as_cy, ws).await, StatusCode::OK);

    // Out of the workspace: both stop.
    let removed = team
        .ada
        .send(
            Method::DELETE,
            &format!("/api/v1/workspaces/{ws}/members/{cy}"),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    assert_eq!(works(&app, &as_cy, ws).await, StatusCode::UNAUTHORIZED);
    assert_eq!(works(&app, &as_scout, ws).await, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn people_list_and_disconnect_their_apps_and_admins_those_of_agents(pool: PgPool) {
    let state = public(pool, Clock::system()).await;
    let app = router(state.clone(), false);
    let (team, scout) = setting_up(&state, &app).await;
    let ws = &team.workspace;
    let as_cy = connect(&app, &team.cy, ws, None, &["read", "write"]).await;
    let as_scout = connect(&app, &team.cy, ws, Some(&scout), &["read"]).await;
    let cy = me(&team.cy).await;

    let listed = team.cy.get("/api/v1/account/connected-apps").await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let apps = listed.body["apps"].as_array().expect("a list");
    assert_eq!(apps.len(), 2);
    let find = |client: &str| {
        apps.iter()
            .find(|app| app["client"]["id"] == client)
            .expect("listed")
            .clone()
    };
    let mine = find(&as_cy.client);
    assert_eq!(mine["client"]["name"], "Claude Code");
    assert_eq!(mine["workspace"]["id"], ws.as_str());
    assert_eq!(mine["workspace"]["name"], "Design");
    assert_eq!(mine["identity"]["id"], cy.as_str());
    assert_eq!(mine["identity"]["kind"], "human");
    assert_eq!(mine["connected_by"]["id"], cy.as_str());
    assert_eq!(mine["scopes"], json!(["read", "write"]));
    assert_eq!(mine["resource"], "mcp");
    assert!(mine["last_used_at"].is_string());
    let agents = find(&as_scout.client);
    assert_eq!(agents["identity"]["id"], scout.as_str());
    assert_eq!(agents["identity"]["kind"], "agent");
    assert_eq!(agents["identity"]["owner"]["id"], cy.as_str());
    assert_eq!(
        team.dee.get("/api/v1/account/connected-apps").await.body["apps"],
        json!([])
    );

    // Workspace admins see the apps acting as agents, and members their own agents'.
    let workspace_apps = format!("/api/v1/workspaces/{ws}/connected-apps");
    for (who, count) in [
        (&team.bob, 1),
        (&team.ada, 1),
        (&team.cy, 1),
        (&team.dee, 0),
    ] {
        let seen = who.get(&workspace_apps).await;
        assert_eq!(seen.status, StatusCode::OK, "{}", seen.body);
        let seen = seen.body["apps"].as_array().expect("a list");
        assert_eq!(seen.len(), count);
        if let Some(first) = seen.first() {
            assert_eq!(first["client"]["id"], as_scout.client.as_str());
        }
    }
    let gone = format!("{workspace_apps}/{}", agents["id"].as_str().expect("an id"));
    let refused = team.dee.send(Method::DELETE, &gone, None).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    let not_an_agent = format!("{workspace_apps}/{}", mine["id"].as_str().expect("an id"));
    assert_eq!(
        team.bob
            .send(Method::DELETE, &not_an_agent, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let token = team.bob.token(&["read", "write"]).await;
    let by_token = Bearer::new(&app, token)
        .send(Method::DELETE, &gone, None)
        .await;
    assert_eq!(by_token.status, StatusCode::FORBIDDEN, "{}", by_token.body);
    let disconnected = team.bob.send(Method::DELETE, &gone, None).await;
    assert_eq!(
        disconnected.status,
        StatusCode::NO_CONTENT,
        "{}",
        disconnected.body
    );
    assert_eq!(works(&app, &as_scout, ws).await, StatusCode::UNAUTHORIZED);
    assert_eq!(works(&app, &as_cy, ws).await, StatusCode::OK);

    // A person disconnects their own, from a browser.
    let own = format!(
        "/api/v1/account/connected-apps/{}",
        mine["id"].as_str().expect("an id")
    );
    assert_eq!(
        team.dee.send(Method::DELETE, &own, None).await.status,
        StatusCode::NOT_FOUND
    );
    let cy_token = team.cy.token(&["read", "write"]).await;
    let by_token = Bearer::new(&app, cy_token)
        .send(Method::DELETE, &own, None)
        .await;
    assert_eq!(by_token.status, StatusCode::FORBIDDEN);
    assert_eq!(
        team.cy.send(Method::DELETE, &own, None).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(works(&app, &as_cy, ws).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        team.cy.get("/api/v1/account/connected-apps").await.body["apps"],
        json!([])
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_cleanup_removes_what_expired_and_clients_nobody_connected(pool: PgPool) {
    let time = ManualTime::new(OffsetDateTime::now_utc());
    let state = public(pool, time.clock()).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    setting(&state, "oauth.refresh_token_ttl", json!("2h")).await;
    let unconnected = register(&app, &[REDIRECT]).await;
    let connected = connect(&app, &team.cy, ws, None, &["read"]).await;

    let report = state.oauth.cleanup().await.expect("cleaned");
    assert_eq!(report, neon_app::oauth::CleanupReport::default());

    // Its tokens expired, the grant is spent; the clients stay a day after their last use.
    time.advance(Duration::from_secs(3 * 3600));
    let report = state.oauth.cleanup().await.expect("cleaned");
    assert_eq!(
        report,
        neon_app::oauth::CleanupReport {
            codes: 1,
            tokens: 2,
            grants: 1,
            clients: 0,
        }
    );
    assert_eq!(
        oauth_error(&refresh(&app, &connected.client, &connected.refresh, None).await),
        "invalid_grant"
    );
    assert_eq!(
        team.cy.get("/api/v1/account/connected-apps").await.body["apps"],
        json!([])
    );

    // A day on, both go: the one nobody connected, and the one whose grant ended.
    time.advance(Duration::from_secs(22 * 3600));
    let report = state.oauth.cleanup().await.expect("cleaned");
    assert_eq!(report.clients, 2);
    let asked = request(&unconnected, &verifier('v'), None);
    let gone = team.cy.post("/api/v1/oauth/consent", asked).await;
    assert_eq!(gone.code(), "unknown_client");
}

/// A user who signed up while registration was open.
async fn user(state: &AppState, email: &str) {
    super::auth_tests::user(state, email).await;
}
