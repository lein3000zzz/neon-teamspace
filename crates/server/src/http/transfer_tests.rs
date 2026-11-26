//! Imports and exports over HTTP: the form, the downloads and their headers, the body
//! limit of uploads, the scopes and origin they need, and their error codes.

use std::collections::BTreeMap;
use std::io::{Cursor, Read as _, Write as _};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, COOKIE, ORIGIN, RETRY_AFTER,
};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt as _;
use neon_app::crypto::Envelope;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use url::Url;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::auth_tests::{call, sign_in, user};
use super::pages_tests::{create, general, guest};
use super::tests::{state, state_with};
use super::workspaces_tests::{Browser, team};
use super::*;

pub(super) const BOUNDARY: &str = "neon-test-form";

/// A part of a form: its field's name, its file's name if any, and its content.
pub(super) type Part<'a> = (&'a str, Option<&'a str>, &'a [u8]);

/// A `multipart/form-data` body of `parts`.
pub(super) fn form(parts: &[Part<'_>]) -> Vec<u8> {
    let mut body = Vec::new();
    for (field, file, content) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        let disposition = match file {
            Some(file) => format!("form-data; name=\"{field}\"; filename=\"{file}\""),
            None => format!("form-data; name=\"{field}\""),
        };
        body.extend_from_slice(format!("Content-Disposition: {disposition}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// What a request answered: its status, headers and body as they came.
pub(super) struct Raw {
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) body: Bytes,
}

impl Raw {
    pub(super) fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&self.body)))
    }

    pub(super) fn code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or_else(|| panic!("not the error shape: {}", self.json()))
            .to_owned()
    }

    pub(super) fn header(&self, name: impl axum::http::header::AsHeaderName) -> &str {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    }
}

pub(super) async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    content_type: Option<&str>,
    body: Vec<u8>,
) -> Raw {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if let Some(content_type) = content_type {
        request = request.header(CONTENT_TYPE, content_type);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body)).expect("a request"))
        .await
        .expect("a response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    Raw {
        status,
        headers,
        body,
    }
}

/// Posts a form of `parts` to import into the place `query` names.
pub(super) async fn import(
    by: &Browser<'_>,
    workspace: &str,
    query: &str,
    parts: &[Part<'_>],
) -> Raw {
    send(
        by.app(),
        Method::POST,
        &format!("/api/v1/workspaces/{workspace}/pages/import?{query}"),
        &[(COOKIE.as_str(), by.cookie())],
        Some(&format!("multipart/form-data; boundary={BOUNDARY}")),
        form(parts),
    )
    .await
}

pub(super) async fn download(by: &Browser<'_>, uri: &str) -> Raw {
    send(
        by.app(),
        Method::GET,
        uri,
        &[(COOKIE.as_str(), by.cookie())],
        None,
        Vec::new(),
    )
    .await
}

/// A zip of `files`, each a name and its content.
pub(super) fn zipped(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in files {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .expect("a file");
        writer.write_all(content).expect("written");
    }
    writer.finish().expect("finished").into_inner()
}

pub(super) fn unzipped(bytes: &[u8]) -> BTreeMap<String, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip");
    (0..zip.len())
        .map(|i| {
            let mut file = zip.by_index(i).expect("an entry");
            let mut text = String::new();
            file.read_to_string(&mut text).expect("text");
            (file.name().expect("a name").into_owned(), text)
        })
        .collect()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_zip_imports_and_exports_back(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let archive = zipped(&[
        ("Roadmap.md", b"# Roadmap\n\nWhere we are going.\n"),
        ("Roadmap/Q1.md", b"# Q1\n\n1. hire\n2. ship\n"),
        ("Roadmap/logo.png", b"png"),
        ("Notes.md", b"Plain notes.\n"),
    ]);

    let imported = import(
        &t.ada,
        ws,
        &format!("teamspace_id={general}"),
        &[("file", Some("Export.zip"), &archive)],
    )
    .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json());
    let body = imported.json();
    assert_eq!(body["count"], 3);
    assert_eq!(body["pages"][0]["title"], "Roadmap");
    assert_eq!(body["pages"][0]["access"], "edit");
    assert_eq!(body["pages"][1]["title"], "Notes");
    assert_eq!(
        body["skipped"],
        json!([{"path": "Roadmap/logo.png", "reason": "attachment"}])
    );
    let roadmap = body["pages"][0]["id"].as_str().expect("an id").to_owned();
    let tree = t
        .ada
        .get(&format!("/api/v1/teamspaces/{general}/pages"))
        .await;
    let titles: Vec<&str> = tree.body["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|page| page["title"].as_str().expect("a title"))
        .collect();
    assert_eq!(titles, ["Roadmap", "Q1", "Notes"]);

    let file = download(&t.cy, &format!("/api/v1/pages/{roadmap}/export")).await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.header(CONTENT_TYPE), "text/markdown; charset=utf-8");
    assert_eq!(
        file.header(CONTENT_DISPOSITION),
        "attachment; filename=\"Roadmap.md\"; filename*=UTF-8''Roadmap.md"
    );
    assert_eq!(file.header(CACHE_CONTROL), "no-store");
    assert_eq!(&file.body[..], b"# Roadmap\n\nWhere we are going.\n");

    let zip = download(
        &t.cy,
        &format!("/api/v1/pages/{roadmap}/export?children=true"),
    )
    .await;
    assert_eq!(zip.status, StatusCode::OK);
    assert_eq!(zip.header(CONTENT_TYPE), "application/zip");
    assert!(
        zip.header(CONTENT_DISPOSITION)
            .contains("filename=\"Roadmap.zip\"")
    );
    assert_eq!(
        unzipped(&zip.body),
        BTreeMap::from([
            (
                "Roadmap.md".to_owned(),
                "# Roadmap\n\nWhere we are going.\n".to_owned()
            ),
            (
                "Roadmap/Q1.md".to_owned(),
                "# Q1\n\n1. hire\n2. ship\n".to_owned()
            ),
        ])
    );

    let teamspace = download(&t.dee, &format!("/api/v1/teamspaces/{general}/export")).await;
    assert_eq!(teamspace.status, StatusCode::OK);
    assert!(
        teamspace
            .header(CONTENT_DISPOSITION)
            .contains("filename=\"General.zip\"")
    );
    assert_eq!(
        unzipped(&teamspace.body).keys().collect::<Vec<_>>(),
        ["Notes.md", "Roadmap.md", "Roadmap/Q1.md"]
    );

    // One markdown file, under a page.
    let one = import(
        &t.dee,
        ws,
        &format!("parent_id={roadmap}"),
        &[("file", Some("Retro.md"), b"# Retro\n\nWent well.\n")],
    )
    .await;
    assert_eq!(one.status, StatusCode::CREATED, "{}", one.json());
    assert_eq!(one.json()["pages"][0]["parent_id"], roadmap.as_str());
    let private = import(
        &t.dee,
        ws,
        "private=true",
        &[("file", Some("mine.markdown"), b"mine")],
    )
    .await;
    assert_eq!(private.status, StatusCode::CREATED, "{}", private.json());
    assert_eq!(private.json()["pages"][0]["title"], "mine");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn imports_and_exports_answer_in_the_error_shape(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let place = format!("teamspace_id={general}");
    let note: Part<'_> = ("file", Some("a.md"), b"a");

    let refused = [
        (import(&t.ada, ws, "", &[note]).await, "invalid_input"),
        (
            import(&t.ada, ws, &format!("{place}&private=true"), &[note]).await,
            "invalid_input",
        ),
        (import(&t.ada, ws, &place, &[]).await, "invalid_input"),
        (
            import(&t.ada, ws, &place, &[note, note]).await,
            "invalid_input",
        ),
        (
            import(&t.ada, ws, &place, &[("upload", Some("a.md"), b"a")]).await,
            "invalid_input",
        ),
        (
            import(&t.ada, ws, &place, &[("file", Some("a.txt"), b"a")]).await,
            "invalid_input",
        ),
        (
            import(&t.ada, ws, &place, &[("file", Some("a.zip"), b"not a zip")]).await,
            "invalid_input",
        ),
        (
            import(
                &t.ada,
                ws,
                &place,
                &[("file", Some("a.zip"), &zipped(&[("../a.md", b"a")]))],
            )
            .await,
            "invalid_input",
        ),
    ];
    for (answer, code) in refused {
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.json());
        assert_eq!(answer.code(), code);
    }
    let not_a_form = send(
        &app,
        Method::POST,
        &format!("/api/v1/workspaces/{ws}/pages/import?{place}"),
        &[(COOKIE.as_str(), t.ada.cookie())],
        Some("application/json"),
        b"{}".to_vec(),
    )
    .await;
    assert_eq!(not_a_form.status, StatusCode::BAD_REQUEST);
    assert_eq!(not_a_form.code(), "invalid_body");

    user(&state, "eve@example.com").await;
    user(&state, "gus@example.com").await;
    let outsider = Browser::sign_in(&app, "eve@example.com").await;
    let hidden = import(&outsider, ws, &place, &[note]).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{}", hidden.json());
    let page = create(&t.ada, ws, json!({"teamspace_id": general}), "Plan").await;
    let gus = guest(&app, &t.ada, ws, "gus@example.com").await;
    let shared = t
        .ada
        .post(
            &format!("/api/v1/pages/{page}/shares"),
            json!({"user_id": me(&gus).await, "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    let viewer = import(&gus, ws, &format!("parent_id={page}"), &[note]).await;
    assert_eq!(viewer.status, StatusCode::FORBIDDEN, "{}", viewer.json());
    assert_eq!(viewer.code(), "forbidden");

    for uri in [
        format!("/api/v1/pages/{page}/export"),
        format!("/api/v1/teamspaces/{general}/export"),
    ] {
        let hidden = download(&outsider, &uri).await;
        assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(hidden.code(), "not_found");
    }
    let guest_teamspace = download(&gus, &format!("/api/v1/teamspaces/{general}/export")).await;
    assert_eq!(guest_teamspace.status, StatusCode::NOT_FOUND);
    let shared_page = download(&gus, &format!("/api/v1/pages/{page}/export")).await;
    assert_eq!(shared_page.status, StatusCode::OK);
    let bad = download(
        &t.ada,
        &format!("/api/v1/pages/{page}/export?children=maybe"),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad.code(), "invalid_query");
}

/// The id of whoever is signed in in `by`.
async fn me(by: &Browser<'_>) -> String {
    by.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn uploads_have_a_body_limit_of_their_own(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let place = format!("teamspace_id={general}");

    // Larger than any other route takes, as a zip that holds an image beside its page.
    let image = vec![7u8; 3 * 1024 * 1024];
    let archive = zipped(&[("Page.md", b"# Page\n"), ("Page/photo.jpg", &image)]);
    let large = import(&t.ada, ws, &place, &[("file", Some("big.zip"), &archive)]).await;
    assert_eq!(large.status, StatusCode::CREATED, "{}", large.json());
    assert_eq!(large.json()["skipped"][0]["reason"], "attachment");

    let max = state.transfer.limits().max_upload;
    let too_large = vec![b'x'; max + 1];
    let refused = import(&t.ada, ws, &place, &[("file", Some("big.md"), &too_large)]).await;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(refused.code(), "too_large");
    let past_the_form = vec![b'x'; max + 128 * 1024];
    let refused = import(
        &t.ada,
        ws,
        &place,
        &[("file", Some("big.md"), &past_the_form)],
    )
    .await;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(refused.code(), "payload_too_large");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn imports_need_the_write_scope_the_origin_and_their_rate(pool: PgPool) {
    let public = Url::parse("https://neon.example.com").expect("a URL");
    let state = state_with(
        pool,
        Envelope::default(),
        WebConfig::new(Some(&public), Vec::new()),
    )
    .await;
    user(&state, "ada@example.com").await;
    let app = router(state, false);
    let cookie = sign_in(&app, "ada@example.com").await;
    let same_origin = [
        (COOKIE.as_str(), cookie.as_str()),
        (ORIGIN.as_str(), "https://neon.example.com"),
    ];
    let created = call(
        &app,
        Method::POST,
        "/api/v1/workspaces",
        &same_origin,
        Some(json!({"name": "Design"})),
    )
    .await;
    let ws = created.body["id"].as_str().expect("an id").to_owned();
    let made = call(
        &app,
        Method::POST,
        "/api/v1/account/tokens",
        &same_origin,
        Some(json!({"name": "reader", "scopes": ["read"]})),
    )
    .await;
    let reader = format!("Bearer {}", made.body["token"].as_str().expect("a token"));
    let uri = format!("/api/v1/workspaces/{ws}/pages/import?private=true");
    let content_type = format!("multipart/form-data; boundary={BOUNDARY}");
    let body = || form(&[("file", Some("a.md"), b"a")]);

    let cross = send(
        &app,
        Method::POST,
        &uri,
        &[
            (COOKIE.as_str(), cookie.as_str()),
            (ORIGIN.as_str(), "https://evil.example"),
        ],
        Some(&content_type),
        body(),
    )
    .await;
    assert_eq!(cross.status, StatusCode::FORBIDDEN);
    assert_eq!(cross.code(), "forbidden");
    let read_only = send(
        &app,
        Method::POST,
        &uri,
        &[(AUTHORIZATION.as_str(), reader.as_str())],
        Some(&content_type),
        body(),
    )
    .await;
    assert_eq!(read_only.status, StatusCode::FORBIDDEN);
    assert_eq!(read_only.code(), "insufficient_scope");

    // Ten imports at once, then one every 30 seconds.
    let mut made = Vec::new();
    for n in 0..10 {
        let done = send(
            &app,
            Method::POST,
            &uri,
            &same_origin,
            Some(&content_type),
            body(),
        )
        .await;
        assert_eq!(
            done.status,
            StatusCode::CREATED,
            "import {n}: {}",
            done.json()
        );
        made.push(
            done.json()["pages"][0]["id"]
                .as_str()
                .expect("an id")
                .to_owned(),
        );
    }
    let limited = send(
        &app,
        Method::POST,
        &uri,
        &same_origin,
        Some(&content_type),
        body(),
    )
    .await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.code(), "rate_limited");
    assert!(limited.header(RETRY_AFTER).parse::<u64>().is_ok());

    // Exporting reads, so the read scope is enough.
    let exported = send(
        &app,
        Method::GET,
        &format!("/api/v1/pages/{}/export", made[0]),
        &[(AUTHORIZATION.as_str(), reader.as_str())],
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(exported.status, StatusCode::OK);
    assert_eq!(&exported.body[..], b"# a\n\na\n");
}
