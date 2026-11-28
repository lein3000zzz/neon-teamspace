//! Attachments: uploading a file to a page, and reading one back.
//!
//! An upload is a `multipart/form-data` body with one part, `file`, to the page it is for,
//! whose kind of block the query names, so the rules, the caller's rate and the size the
//! client declares answer before any of it is read. Its bytes then stream to the blob store
//! and are never held whole; past the largest file one takes, the upload stops. The body is
//! bounded as it is read, declared or not: the form's file must start within its first
//! [`FORM_OVERHEAD`] bytes, and what follows may hold no more than the largest file and as
//! much again as the form's headers, so a form that never comes to its file, or a file past
//! the largest, stops before it fills memory.
//!
//! A file is read at `/api/v1/files/{id}`, the path documents refer to it by, with the
//! rules asked on every request: a file follows its page. Media are served for the
//! browser to play a part at a time, with `Range`. Every answer keeps the file in its own
//! sandbox: its type as its bytes said, `nosniff`, a Content-Security-Policy that runs no
//! script and treats it as a page of an origin of its own, and `attachment`, a download,
//! for everything but images, video and audio that run nothing. Caches may keep a file for
//! the browser that read it, never for anyone else, and ask again each time, so the rules
//! see every read.

use std::fmt::Write as _;
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};

use axum::BoxError;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, FromRequest as _, Multipart, Request, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
    CONTENT_SECURITY_POLICY, CONTENT_TYPE, ETAG, IF_NONE_MATCH, IF_RANGE, RANGE,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{Stream, StreamExt as _};
use neon_app::attachments::Readable;
use neon_domain::attachments::{self as domain, Ranged, byte_range};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path, Query};
use super::principal::Authenticated;
use super::transfer::{Disposition, disposition};

/// The form field that holds the file.
const FILE_FIELD: &str = "file";

/// What a form adds to the file it carries, at most: its boundary and part headers.
pub(super) const FORM_OVERHEAD: u64 = 64 * 1024;

/// The route of uploads, whose time limit is an upload's, not a request's.
pub(super) const UPLOAD_ROUTE: &str = "/api/v1/pages/{id}/files";

/// What a file's answer allows: nothing to load or run, and a sandbox of an origin of its
/// own, should a browser show it as a page all the same.
const SANDBOX: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

/// Uploads stream past the body limit of other routes: the largest file one takes bounds
/// them instead, as they come, through a [`Gate`].
pub(super) fn upload_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::disable()
}

/// Bounds a request body as it is read, so a form's reader never holds more than the bound
/// and one chunk: past it, the body stops, giving nothing more until the bound is raised,
/// and the gate says it stalled. Until [`Gate::open`], the bound is what a form's boundary
/// and part headers may take, so a form whose file did not start by then is refused. Once
/// open, it is the largest file and its form, which the file's own count stops at first.
#[derive(Debug, Clone)]
struct Gate {
    state: Arc<Mutex<GateState>>,
}

#[derive(Debug)]
struct GateState {
    passed: u64,
    bound: u64,
    /// The reader waiting for more, once the body stopped at the bound.
    parked: Option<Waker>,
}

impl Gate {
    fn new(bound: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(GateState {
                passed: 0,
                bound,
                parked: None,
            })),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lets `more` bytes through past those through already, and wakes the reader.
    fn open(&self, more: u64) {
        let mut state = self.state();
        state.bound = state.passed.saturating_add(more);
        if let Some(waker) = state.parked.take() {
            waker.wake();
        }
    }

    /// Whether the body stopped at the bound.
    fn stalled(&self) -> bool {
        self.state().parked.is_some()
    }

    fn wrap(&self, body: Body) -> Body {
        Body::from_stream(Gated {
            inner: body.into_data_stream(),
            gate: self.clone(),
        })
    }
}

/// A body through a [`Gate`].
struct Gated {
    inner: axum::body::BodyDataStream,
    gate: Gate,
}

impl Stream for Gated {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        {
            let mut state = self.gate.state();
            if state.passed >= state.bound {
                state.parked = Some(cx.waker().clone());
                return Poll::Pending;
            }
        }
        let polled = Pin::new(&mut self.inner).poll_next(cx);
        if let Poll::Ready(Some(Ok(chunk))) = &polled {
            let mut state = self.gate.state();
            state.passed = state
                .passed
                .saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        }
        polled.map(|chunk| chunk.map(|chunk| chunk.map_err(BoxError::from)))
    }
}

/// What a file is uploaded for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// An image block, or a cover: PNG, JPEG, GIF, WebP, AVIF or SVG.
    Image,
    /// A video block: MP4, WebM, Ogg or QuickTime.
    Video,
    /// An audio block: MP3, WAV, Ogg, FLAC, AAC, M4A, WebM or MP4.
    Audio,
    /// A file block: any file.
    File,
}

impl From<FileKind> for domain::FileKind {
    fn from(kind: FileKind) -> Self {
        match kind {
            FileKind::Image => Self::Image,
            FileKind::Video => Self::Video,
            FileKind::Audio => Self::Audio,
            FileKind::File => Self::File,
        }
    }
}

impl From<domain::FileKind> for FileKind {
    fn from(kind: domain::FileKind) -> Self {
        match kind {
            domain::FileKind::Image => Self::Image,
            domain::FileKind::Video => Self::Video,
            domain::FileKind::Audio => Self::Audio,
            domain::FileKind::File => Self::File,
        }
    }
}

/// A file uploaded to a page.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct File {
    pub id: Uuid,
    /// The page it was uploaded to, whose access it follows.
    pub page_id: Uuid,
    /// Where it is read, which documents refer to it by: the URL of an image, video, audio
    /// or file block, or of markdown's link.
    #[schema(example = "/api/v1/files/0193a5b2-0000-7000-8000-0000000000f1")]
    pub url: String,
    pub kind: FileKind,
    /// The name it was uploaded with, which a download is saved as.
    #[schema(example = "diagram.png")]
    pub name: String,
    /// Its type, as its bytes say.
    #[schema(example = "image/png")]
    pub content_type: String,
    /// Its size in bytes.
    pub size: u64,
    /// The SHA-256 of its bytes, in hex.
    pub sha256: String,
    /// An image's width in pixels, when its header says.
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<domain::File> for File {
    fn from(file: domain::File) -> Self {
        let (width, height) = file.dimensions.unzip();
        Self {
            url: file.path(),
            id: file.id,
            page_id: file.page_id,
            kind: file.kind.into(),
            name: file.name,
            content_type: file.media.as_str().to_owned(),
            size: file.size,
            sha256: hex(&file.sha256),
            width,
            height,
            created_at: file.created_at,
        }
    }
}

/// Bytes as lowercase hex digits, two to a byte.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UploadQuery {
    /// The kind of block the file is for, whose types it must be of.
    pub kind: FileKind,
}

/// The form of an upload.
#[derive(ToSchema)]
#[expect(
    dead_code,
    reason = "describes the form in the API's description; the handler streams it"
)]
pub struct UploadForm {
    /// The file, its name as the part's filename. At most `attachments.max_file_bytes`.
    #[schema(value_type = String, content_media_type = "application/octet-stream")]
    pub file: Vec<u8>,
}

#[utoipa::path(
    post,
    path = "/pages/{id}/files",
    tag = "files",
    summary = "Upload a file to a page",
    description = "Uploads a file for one of the page's image, video, audio or file blocks, or \
                   its cover, for those who edit the page, out of the trash. Its type comes from \
                   its bytes, whatever the client says, and must be one the kind takes. It \
                   streams to the file store as it comes, and counts against the workspace's \
                   storage limit, which refuses it once reached. Give the answer's `url` to the \
                   block, or its `id` to the page as `cover_image`. Each caller may upload 60 \
                   files at once, then one more a second, and runs 4 uploads at once; an upload \
                   may take 15 minutes, and is given up on once it sends nothing for 30 \
                   seconds.",
    params(("id" = Uuid, Path, description = "The page's id."), UploadQuery),
    request_body(content = inline(UploadForm), content_type = "multipart/form-data"),
    responses(
        (status = CREATED, description = "The file.", body = File),
        (status = BAD_REQUEST, description = "invalid_input: a form whose first part is not `file`; or invalid_body, a form that cannot be read or stopped short.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views the page and does not edit it, or it is in the trash; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "storage_limit: the workspace has no room for the file.", body = ErrorBody),
        (status = PAYLOAD_TOO_LARGE, description = "file_too_large: past attachments.max_file_bytes, said as it arrives or as declared; or payload_too_large, a form whose file does not start within its first 64 KiB.", body = ErrorBody),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "unsupported_type: the file's bytes are of a type the kind does not take.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = SERVICE_UNAVAILABLE, description = "storage_unavailable: the file store did not answer; or busy, with Retry-After: the caller runs 4 uploads already, or the server 32.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn upload_file(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<UploadQuery>,
    request: Request,
) -> Result<(StatusCode, Json<File>), ApiError> {
    let declared = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let ticket = state
        .attachments
        .admit_upload(&principal, id, query.kind.into(), declared, FORM_OVERHEAD)
        .await?;
    let gate = Gate::new(FORM_OVERHEAD);
    let request = request.map(|body| gate.wrap(body));
    let mut form = Multipart::from_request(request, &state)
        .await
        .map_err(|rejection| {
            ApiError::new(rejection.status(), "invalid_body", rejection.body_text())
        })?;
    let field = {
        // The form's reader waits for more only when the body has more to give: one that
        // stopped at the gate wants more than a form's headers take.
        let mut next = pin!(form.next_field());
        std::future::poll_fn(|cx| match next.as_mut().poll(cx) {
            Poll::Pending if gate.stalled() => Poll::Ready(Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                format!(
                    "the form's file does not start within its first {} KiB",
                    FORM_OVERHEAD / 1024
                ),
            ))),
            Poll::Pending => Poll::Pending,
            Poll::Ready(field) => Poll::Ready(field.map_err(|error| {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_body", error.body_text())
            })),
        })
        .await?
    }
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("the form has no {FILE_FIELD}"),
        )
    })?;
    if field.name() != Some(FILE_FIELD) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("the form takes one part, {FILE_FIELD}"),
        ));
    }
    // The file, and the boundary that ends it.
    gate.open(ticket.max().saturating_add(FORM_OVERHEAD));
    let name = field.file_name().unwrap_or_default().to_owned();
    let body = field.map(|chunk| chunk.map_err(|error| error.body_text()));
    let file = state.attachments.upload(ticket, &name, body).await?;
    Ok((StatusCode::CREATED, Json(file.into())))
}

#[utoipa::path(
    get,
    path = "/files/{id}",
    tag = "files",
    summary = "A file",
    description = "The file's bytes, for those who see the page it was uploaded to: a page in \
                   the trash shows to those who could edit it. Images, video and audio that run \
                   no script come inline; anything else, SVG and HTML among them, as a download. \
                   A `Range` of bytes answers `206` with that part, as media players ask; \
                   `If-None-Match` with the file's ETag answers `304`. The answer may be kept by \
                   the caller's browser alone, which asks again each time.",
    params(("id" = Uuid, Path, description = "The file's id.")),
    responses(
        (status = OK, description = "The file.", content(("application/octet-stream"))),
        (status = PARTIAL_CONTENT, description = "The part of it the Range asked for.", content(("application/octet-stream"))),
        (status = NOT_MODIFIED, description = "The file is the one the ETag names."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such file, or none the caller may see; file_missing: its bytes are gone from the file store.", body = ErrorBody),
        (status = RANGE_NOT_SATISFIABLE, description = "The Range starts past the file's end.", body = ErrorBody),
        (status = SERVICE_UNAVAILABLE, description = "storage_unavailable: the file store did not answer.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn read_file(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let readable = state.attachments.file(&principal, id).await?;
    let file = readable.file();
    let etag = format!("\"{}\"", hex(&file.sha256));
    let mut answer = HeaderMap::new();
    sandboxed(&mut answer, &readable, &etag);
    if headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|tags| matches_etag(tags, &etag))
    {
        return Ok((StatusCode::NOT_MODIFIED, answer).into_response());
    }
    let wanted = headers
        .get(RANGE)
        .and_then(|value| value.to_str().ok())
        .filter(|_| {
            // A range of another version of the file is not this one's.
            headers
                .get(IF_RANGE)
                .and_then(|value| value.to_str().ok())
                .is_none_or(|tag| tag.trim() == etag)
        })
        .map_or(Ranged::Whole, |range| byte_range(range, file.size));
    let (status, range) = match wanted {
        Ranged::Whole => (StatusCode::OK, None),
        Ranged::Part(range) => (StatusCode::PARTIAL_CONTENT, Some(range)),
        Ranged::Unsatisfiable => {
            // An error of its own, not the file: none of the file's headers but its size.
            let mut refused = ApiError::new(
                StatusCode::RANGE_NOT_SATISFIABLE,
                "range_not_satisfiable",
                format!("the file has {} bytes", file.size),
            )
            .into_response();
            let headers = refused.headers_mut();
            insert(headers, CONTENT_RANGE, &format!("bytes */{}", file.size));
            insert(headers, X_CONTENT_TYPE_OPTIONS, "nosniff");
            return Ok(refused);
        }
    };
    let length = match &range {
        Some(range) => {
            insert(
                &mut answer,
                CONTENT_RANGE,
                &format!("bytes {}-{}/{}", range.start, range.end - 1, file.size),
            );
            range.end - range.start
        }
        None => file.size,
    };
    insert(&mut answer, CONTENT_LENGTH, &length.to_string());
    let body = if length == 0 {
        Body::empty()
    } else {
        let read = state.attachments.read(&readable, range).await?;
        Body::from_stream(read.bytes)
    };
    Ok((status, answer, body).into_response())
}

/// The headers every answer with a file carries; see the module docs.
fn sandboxed(answer: &mut HeaderMap, readable: &Readable, etag: &str) {
    let file = readable.file();
    insert(answer, CONTENT_TYPE, file.media.header_value());
    let how = if file.media.is_inline() {
        Disposition::Inline
    } else {
        Disposition::Attachment
    };
    insert(answer, CONTENT_DISPOSITION, &disposition(how, &file.name));
    insert(answer, X_CONTENT_TYPE_OPTIONS, "nosniff");
    insert(answer, CONTENT_SECURITY_POLICY, SANDBOX);
    insert(
        answer,
        HeaderName::from_static("cross-origin-resource-policy"),
        "same-origin",
    );
    insert(answer, CACHE_CONTROL, "private, no-cache");
    insert(answer, ETAG, etag);
    insert(answer, ACCEPT_RANGES, "bytes");
}

fn insert(headers: &mut HeaderMap, name: HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

/// Whether an `If-None-Match` list names `etag`, or is `*`. Weak tags match too, as the
/// comparison for this header is weak.
fn matches_etag(tags: &str, etag: &str) -> bool {
    tags.split(',')
        .map(str::trim)
        .any(|tag| tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etags_match_as_if_none_match_compares_them() {
        let etag = "\"abc\"";
        assert!(matches_etag("\"abc\"", etag));
        assert!(matches_etag("\"x\", W/\"abc\"", etag));
        assert!(matches_etag("*", etag));
        assert!(!matches_etag("\"abd\"", etag));
        assert!(!matches_etag("abc", etag));
    }

    #[test]
    fn hashes_are_lowercase_hex() {
        assert_eq!(hex(&[0x00, 0xab, 0x10]), "00ab10");
    }
}
