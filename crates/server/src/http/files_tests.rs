//! Attachments over HTTP: uploading to a page, the types its bytes say and the kinds that
//! take them, the size and storage limits, reading files back with the headers that keep
//! them in their sandbox, ranges and revalidation, who may read and upload, and covers.

use std::sync::Arc;

use axum::Router;
use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN};
use axum::http::{Method, StatusCode};
use neon_app::clock::Clock;
use neon_app::ratelimit::{Bucket, Limited, RateLimiter};
use neon_app::settings::Change;
use serde_json::{Value, json};
use sqlx::PgPool;

use super::agents_tests::{agent, agent_token};
use super::auth_tests::user;
use super::pages_tests::{create, general, guest};
use super::tests::{state, state_with_blobs};
use super::transfer_tests::{BOUNDARY, Raw, form, send};
use super::workspaces_tests::{Browser, id, team};
use super::*;

/// The smallest PNG header that says its size: 640 by 480.
fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend(640u32.to_be_bytes());
    bytes.extend(480u32.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes.extend(b"\0\0\0\0IEND\xaeB`\x82");
    bytes
}

const HTML: &[u8] = b"<!doctype html><script>document.cookie</script>";
const SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>";

/// A form whose one part declares `declared` as its type, which the server ignores.
fn declared_form(file: &str, declared: &str, content: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file}\"\r\n\
         Content-Type: {declared}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// Uploads a file to a page as a browser, for a block of `kind`.
async fn upload(by: &Browser<'_>, page: &str, kind: &str, name: &str, content: &[u8]) -> Raw {
    upload_form(by, page, kind, form(&[("file", Some(name), content)])).await
}

async fn upload_form(by: &Browser<'_>, page: &str, kind: &str, body: Vec<u8>) -> Raw {
    send(
        by.app(),
        Method::POST,
        &format!("/api/v1/pages/{page}/files?kind={kind}"),
        &[(COOKIE.as_str(), by.cookie())],
        Some(&format!("multipart/form-data; boundary={BOUNDARY}")),
        body,
    )
    .await
}

/// Reads a file as a browser, with `headers` besides its cookie.
async fn read(by: &Browser<'_>, url: &str, headers: &[(&str, &str)]) -> Raw {
    let mut all = vec![(COOKIE.as_str(), by.cookie())];
    all.extend_from_slice(headers);
    send(by.app(), Method::GET, url, &all, None, Vec::new()).await
}

/// Reads a file with a bearer token alone.
async fn read_as(app: &Router, token: &str, url: &str) -> Raw {
    send(
        app,
        Method::GET,
        url,
        &[(AUTHORIZATION.as_str(), token)],
        None,
        Vec::new(),
    )
    .await
}

fn url_of(uploaded: &Raw) -> String {
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.json());
    uploaded.json()["url"].as_str().expect("a url").to_owned()
}

async fn page_in_general(by: &Browser<'_>, workspace: &str, title: &str) -> String {
    let general = general(by, workspace).await;
    create(by, workspace, json!({"teamspace_id": general}), title).await
}

async fn set(state: &AppState, key: &str, value: Value) {
    state
        .settings
        .update([(key.to_owned(), Change::Set(value))], None)
        .await
        .expect("saved");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_image_uploads_and_reads_back_in_place(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let page = page_in_general(&t.ada, &t.workspace, "Plan").await;

    let uploaded = upload(&t.ada, &page, "image", "C:\\shots\\diagram.png", &png()).await;
    let file = uploaded.json();
    let url = url_of(&uploaded);
    assert_eq!(
        url,
        format!("/api/v1/files/{}", file["id"].as_str().expect("an id"))
    );
    assert_eq!(file["page_id"], page.as_str());
    assert_eq!(file["kind"], "image");
    assert_eq!(
        file["name"], "diagram.png",
        "the client's folders are left out"
    );
    assert_eq!(file["content_type"], "image/png");
    assert_eq!(file["size"], png().len());
    assert_eq!(
        (file["width"].clone(), file["height"].clone()),
        (json!(640), json!(480))
    );
    assert_eq!(file["sha256"].as_str().expect("a hash").len(), 64);

    let got = read(&t.cy, &url, &[]).await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(&got.body[..], &png()[..]);
    assert_eq!(got.header("content-type"), "image/png");
    assert_eq!(got.header("content-length"), png().len().to_string());
    assert_eq!(
        got.header("content-disposition"),
        "inline; filename=\"diagram.png\"; filename*=UTF-8''diagram.png"
    );
    assert_eq!(got.header("x-content-type-options"), "nosniff");
    assert_eq!(
        got.header("content-security-policy"),
        "default-src 'none'; style-src 'unsafe-inline'; sandbox"
    );
    assert_eq!(got.header("cross-origin-resource-policy"), "same-origin");
    assert_eq!(got.header("cache-control"), "private, no-cache");
    assert_eq!(got.header("accept-ranges"), "bytes");
    let etag = got.header("etag").to_owned();
    assert_eq!(
        etag,
        format!("\"{}\"", file["sha256"].as_str().expect("a hash"))
    );

    // A browser that kept it asks again, and the rules answer again.
    let again = read(&t.cy, &url, &[("if-none-match", &etag)]).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    assert!(again.body.is_empty());
    assert_eq!(again.header("etag"), etag);
    assert_eq!(again.header("cache-control"), "private, no-cache");
    let other = read(&t.cy, &url, &[("if-none-match", "\"another\"")]).await;
    assert_eq!(other.status, StatusCode::OK);

    // Uploads from a browser come from the server's own origin, as every change does.
    let foreign = send(
        &app,
        Method::POST,
        &format!("/api/v1/pages/{page}/files?kind=image"),
        &[
            (COOKIE.as_str(), t.ada.cookie()),
            (ORIGIN.as_str(), "https://evil.example"),
        ],
        Some(&format!("multipart/form-data; boundary={BOUNDARY}")),
        form(&[("file", Some("x.png"), &png())]),
    )
    .await;
    assert_eq!(foreign.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_bytes_say_what_a_file_is_and_its_kind_takes_only_its_types(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let page = page_in_general(&t.ada, &t.workspace, "Plan").await;

    // HTML that calls itself a PNG is no image.
    let disguised = upload_form(
        &t.ada,
        &page,
        "image",
        declared_form("cat.png", "image/png", HTML),
    )
    .await;
    assert_eq!(disguised.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(disguised.code(), "unsupported_type");
    assert!(
        disguised.json()["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("text/html")
    );
    for (kind, content) in [
        ("video", png()),
        ("audio", b"%PDF-1.7\n".to_vec()),
        ("image", b"just words".to_vec()),
    ] {
        let refused = upload(&t.ada, &page, kind, "x", &content).await;
        assert_eq!(refused.status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{kind}");
    }
    let bad_kind = upload(&t.ada, &page, "spreadsheet", "x", b"x").await;
    assert_eq!(bad_kind.status, StatusCode::BAD_REQUEST);

    // A file block takes anything, served as what its bytes are, to download.
    let html = upload_form(
        &t.ada,
        &page,
        "file",
        declared_form("page.png", "image/png", HTML),
    )
    .await;
    let url = url_of(&html);
    assert_eq!(html.json()["content_type"], "text/html");
    let got = read(&t.ada, &url, &[]).await;
    assert_eq!(got.header("content-type"), "text/html; charset=utf-8");
    assert!(got.header("content-disposition").starts_with("attachment;"));
    assert_eq!(got.header("x-content-type-options"), "nosniff");
    assert!(got.header("content-security-policy").ends_with("sandbox"));

    // SVG is an image, shown by <img>, where it runs nothing, and downloaded otherwise.
    let svg = upload(&t.ada, &page, "image", "logo.svg", SVG).await;
    let svg_url = url_of(&svg);
    assert_eq!(svg.json()["width"], Value::Null);
    let got = read(&t.ada, &svg_url, &[]).await;
    assert_eq!(got.header("content-type"), "image/svg+xml");
    assert!(got.header("content-disposition").starts_with("attachment;"));
    assert!(got.header("content-security-policy").contains("sandbox"));

    // Media play in place; documents download.
    let pdf = upload(&t.ada, &page, "file", "report.pdf", b"%PDF-1.7\n%stuff").await;
    let got = read(&t.ada, &url_of(&pdf), &[]).await;
    assert_eq!(got.header("content-type"), "application/pdf");
    assert!(got.header("content-disposition").starts_with("attachment;"));
    let mp3 = upload(
        &t.ada,
        &page,
        "audio",
        "talk.mp3",
        b"ID3\x04\0\0\0\0\0\0frames",
    )
    .await;
    let got = read(&t.ada, &url_of(&mp3), &[]).await;
    assert_eq!(got.header("content-type"), "audio/mpeg");
    assert!(got.header("content-disposition").starts_with("inline;"));

    // A form holds the file alone.
    let other = upload_form(&t.ada, &page, "file", form(&[("note", None, b"x")])).await;
    assert_eq!(other.status, StatusCode::BAD_REQUEST);
    assert_eq!(other.code(), "invalid_input");
    let empty = upload_form(
        &t.ada,
        &page,
        "file",
        format!("--{BOUNDARY}--\r\n").into_bytes(),
    )
    .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn ranges_serve_parts_of_a_file(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let page = page_in_general(&t.ada, &t.workspace, "Media").await;
    let mut content = b"ID3\x04\0\0\0\0\0\0".to_vec();
    content.extend((0..1000u32).map(|n| u8::try_from(n % 251).expect("a byte")));
    let size = content.len();
    let url = url_of(&upload(&t.ada, &page, "audio", "talk.mp3", &content).await);

    let part = read(&t.ada, &url, &[("range", "bytes=10-19")]).await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(&part.body[..], &content[10..20]);
    assert_eq!(part.header("content-range"), format!("bytes 10-19/{size}"));
    assert_eq!(part.header("content-length"), "10");
    let tail = read(&t.ada, &url, &[("range", "bytes=-5")]).await;
    assert_eq!(&tail.body[..], &content[size - 5..]);
    assert_eq!(
        tail.header("content-range"),
        format!("bytes {}-{}/{size}", size - 5, size - 1)
    );
    let rest = read(&t.ada, &url, &[("range", &format!("bytes={}-", size - 3))]).await;
    assert_eq!(&rest.body[..], &content[size - 3..]);
    let past = read(&t.ada, &url, &[("range", "bytes=0-999999")]).await;
    assert_eq!(past.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(past.body.len(), size);

    let beyond = read(&t.ada, &url, &[("range", &format!("bytes={size}-"))]).await;
    assert_eq!(beyond.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(beyond.header("content-range"), format!("bytes */{size}"));
    assert_eq!(beyond.code(), "range_not_satisfiable");

    // Several ranges, or one that does not parse, answer the whole file.
    for range in ["bytes=0-1,5-6", "bytes=5-1", "lines=1-2"] {
        let whole = read(&t.ada, &url, &[("range", range)]).await;
        assert_eq!(whole.status, StatusCode::OK, "{range}");
        assert_eq!(whole.body.len(), size, "{range}");
    }
    // A range of another version of the file is not this one's.
    let etag = part.header("etag").to_owned();
    let stale = read(
        &t.ada,
        &url,
        &[("range", "bytes=0-1"), ("if-range", "\"old\"")],
    )
    .await;
    assert_eq!(stale.status, StatusCode::OK);
    let current = read(&t.ada, &url, &[("range", "bytes=0-1"), ("if-range", &etag)]).await;
    assert_eq!(current.status, StatusCode::PARTIAL_CONTENT);

    // An empty file has no range to give.
    let empty = url_of(&upload(&t.ada, &page, "file", "empty.txt", b"").await);
    let whole = read(&t.ada, &empty, &[]).await;
    assert_eq!(whole.status, StatusCode::OK);
    assert!(whole.body.is_empty());
    let none = read(&t.ada, &empty, &[("range", "bytes=0-")]).await;
    assert_eq!(none.status, StatusCode::RANGE_NOT_SATISFIABLE);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_file_follows_its_page_for_every_kind_of_access(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    user(&state, "gus@example.com").await;
    let gus = guest(&app, &t.ada, ws, "gus@example.com").await;
    let page = page_in_general(&t.ada, ws, "Shared").await;
    let url = url_of(&upload(&t.ada, &page, "image", "a.png", &png()).await);

    // Members of its teamspace see it; a guest sees it once the page is shared with them.
    assert_eq!(read(&t.cy, &url, &[]).await.status, StatusCode::OK);
    let hidden = read(&gus, &url, &[]).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(hidden.code(), "not_found");
    let gus_id = gus.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let shared = t
        .ada
        .post(
            &format!("/api/v1/pages/{page}/shares"),
            json!({"user_id": gus_id, "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    assert_eq!(read(&gus, &url, &[]).await.status, StatusCode::OK);
    // Viewing is not editing: a viewer uploads nothing.
    let refused = upload(&gus, &page, "image", "b.png", &png()).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    // A private teamspace's files are hidden from those outside it, as its pages are.
    let secret = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces"),
            json!({"name": "Secret", "access": "private"}),
        )
        .await;
    let secret = id(&secret);
    let hidden_page = create(&t.ada, ws, json!({"teamspace_id": secret}), "Hidden").await;
    let hidden_url = url_of(&upload(&t.ada, &hidden_page, "file", "plan.pdf", b"%PDF-1.7").await);
    assert_eq!(
        read(&t.cy, &hidden_url, &[]).await.status,
        StatusCode::NOT_FOUND
    );
    let refused = upload(&t.cy, &hidden_page, "file", "x.pdf", b"%PDF-1.7").await;
    assert_eq!(
        refused.status,
        StatusCode::NOT_FOUND,
        "nothing says the page exists"
    );

    // An agent reads what both it and its owner may: shared with the agent alone, the
    // hidden page's file stays hidden, as its owner cannot see it.
    let scout = agent(&t.cy, ws, "Scout").await;
    let token = agent_token(&t.cy, ws, &scout, &["read", "write"]).await;
    assert_eq!(read_as(&app, &token, &url).await.status, StatusCode::OK);
    let shared = t
        .ada
        .post(
            &format!("/api/v1/pages/{hidden_page}/shares"),
            json!({"user_id": scout, "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    assert_eq!(
        read_as(&app, &token, &hidden_url).await.status,
        StatusCode::NOT_FOUND
    );

    // A token bound to another workspace reads nothing of this one.
    let other = t
        .dee
        .post("/api/v1/workspaces", json!({"name": "Elsewhere"}))
        .await;
    let elsewhere = id(&other);
    let bot = agent(&t.dee, &elsewhere, "Bot").await;
    let bound = agent_token(&t.dee, &elsewhere, &bot, &["read"]).await;
    assert_eq!(
        read_as(&app, &bound, &url).await.status,
        StatusCode::NOT_FOUND
    );
    // Without credentials, nobody reads anything.
    let anonymous = send(&app, Method::GET, &url, &[], None, Vec::new()).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    // An id nobody uploaded is not found either.
    let unknown = read(
        &t.ada,
        "/api/v1/files/0193a5b2-0000-7000-8000-0000000000ff",
        &[],
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    // In the trash, a page's files show to those who could edit it, and to nobody else.
    let trashed = t
        .ada
        .post(&format!("/api/v1/pages/{page}/trash"), json!({}))
        .await;
    assert_eq!(trashed.status, StatusCode::OK, "{}", trashed.body);
    assert_eq!(read(&t.cy, &url, &[]).await.status, StatusCode::OK);
    assert_eq!(read(&gus, &url, &[]).await.status, StatusCode::NOT_FOUND);
    let refused = upload(&t.ada, &page, "image", "c.png", &png()).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    // Deleted for good, they go with it.
    let deleted = t
        .ada
        .send(Method::DELETE, &format!("/api/v1/pages/{page}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    assert_eq!(read(&t.ada, &url, &[]).await.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn uploads_stop_at_the_largest_file_and_the_storage_limit(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    super::auth_tests::admin(&state).await;
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let page = page_in_general(&t.ada, ws, "Plan").await;
    set(&state, "attachments.max_file_bytes", json!(1024)).await;

    let big = vec![b'a'; 2048];
    let refused = upload(&t.ada, &page, "file", "big.txt", &big).await;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(refused.code(), "file_too_large");
    // Declared larger than a form of the largest file can be, it is refused before it is
    // read.
    let declared = send(
        &app,
        Method::POST,
        &format!("/api/v1/pages/{page}/files?kind=file"),
        &[
            (COOKIE.as_str(), t.ada.cookie()),
            ("content-length", "999999999"),
        ],
        Some(&format!("multipart/form-data; boundary={BOUNDARY}")),
        Vec::new(),
    )
    .await;
    assert_eq!(declared.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(declared.code(), "file_too_large");
    let fits = upload(&t.ada, &page, "file", "small.txt", &[b'a'; 1024]).await;
    assert_eq!(fits.status, StatusCode::CREATED, "{}", fits.json());

    // The workspace's storage counts its files against its limit.
    let details = t.ada.get(&format!("/api/v1/workspaces/{ws}")).await;
    assert_eq!(details.body["storage_bytes"], 1024);
    set(&state, "workspaces.limits.storage_bytes", json!(1500)).await;
    let full = upload(&t.ada, &page, "file", "more.txt", &[b'b'; 600]).await;
    assert_eq!(full.status, StatusCode::CONFLICT);
    assert_eq!(full.code(), "storage_limit");
    let exact = upload(&t.ada, &page, "file", "rest.txt", &[b'c'; 476]).await;
    assert_eq!(exact.status, StatusCode::CREATED, "up to the limit fits");
    let past = upload(&t.ada, &page, "file", "one.txt", b"d").await;
    assert_eq!(past.status, StatusCode::CONFLICT);
    let details = t.ada.get(&format!("/api/v1/workspaces/{ws}")).await;
    assert_eq!(details.body["storage_bytes"], 1500);
    assert_eq!(details.body["limits"]["storage_bytes"], 1500);
    // Editing goes on at the limit.
    let renamed = t
        .ada
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{page}"),
            Some(json!({"title": "Plan v2"})),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK);

    // Instance admins see the usage beside the limit.
    let admin = Browser::sign_in(&app, "admin@example.com").await;
    let listed = admin.get("/api/v1/admin/workspaces").await;
    let design = listed.body["workspaces"]
        .as_array()
        .expect("workspaces")
        .iter()
        .find(|entry| entry["id"] == ws.as_str())
        .expect("Design is listed")
        .clone();
    assert_eq!(design["usage"]["storage_bytes"], 1500);
}

/// A body of `total` bytes in chunks of `chunk`, after `head`, sent as it is read with no
/// declared length, as a chunked request comes; and how many of its bytes were read.
fn streamed(
    head: Vec<u8>,
    fill: u8,
    total: usize,
    chunk: usize,
) -> (axum::body::Body, Arc<std::sync::atomic::AtomicUsize>) {
    use futures_util::StreamExt as _;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let read = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&read);
    let chunks = std::iter::once(head)
        .chain((0..total.div_ceil(chunk)).map(move |n| vec![fill; chunk.min(total - n * chunk)]));
    let body = futures_util::stream::iter(chunks).map(move |bytes| {
        counted.fetch_add(bytes.len(), Ordering::SeqCst);
        Ok::<_, std::convert::Infallible>(bytes::Bytes::from(bytes))
    });
    (axum::body::Body::from_stream(body), read)
}

async fn upload_streamed(by: &Browser<'_>, page: &str, body: axum::body::Body) -> Raw {
    upload_streamed_to(by.app(), by.cookie(), page, body).await
}

async fn upload_streamed_to(app: &Router, cookie: &str, page: &str, body: axum::body::Body) -> Raw {
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;
    let request = axum::http::Request::builder()
        .method(Method::POST)
        .uri(format!("/api/v1/pages/{page}/files?kind=file"))
        .header(COOKIE, cookie)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(body)
        .expect("a request");
    let response = app.clone().oneshot(request).await.expect("a response");
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

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_form_is_read_no_further_than_its_file_and_the_largest_file_bound_it(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let page = page_in_general(&t.ada, &t.workspace, "Plan").await;
    set(&state, "attachments.max_file_bytes", json!(256 * 1024)).await;
    let overhead = usize::try_from(files::FORM_OVERHEAD).expect("small");
    let chunk = 4096;

    // A form that never comes to its boundary is cut off once past what a form's headers
    // take, whatever it would go on to send.
    let (body, read) = streamed(Vec::new(), b'A', 64 * 1024 * 1024, chunk);
    let refused = upload_streamed(&t.ada, &page, body).await;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(refused.code(), "payload_too_large");
    let sent = read.load(std::sync::atomic::Ordering::SeqCst);
    assert!(sent <= overhead + 2 * chunk, "read {sent} bytes");

    // Nor do part headers that never end.
    let head = format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"\r\nX-More: ")
        .into_bytes();
    let (body, read) = streamed(head, b'h', 64 * 1024 * 1024, chunk);
    let refused = upload_streamed(&t.ada, &page, body).await;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(refused.code(), "payload_too_large");
    let sent = read.load(std::sync::atomic::Ordering::SeqCst);
    assert!(sent <= overhead + 2 * chunk, "read {sent} bytes");

    // A file larger than the form's headers may be passes once it starts, up to the largest.
    let head = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n\r\n"
    )
    .into_bytes();
    let mut whole = head.clone();
    whole.extend(vec![b'a'; 200 * 1024]);
    whole.extend(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    let (body, _) = streamed(whole, b'a', 0, chunk);
    let fits = upload_streamed(&t.ada, &page, body).await;
    assert_eq!(fits.status, StatusCode::CREATED, "{}", fits.json());
    assert_eq!(fits.json()["size"], 200 * 1024);
    // One past it stops as it comes, undeclared.
    let (body, read) = streamed(head, b'a', 64 * 1024 * 1024, chunk);
    let refused = upload_streamed(&t.ada, &page, body).await;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(refused.code(), "file_too_large");
    let sent = read.load(std::sync::atomic::Ordering::SeqCst);
    // The form's headers, then the largest file and its form, and a chunk past each.
    assert!(
        sent <= 2 * overhead + 256 * 1024 + 2 * chunk,
        "read {sent} bytes"
    );
}

/// Refuses uploads, as if the caller had used theirs up, and lets everything else in.
struct NoUploads;

#[async_trait::async_trait]
impl RateLimiter for NoUploads {
    async fn check(&self, bucket: Bucket, _: &str) -> Result<(), Limited> {
        if bucket == Bucket::FileUpload {
            Err(Limited {
                retry_after: std::time::Duration::from_secs(3),
            })
        } else {
            Ok(())
        }
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn each_person_uploads_at_a_rate(pool: PgPool) {
    let blobs = Arc::new(neon_app::attachments::ObjectBlobs::new(Arc::new(
        object_store::memory::InMemory::new(),
    )));
    let state = state_with_blobs(pool, blobs, Arc::new(NoUploads), Clock::system()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let page = page_in_general(&t.ada, &t.workspace, "Plan").await;

    let limited = upload(&t.ada, &page, "image", "a.png", &png()).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.code(), "rate_limited");
    assert_eq!(limited.header("retry-after"), "3");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_takes_one_of_its_own_images_as_its_cover(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let page = page_in_general(&t.ada, ws, "Plan").await;
    let other = page_in_general(&t.ada, ws, "Other").await;
    let image = upload(&t.ada, &page, "image", "cover.png", &png()).await;
    let image_id = image.json()["id"].as_str().expect("an id").to_owned();
    let pdf = upload(&t.ada, &page, "file", "a.pdf", b"%PDF-1.7").await;
    let pdf_id = pdf.json()["id"].as_str().expect("an id").to_owned();
    let elsewhere = upload(&t.ada, &other, "image", "b.png", &png()).await;
    let elsewhere_id = elsewhere.json()["id"].as_str().expect("an id").to_owned();
    let patch = |body: Value| {
        let ada = &t.ada;
        let page = page.clone();
        async move {
            ada.send(Method::PATCH, &format!("/api/v1/pages/{page}"), Some(body))
                .await
        }
    };

    let themed = patch(json!({"cover": "dusk"})).await;
    assert_eq!(themed.body["cover"], "dusk");
    let covered = patch(json!({"cover_image": image_id})).await;
    assert_eq!(covered.status, StatusCode::OK, "{}", covered.body);
    assert_eq!(covered.body["cover_image"], image_id.as_str());
    assert_eq!(covered.body["cover"], Value::Null, "one cover at a time");
    let read_back = t.cy.get(&format!("/api/v1/pages/{page}")).await;
    assert_eq!(read_back.body["cover_image"], image_id.as_str());

    for refused in [pdf_id, elsewhere_id] {
        let answer = patch(json!({"cover_image": refused})).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST);
        assert_eq!(answer.body["error"]["code"], "invalid_input");
    }
    let both = patch(json!({"cover": "teal", "cover_image": image_id})).await;
    assert_eq!(both.status, StatusCode::BAD_REQUEST);
    let back = patch(json!({"cover": "teal"})).await;
    assert_eq!(back.body["cover_image"], Value::Null);
    assert_eq!(back.body["cover"], "teal");
    let again = patch(json!({"cover_image": image_id})).await;
    assert_eq!(again.body["cover_image"], image_id.as_str());
    let removed = patch(json!({"cover_image": null})).await;
    assert_eq!(removed.body["cover_image"], Value::Null);
    assert_eq!(removed.body["cover"], Value::Null);
}
