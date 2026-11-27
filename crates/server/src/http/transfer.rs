//! Importing pages from markdown, and exporting pages and teamspaces as markdown.
//!
//! An import is a `multipart/form-data` upload of one file, a markdown file or a zip of
//! them, whose place the query string names, so the rules and the caller's limits answer
//! before the file is read; the server's own limit of transfers running answers once it is.
//! Exports answer with the file itself, to download.

use std::convert::Infallible;

use axum::body::Body;
use axum::extract::multipart::MultipartRejection;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use neon_app::transfer::{Exported, ImportReport, Limits, SkipReason as AppSkipReason, Upload};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path, Query};
use super::pages::{Page, location};
use super::principal::Authenticated;

/// The form field that holds the file to import.
const FILE_FIELD: &str = "file";

/// What a form adds to the file it carries, at most: its boundaries and part headers.
const FORM_OVERHEAD: usize = 64 * 1024;

/// The body limit of the import route: the largest upload and its form around it.
pub(super) fn import_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(Limits::default().max_upload + FORM_OVERHEAD)
}

/// Where an import goes: name one.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ImportPlace {
    /// The page of the workspace to import under, which the caller edits.
    pub parent_id: Option<Uuid>,
    /// The teamspace to import at the top of, which the caller is in.
    pub teamspace_id: Option<Uuid>,
    /// True to import among the caller's private pages.
    #[serde(default)]
    pub private: bool,
}

/// The form of an import.
#[derive(ToSchema)]
#[expect(
    dead_code,
    reason = "describes the form in the API's description; the handler reads it part by part"
)]
pub struct ImportForm {
    /// A markdown file, named `.md` or `.markdown`, or a zip of them, named `.zip`, at most
    /// 32 MiB.
    #[schema(value_type = String, content_media_type = "application/octet-stream")]
    pub file: Vec<u8>,
}

/// Why a file of a zip was left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Not a markdown file, such as an image, and no page links to it by a relative path.
    Attachment,
    /// A file a page links to whose type its block does not take, such as a PDF in an
    /// image.
    UnsupportedType,
    /// A file a page links to that is larger than an upload may be.
    TooLarge,
    /// Its name, or a folder's it is in, starts with a dot.
    Hidden,
    /// A symbolic link.
    Link,
    /// Encrypted.
    Encrypted,
    /// Compressed otherwise than deflated or stored.
    UnsupportedCompression,
    /// Not text in UTF-8.
    NotText,
    /// A second markdown file for one page, such as `Notes.markdown` beside `Notes.md`.
    Duplicate,
}

impl From<AppSkipReason> for SkipReason {
    fn from(reason: AppSkipReason) -> Self {
        match reason {
            AppSkipReason::Attachment => Self::Attachment,
            AppSkipReason::UnsupportedType => Self::UnsupportedType,
            AppSkipReason::TooLarge => Self::TooLarge,
            AppSkipReason::Hidden => Self::Hidden,
            AppSkipReason::Link => Self::Link,
            AppSkipReason::Encrypted => Self::Encrypted,
            AppSkipReason::Compression => Self::UnsupportedCompression,
            AppSkipReason::NotText => Self::NotText,
            AppSkipReason::Duplicate => Self::Duplicate,
        }
    }
}

/// A file of a zip left out of an import.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkippedFile {
    /// Its path in the zip.
    #[schema(example = "Roadmap/diagram.png")]
    pub path: String,
    pub reason: SkipReason,
}

/// What an import made.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImportResult {
    /// The pages made where the import went, in order. The pages under them are listed by
    /// the trees.
    pub pages: Vec<Page>,
    /// How many pages were made, those under them included.
    pub count: u64,
    /// The files of a zip left out, in the zip's order.
    pub skipped: Vec<SkippedFile>,
}

impl From<ImportReport> for ImportResult {
    fn from(report: ImportReport) -> Self {
        Self {
            pages: report.pages.into_iter().map(Page::from).collect(),
            count: u64::try_from(report.count).unwrap_or(u64::MAX),
            skipped: report
                .skipped
                .into_iter()
                .map(|skipped| SkippedFile {
                    path: skipped.path,
                    reason: skipped.reason.into(),
                })
                .collect(),
        }
    }
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/pages/import",
    tag = "pages",
    summary = "Import pages from markdown",
    description = "Makes pages from a markdown file, or from a zip of them whose folders become \
                   pages with the pages of their files under them. A folder and a markdown file of \
                   its name beside it are one page. A file's first heading is its page's title, \
                   else its name. The pages go last under a page the caller edits, at the top of \
                   a teamspace they are in, or among their private pages, as the query names; \
                   nothing is made unless all of it can be. Other files of a zip, such as images, \
                   are left out and listed. Each caller may import 10 times, then once more \
                   every 30 seconds.",
    params(("id" = Uuid, Path, description = "The workspace's id."), ImportPlace),
    request_body(content = inline(ImportForm), content_type = "multipart/form-data"),
    responses(
        (status = CREATED, description = "The pages made.", body = ImportResult),
        (status = BAD_REQUEST, description = "invalid_input: no place or several, no file or several, a file that is not markdown or a zip, not UTF-8, a zip that cannot be read or holds no markdown file, or an entry whose path leads outside the zip; or invalid_body, a form that cannot be read.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views the parent and does not edit it, is not in the teamspace, or is a guest importing private pages; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace, teamspace or parent, or none the caller may see.", body = ErrorBody),
        (status = CONFLICT, description = "too_deep: the pages would nest deeper than 100 levels.", body = ErrorBody),
        (status = PAYLOAD_TOO_LARGE, description = "too_large: past a limit of imports, which the message names; or payload_too_large, a form larger than the largest upload.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = SERVICE_UNAVAILABLE, description = "busy: too many imports and exports are running, on the server or for the caller, with Retry-After; or timeout: the import took longer than 20 seconds.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn import_pages(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(place): Query<ImportPlace>,
    form: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, Json<ImportResult>), ApiError> {
    let location = location(place.parent_id, place.teamspace_id, place.private)?;
    let mut form = form.map_err(|rejection| {
        ApiError::new(rejection.status(), "invalid_body", rejection.body_text())
    })?;
    let ticket = state
        .transfer
        .admit_import(&principal, id, location)
        .await?;
    let upload = read_upload(&mut form).await?;
    let report = state.transfer.import(ticket, upload).await?;
    Ok((StatusCode::CREATED, Json(report.into())))
}

/// The one file of an import's form, read whole: the body limit bounds it.
async fn read_upload(form: &mut Multipart) -> Result<Upload, ApiError> {
    let unreadable = |error: axum::extract::multipart::MultipartError| {
        let status = error.status();
        if status == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(status, "payload_too_large", error.body_text())
        } else if status.is_server_error() {
            tracing::warn!(%error, "an import's form could not be read");
            ApiError::new(StatusCode::BAD_REQUEST, "invalid_body", error.body_text())
        } else {
            ApiError::new(status, "invalid_body", error.body_text())
        }
    };
    let mut upload = None;
    while let Some(field) = form.next_field().await.map_err(unreadable)? {
        if field.name() != Some(FILE_FIELD) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                format!("the form takes one part, {FILE_FIELD}"),
            ));
        }
        let name = field.file_name().unwrap_or_default().to_owned();
        let bytes = field.bytes().await.map_err(unreadable)?;
        if upload.replace(Upload { name, bytes }).is_some() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                "import one file at a time: a zip holds several",
            ));
        }
    }
    upload.ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("the form has no {FILE_FIELD}"),
        )
    })
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExportPage {
    /// True for a zip of the page and the pages under it; a markdown file of the page alone
    /// when false or absent.
    #[serde(default)]
    pub children: bool,
}

#[utoipa::path(
    get,
    path = "/pages/{id}/export",
    tag = "pages",
    summary = "Export a page as markdown",
    description = "The page as a markdown file, its title as its first heading. With \
                   `children=true`, a zip of its file and, in a folder of the same name beside \
                   it, those of the pages under it, each page's folder holding the files of the \
                   pages under that one: the pages out of the trash, or for a page in the trash, \
                   those that went there with it. What markdown cannot hold, such as colours, is \
                   left out. Each caller may export 10 times, then once more every 30 \
                   seconds.",
    params(("id" = Uuid, Path, description = "The page's id."), ExportPage),
    responses(
        (status = OK, description = "The file, as an attachment named after the page.", content(
            ("text/markdown"),
            ("application/zip"),
        )),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = PAYLOAD_TOO_LARGE, description = "too_large: more than 5000 pages, more than 10000 rows of databases besides them, or more than 128 MiB of markdown.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = SERVICE_UNAVAILABLE, description = "busy: too many imports and exports are running, on the server or for the caller, with Retry-After; or timeout: the export took longer than 20 seconds.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn export_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<ExportPage>,
) -> Result<Response, ApiError> {
    let export = state
        .transfer
        .export_page(&principal, id, query.children)
        .await?;
    Ok(download(export))
}

#[utoipa::path(
    get,
    path = "/teamspaces/{id}/export",
    tag = "pages",
    summary = "Export a teamspace as markdown",
    description = "A zip of the teamspace's pages out of the trash, for everyone who may list \
                   them: each page's markdown file, its title as its first heading, the pages at \
                   the top of the teamspace at the top of the zip, and the files of the pages \
                   under a page in a folder of the same name beside its file. Guests get \
                   `not_found`. Each caller may export 10 times, then once more every 30 \
                   seconds.",
    params(("id" = Uuid, Path, description = "The teamspace's id.")),
    responses(
        (status = OK, description = "The zip, as an attachment named after the teamspace.", content(
            ("application/zip"),
        )),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = PAYLOAD_TOO_LARGE, description = "too_large: more than 5000 pages, more than 10000 rows of databases besides them, or more than 128 MiB of markdown.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = SERVICE_UNAVAILABLE, description = "busy: too many imports and exports are running, on the server or for the caller, with Retry-After; or timeout: the export took longer than 20 seconds.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn export_teamspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let export = state.transfer.export_teamspace(&principal, id).await?;
    Ok(download(export))
}

/// How much of an export's file goes to the connection at a time.
const DOWNLOAD_CHUNK: usize = 64 * 1024;

/// An export as an attachment to save, named as its file, never kept by a cache. Its body
/// keeps the export's place and turn until it is sent whole, or the connection closes, so
/// a client that reads slowly holds them as long as the file it holds in memory.
fn download(Exported { export, hold }: Exported) -> Response {
    let length = export.bytes.len();
    let bytes = Bytes::from(export.bytes);
    let chunks = (0..length).step_by(DOWNLOAD_CHUNK).map(move |at| {
        let _held = &hold;
        Ok::<_, Infallible>(bytes.slice(at..length.min(at + DOWNLOAD_CHUNK)))
    });
    let mut response = Body::from_stream(futures_util::stream::iter(chunks)).into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(export.format.content_type()),
    );
    if let Ok(value) = HeaderValue::from_str(&disposition(Disposition::Attachment, &export.name)) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// How a browser takes a file it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Disposition {
    /// Shown in place.
    Inline,
    /// Saved.
    Attachment,
}

/// `Content-Disposition` for a file named `name`: in ASCII for old clients, `_` in place of
/// what ASCII lacks, and in full by RFC 6266's `filename*`, percent-encoded.
pub(super) fn disposition(how: Disposition, name: &str) -> String {
    let how = match how {
        Disposition::Inline => "inline",
        Disposition::Attachment => "attachment",
    };
    let ascii: String = name
        .chars()
        .map(|c| match c {
            ' '..='~' if c != '"' && c != '\\' && c != '%' => c,
            _ => '_',
        })
        .collect();
    let mut encoded = String::with_capacity(name.len() * 3);
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("{how}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_survive_the_header() {
        let disposition = |name| disposition(Disposition::Attachment, name);
        assert_eq!(
            disposition("Roadmap.md"),
            "attachment; filename=\"Roadmap.md\"; filename*=UTF-8''Roadmap.md"
        );
        assert_eq!(
            super::disposition(Disposition::Inline, "photo.png"),
            "inline; filename=\"photo.png\"; filename*=UTF-8''photo.png"
        );
        assert_eq!(
            disposition("Q1 Planung für 2027.zip"),
            "attachment; filename=\"Q1 Planung f_r 2027.zip\"; \
             filename*=UTF-8''Q1%20Planung%20f%C3%BCr%202027.zip"
        );
        assert_eq!(
            disposition("50% \"done\".md"),
            "attachment; filename=\"50_ _done_.md\"; filename*=UTF-8''50%25%20%22done%22.md"
        );
        assert!(HeaderValue::from_str(&disposition("Notes 🗺.md")).is_ok());
    }
}
