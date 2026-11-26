//! The inbox: `/api/v1/inbox`, `/api/v1/notifications/{id}`, how one wants mail of it at
//! `/api/v1/me/notifications`, and `/api/v1/unsubscribe`, which stops that mail from a link
//! without signing in.
//!
//! Every route but the last needs a browser session or an API token: `read` for `GET`,
//! `write` for the rest. A token bound to a workspace reads that workspace's notifications
//! alone. Each notification is checked again, as it is listed and counted, against what its
//! person may see now: one about a page they lost is left out.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::notifications::{InboxPage, InboxQuery, MailSettings, Shown};
use neon_domain::notifications as domain;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::account::InvitingWorkspace;
use super::dto::{Person, WorkspaceRole};
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path, Query};
use super::pages::Page;
use super::principal::Authenticated;

/// What a notification tells of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// A page's text, or a comment on it, mentions the person.
    Mention,
    /// Someone replied in a thread they wrote in or were mentioned in.
    Reply,
    /// Someone commented on a page they follow.
    Comment,
    /// They were added to a person property of a row.
    Assigned,
    /// A workspace invites them to join it.
    Invitation,
}

impl From<domain::NotificationKind> for NotificationKind {
    fn from(kind: domain::NotificationKind) -> Self {
        match kind {
            domain::NotificationKind::Mention => Self::Mention,
            domain::NotificationKind::Reply => Self::Reply,
            domain::NotificationKind::Comment => Self::Comment,
            domain::NotificationKind::Assigned => Self::Assigned,
            domain::NotificationKind::Invitation => Self::Invitation,
        }
    }
}

/// An invitation a notification is, while it waits for an answer, which
/// `POST /me/invitations/{id}/accept` and `/decline` give.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotifiedInvitation {
    pub id: Uuid,
    pub role: WorkspaceRole,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// A notification as its person reads it now.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Notification {
    pub id: Uuid,
    pub kind: NotificationKind,
    /// The workspace it is in, or the one an invitation invites to.
    pub workspace: InvitingWorkspace,
    /// Who did what it tells of; absent once their account is gone.
    pub actor: Option<Person>,
    /// The page it is about, as its person sees it now; absent for an invitation.
    pub page: Option<Page>,
    /// The thread and comment it is about, for a comment's notification.
    pub thread_id: Option<Uuid>,
    pub comment_id: Option<Uuid>,
    /// For a mention in a page's text, the block it is in.
    pub block_id: Option<String>,
    /// For an assignment, the person property.
    pub property_id: Option<Uuid>,
    /// The start of the comment it is about.
    pub excerpt: Option<String>,
    pub invitation: Option<NotifiedInvitation>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub read_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub archived_at: Option<OffsetDateTime>,
}

impl From<Shown> for Notification {
    fn from(shown: Shown) -> Self {
        let notification = shown.notification;
        let actor = shown.actor.map(|actor| {
            let mut actor = Person::from(actor);
            if shown.members_hidden {
                actor.email = None;
            }
            actor
        });
        Self {
            id: notification.id,
            kind: notification.kind.into(),
            workspace: InvitingWorkspace::new(&shown.workspace),
            actor,
            page: shown.page.map(Page::from),
            thread_id: notification.thread_id,
            comment_id: notification.comment_id,
            block_id: notification.block_id,
            property_id: notification.property_id,
            excerpt: shown.excerpt,
            invitation: shown.invitation.map(|invitation| NotifiedInvitation {
                id: invitation.id,
                role: invitation.role.into(),
                expires_at: invitation.expires_at,
            }),
            created_at: notification.created_at,
            read_at: notification.read_at,
            archived_at: notification.archived_at,
        }
    }
}

/// A part of an inbox, newest first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotificationList {
    pub notifications: Vec<Notification>,
    /// Pass it back as `before` for the next part; absent at the end.
    pub next: Option<Uuid>,
}

impl From<InboxPage> for NotificationList {
    fn from(page: InboxPage) -> Self {
        Self {
            notifications: page
                .notifications
                .into_iter()
                .map(Notification::from)
                .collect(),
            next: page.next,
        }
    }
}

/// Which notifications a list holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InboxFilter {
    /// Every one not archived, read or not.
    #[default]
    All,
    /// Those not read yet, nor archived.
    Unread,
    /// The archived ones.
    Archived,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct ListInbox {
    /// One workspace's notifications, with every waiting invitation; every workspace's
    /// when absent.
    pub workspace_id: Option<Uuid>,
    /// `all` when absent.
    #[param(inline)]
    pub filter: Option<InboxFilter>,
    /// The `next` of the part before.
    pub before: Option<Uuid>,
    /// Notifications in the part, 1 to 200; 50 when absent.
    pub limit: Option<usize>,
}

impl From<ListInbox> for InboxQuery {
    fn from(query: ListInbox) -> Self {
        let filter = query.filter.unwrap_or_default();
        Self {
            workspace_id: query.workspace_id,
            unread: filter == InboxFilter::Unread,
            archived: filter == InboxFilter::Archived,
            before: query.before,
            limit: query.limit,
        }
    }
}

#[utoipa::path(
    get,
    path = "/inbox",
    tag = "notifications",
    summary = "Read the inbox",
    description = "The caller's notifications, newest first, a part at a time: from every \
                   workspace they are in, or the one named, and their invitations into \
                   workspaces while those wait. A token bound to a workspace reads that \
                   workspace's alone, without invitations.",
    params(ListInbox),
    responses(
        (status = OK, description = "A part of the inbox.", body = NotificationList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: a workspace the caller is not in.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_inbox(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Query(query): Query<ListInbox>,
) -> Result<Json<NotificationList>, ApiError> {
    let page = state.inbox.inbox(&principal, query.into()).await?;
    Ok(Json(page.into()))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct InboxCountQuery {
    /// One workspace's, with every waiting invitation; every workspace's when absent.
    pub workspace_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InboxCount {
    /// How many notifications are unread, at most 999.
    pub unread: usize,
}

#[utoipa::path(
    get,
    path = "/inbox/count",
    tag = "notifications",
    summary = "Count the unread notifications",
    params(InboxCountQuery),
    responses(
        (status = OK, description = "The count.", body = InboxCount),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn inbox_count(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Query(query): Query<InboxCountQuery>,
) -> Result<Json<InboxCount>, ApiError> {
    let unread = state
        .inbox
        .unread_count(&principal, query.workspace_id)
        .await?;
    Ok(Json(InboxCount { unread }))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkAllRead {
    /// One workspace's, with every invitation; every workspace's when absent.
    #[serde(default)]
    pub workspace_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Marked {
    /// How many were marked.
    pub marked: u64,
}

#[utoipa::path(
    post,
    path = "/inbox/read",
    tag = "notifications",
    summary = "Mark every notification read",
    request_body = MarkAllRead,
    responses(
        (status = OK, description = "How many were marked.", body = Marked),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn mark_all_read(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Json(body): Json<MarkAllRead>,
) -> Result<Json<Marked>, ApiError> {
    let marked = state
        .inbox
        .mark_all_read(&principal, body.workspace_id)
        .await?;
    Ok(Json(Marked { marked }))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NotificationChange {
    /// Marks it read, or unread again.
    #[serde(default)]
    pub read: Option<bool>,
    /// Archives it, or brings it back to the inbox.
    #[serde(default)]
    pub archived: Option<bool>,
}

#[utoipa::path(
    patch,
    path = "/notifications/{id}",
    tag = "notifications",
    summary = "Mark a notification",
    description = "Read or unread, archived or not.",
    params(("id" = Uuid, Path, description = "The notification's id.")),
    request_body = NotificationChange,
    responses(
        (status = NO_CONTENT, description = "Marked."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn change_notification(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<NotificationChange>,
) -> Result<StatusCode, ApiError> {
    state
        .inbox
        .mark(&principal, id, body.read, body.archived)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// How someone wants mail of their inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MailPreference {
    /// No mail.
    None,
    /// A mail for each notification left unread a while after it came.
    Each,
    /// A digest every few hours of what came and is unread.
    Digest,
}

impl From<domain::MailPreference> for MailPreference {
    fn from(mail: domain::MailPreference) -> Self {
        match mail {
            domain::MailPreference::None => Self::None,
            domain::MailPreference::Each => Self::Each,
            domain::MailPreference::Digest => Self::Digest,
        }
    }
}

impl From<MailPreference> for domain::MailPreference {
    fn from(mail: MailPreference) -> Self {
        match mail {
            MailPreference::None => Self::None,
            MailPreference::Each => Self::Each,
            MailPreference::Digest => Self::Digest,
        }
    }
}

/// How the caller wants mail of their inbox, and whether the instance sends it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotificationSettings {
    pub mail: MailPreference,
    /// The instance sends notification mail now; off, the preference waits until it does.
    pub mail_available: bool,
    /// How often, at most, a digest goes, in seconds.
    pub digest_interval_secs: u64,
}

impl From<MailSettings> for NotificationSettings {
    fn from(settings: MailSettings) -> Self {
        Self {
            mail: settings.preference.into(),
            mail_available: settings.available,
            digest_interval_secs: settings.digest_interval.as_secs(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/me/notifications",
    tag = "notifications",
    summary = "Read how one wants mail of the inbox",
    responses(
        (status = OK, description = "The preference.", body = NotificationSettings),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_notification_settings(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
) -> Result<Json<NotificationSettings>, ApiError> {
    let settings = state.inbox.mail_settings(&principal).await?;
    Ok(Json(settings.into()))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NotificationSettingsChange {
    pub mail: MailPreference,
}

#[utoipa::path(
    put,
    path = "/me/notifications",
    tag = "notifications",
    summary = "Choose how one wants mail of the inbox",
    description = "For people; agents and integrations get no mail.",
    request_body = NotificationSettingsChange,
    responses(
        (status = OK, description = "The preference.", body = NotificationSettings),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: an agent or integration; or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn put_notification_settings(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Json(body): Json<NotificationSettingsChange>,
) -> Result<Json<NotificationSettings>, ApiError> {
    let settings = state.inbox.set_mail(&principal, body.mail.into()).await?;
    Ok(Json(settings.into()))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UnsubscribeQuery {
    /// The token of the link in the mail.
    pub token: String,
}

#[utoipa::path(
    post,
    path = "/unsubscribe",
    tag = "notifications",
    summary = "Stop notification mail from its link",
    description = "Without signing in: the token in the mail's link names whom it is for, \
                   signed by the server. Their preference becomes `none`. Mail clients post \
                   here as RFC 8058 has it, `List-Unsubscribe=One-Click`, which any body \
                   does.",
    params(UnsubscribeQuery),
    responses(
        (status = NO_CONTENT, description = "Mail stopped."),
        (status = BAD_REQUEST, description = "invalid_token.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(())
)]
pub(super) async fn unsubscribe(
    State(state): State<AppState>,
    Query(query): Query<UnsubscribeQuery>,
) -> Result<StatusCode, ApiError> {
    state.inbox.unsubscribe(&query.token).await?;
    Ok(StatusCode::NO_CONTENT)
}
