//! `/api/v1/admin`: accounts, invites, requests to join and runtime settings. Every route
//! needs an admin: a session of one, or their API token with the admin scope.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::auth::{InviteSpec, NewAccount, UserUpdate};
use neon_app::settings::Change;
use neon_domain::auth::JoinRequestStatus;
use neon_domain::settings::Secret;
use neon_domain::settings::{Kind, Source};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::dto::{CodeRequest, CreatedInvite, Invite, Role, User};
use super::error::{ApiError, ErrorBody};
use super::extract::{Json as JsonBody, Path, Query};
use super::principal::Admin;

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListUsers {
    /// Accounts on the page, 1 to 200; 50 when absent.
    pub limit: Option<i64>,
    /// The `next` of the page before.
    pub after: Option<Uuid>,
}

/// A page of accounts in the order they were made.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserPage {
    pub users: Vec<User>,
    /// Pass it as `after` for the next page; absent on the last one.
    pub next: Option<Uuid>,
}

#[utoipa::path(
    get,
    path = "/admin/users",
    tag = "admin",
    summary = "List accounts",
    params(ListUsers),
    responses(
        (status = OK, description = "A page of accounts.", body = UserPage),
        (status = BAD_REQUEST, description = "invalid_input or invalid_query.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_users(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Query(query): Query<ListUsers>,
) -> Result<JsonBody<UserPage>, ApiError> {
    let page = state
        .auth
        .list_users(&admin, query.after, query.limit)
        .await?;
    Ok(JsonBody(UserPage {
        users: page.users.into_iter().map(User::from).collect(),
        next: page.next,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUser {
    #[schema(example = "ada@example.com")]
    pub email: String,
    #[schema(format = Password)]
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// `user` when absent.
    #[serde(default)]
    pub role: Option<Role>,
}

#[utoipa::path(
    post,
    path = "/admin/users",
    tag = "admin",
    summary = "Create an account",
    description = "Works whatever the registration mode, closed included. The address counts as \
                   verified.",
    request_body = CreateUser,
    responses(
        (status = CREATED, description = "The account.", body = User),
        (status = BAD_REQUEST, description = "invalid_input, invalid_email or weak_password.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = CONFLICT, description = "email_taken.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_user(
    State(state): State<AppState>,
    Admin(admin): Admin,
    JsonBody(body): JsonBody<CreateUser>,
) -> Result<(StatusCode, JsonBody<User>), ApiError> {
    let user = state
        .auth
        .create_user(
            &admin,
            NewAccount {
                email: body.email,
                password: Secret::new(body.password),
                display_name: body.display_name,
                role: body.role.unwrap_or(Role::User).into(),
            },
        )
        .await?;
    Ok((StatusCode::CREATED, JsonBody(user.into())))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateUser {
    #[serde(default)]
    pub role: Option<Role>,
    /// Disabling signs the account out everywhere.
    #[serde(default)]
    pub disabled: Option<bool>,
    /// True marks the address verified; it cannot be made false.
    #[serde(default)]
    pub email_verified: Option<bool>,
}

#[utoipa::path(
    patch,
    path = "/admin/users/{id}",
    tag = "admin",
    summary = "Change an account's role, disable or enable it, or verify its address",
    description = "Changes only what the body names. The last admin who can sign in can be \
                   neither demoted nor disabled.",
    params(("id" = Uuid, Path, description = "The account's id.")),
    request_body = UpdateUser,
    responses(
        (status = OK, description = "The account.", body = User),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "last_admin.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn update_user(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
    JsonBody(body): JsonBody<UpdateUser>,
) -> Result<JsonBody<User>, ApiError> {
    let update = UserUpdate {
        role: body.role.map(Into::into),
        disabled: body.disabled,
        email_verified: body.email_verified,
    };
    let user = state.auth.update_user(&admin, id, update).await?;
    Ok(JsonBody(user.into()))
}

#[utoipa::path(
    post,
    path = "/admin/users/{id}/verification",
    tag = "admin",
    summary = "Mail an account its verification link again",
    params(("id" = Uuid, Path, description = "The account's id.")),
    responses(
        (status = NO_CONTENT, description = "The link was made and its mail queued."),
        (status = BAD_REQUEST, description = "invalid_input: verified already.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "mail_not_set_up, saying what is missing.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited: one a minute, five a day.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn send_verification(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.auth.send_verification_link(&admin, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteList {
    /// Newest first.
    pub invites: Vec<Invite>,
    /// Whether invites can be mailed now: switched on, and mail set up.
    pub by_email: bool,
}

#[utoipa::path(
    get,
    path = "/admin/invites",
    tag = "admin",
    summary = "List invites",
    description = "Each invite with who signed up with it. Codes are not listed: copy one with \
                   `GET /admin/invites/{id}/code`.",
    responses(
        (status = OK, description = "The invites.", body = InviteList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_invites(
    State(state): State<AppState>,
    Admin(admin): Admin,
) -> Result<JsonBody<InviteList>, ApiError> {
    let now = OffsetDateTime::now_utc();
    let invites = state.auth.list_invites(&admin).await?;
    Ok(JsonBody(InviteList {
        invites: invites
            .into_iter()
            .map(|entry| Invite::new(entry.invite, entry.signups, now))
            .collect(),
        by_email: state.auth.invite_email_available(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInvite {
    /// Mails the invite there; only that address signs up with it, once. The server makes
    /// its code and its end, so it takes none of the fields below.
    #[serde(default)]
    pub email: Option<String>,
    /// 8 to 64 letters, digits, `-` and `_`, matched whatever their case; made by the
    /// server when absent.
    #[serde(default)]
    pub code: Option<String>,
    /// 1 to 1000; 1 when absent.
    #[serde(default)]
    pub max_uses: Option<i32>,
    /// After now and within a year; `auth.invite_ttl` from now when absent.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

#[utoipa::path(
    post,
    path = "/admin/invites",
    tag = "admin",
    summary = "Create an invite",
    description = "With an address, mails the invite there, naming the admin: the mail is \
                   queued with the invite, and `queued` is true.",
    request_body = CreateInvite,
    responses(
        (status = CREATED, description = "The invite and its code.", body = CreatedInvite),
        (status = BAD_REQUEST, description = "invalid_input or invalid_email.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden, insufficient_scope or invite_email_off.", body = ErrorBody),
        (status = CONFLICT, description = "invite_code_taken, or email_taken: the address has an account.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_invite(
    State(state): State<AppState>,
    Admin(admin): Admin,
    JsonBody(body): JsonBody<CreateInvite>,
) -> Result<(StatusCode, JsonBody<CreatedInvite>), ApiError> {
    let email = body.email.filter(|email| !email.trim().is_empty());
    let (created, queued) = match email {
        Some(email) => {
            if body.code.is_some() || body.max_uses.is_some() || body.expires_at.is_some() {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_input",
                    "a mailed invite admits its address once, with a code and an end the server picks",
                ));
            }
            let created = state.auth.invite_by_email(&admin, &email).await?;
            (created, Some(true))
        }
        None => {
            let spec = InviteSpec {
                code: body.code.map(Secret::new),
                max_uses: body.max_uses,
                expires_at: body.expires_at,
            };
            (state.auth.create_invite(&admin, spec).await?, None)
        }
    };
    Ok((StatusCode::CREATED, JsonBody(created_invite(created, queued))))
}

fn created_invite(created: neon_app::auth::CreatedInvite, queued: Option<bool>) -> CreatedInvite {
    CreatedInvite {
        code: created.code.expose().to_owned(),
        invite: Invite::new(created.invite, Vec::new(), OffsetDateTime::now_utc()),
        queued,
    }
}

/// What keeps a typed code from making an invite.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteCodeCheck {
    /// Its form, or another invite having it; absent when it can make one.
    pub problem: Option<String>,
}

#[utoipa::path(
    post,
    path = "/admin/invites/check",
    tag = "admin",
    summary = "Check a code as it is typed",
    request_body = CodeRequest,
    responses(
        (status = OK, description = "The problem, if any.", body = InviteCodeCheck),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn check_invite_code(
    State(state): State<AppState>,
    Admin(admin): Admin,
    JsonBody(body): JsonBody<CodeRequest>,
) -> Result<JsonBody<InviteCodeCheck>, ApiError> {
    let problem = state
        .auth
        .check_invite_code(&admin, &Secret::new(body.code))
        .await?;
    Ok(JsonBody(InviteCodeCheck {
        problem: problem.map(str::to_owned),
    }))
}

/// An invite's code, to copy again.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteCode {
    pub code: String,
}

#[utoipa::path(
    get,
    path = "/admin/invites/{id}/code",
    tag = "admin",
    summary = "Copy an invite's code again",
    description = "Opens the code's sealed copy, which is kept while a secret key is configured.",
    params(("id" = Uuid, Path, description = "The invite's id.")),
    responses(
        (status = OK, description = "The code.", body = InviteCode),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "code_unavailable: it was not kept, or its key is gone.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn invite_code(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<JsonBody<InviteCode>, ApiError> {
    let code = state.auth.invite_code_of(&admin, id).await?;
    Ok(JsonBody(InviteCode {
        code: code.expose().to_owned(),
    }))
}

#[utoipa::path(
    post,
    path = "/admin/invites/{id}/revoke",
    tag = "admin",
    summary = "Revoke an invite",
    description = "It admits no more sign-ups, and stays listed.",
    params(("id" = Uuid, Path, description = "The invite's id.")),
    responses(
        (status = NO_CONTENT, description = "Revoked."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn revoke_invite(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.auth.revoke_invite(&admin, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Where a request to join is.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JoinStatus {
    Pending,
    Approved,
    Denied,
}

/// A confirmed request to join.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JoinRequest {
    pub id: Uuid,
    pub email: String,
    /// Theirs, plain text.
    pub note: String,
    pub status: JoinStatus,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub confirmed_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub decided_at: Option<OffsetDateTime>,
    pub decided_by: Option<Uuid>,
    /// The invite an approval made.
    pub invite_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JoinRequestList {
    /// Those waiting first, then those decided in the last 30 days.
    pub requests: Vec<JoinRequest>,
}

#[utoipa::path(
    get,
    path = "/admin/join-requests",
    tag = "admin",
    summary = "List requests to join",
    description = "Only confirmed requests: an address must open its mailed link first.",
    responses(
        (status = OK, description = "The requests.", body = JoinRequestList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_join_requests(
    State(state): State<AppState>,
    Admin(admin): Admin,
) -> Result<JsonBody<JoinRequestList>, ApiError> {
    let requests = state.auth.list_join_requests(&admin).await?;
    Ok(JsonBody(JoinRequestList {
        requests: requests
            .into_iter()
            .map(|request| JoinRequest {
                status: match request.status() {
                    JoinRequestStatus::Approved => JoinStatus::Approved,
                    JoinRequestStatus::Denied => JoinStatus::Denied,
                    JoinRequestStatus::Pending | JoinRequestStatus::Unconfirmed => {
                        JoinStatus::Pending
                    }
                },
                id: request.id,
                email: request.email,
                note: request.note,
                created_at: request.created_at,
                confirmed_at: request.confirmed_at,
                decided_at: request.decided_at,
                decided_by: request.decided_by,
                invite_id: request.invite_id,
            })
            .collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/admin/join-requests/{id}/approve",
    tag = "admin",
    summary = "Approve a request to join",
    description = "Makes an invite bound to the request's address, for one sign-up, and mails it \
                   there, the mail queued with the invite. While mail is not set up the invite \
                   is made all the same, `queued` is false, and the code can be handed over.",
    params(("id" = Uuid, Path, description = "The request's id.")),
    responses(
        (status = OK, description = "The invite and its code.", body = CreatedInvite),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such request, or it is not waiting.", body = ErrorBody),
        (status = CONFLICT, description = "email_taken: the address has an account.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn approve_join(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<JsonBody<CreatedInvite>, ApiError> {
    let (created, queued) = state.auth.approve_join(&admin, id).await?;
    Ok(JsonBody(created_invite(created, Some(queued))))
}

#[utoipa::path(
    post,
    path = "/admin/join-requests/{id}/deny",
    tag = "admin",
    summary = "Deny a request to join",
    description = "The address may ask again in 30 days, and is told so by mail while \
                   `auth.request_denied_mail` is on and mail is set up.",
    params(("id" = Uuid, Path, description = "The request's id.")),
    responses(
        (status = NO_CONTENT, description = "Denied."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such request, or it is not waiting.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn deny_join(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.auth.deny_join(&admin, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Where a setting's value comes from.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettingSource {
    Env,
    Yaml,
    Database,
    Default,
}

/// A runtime setting as admins see it, with secrets redacted.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Setting {
    #[schema(example = "auth.registration")]
    pub key: String,
    /// `bool`, `integer`, `string`, `enum`, `duration` or `secret`.
    #[schema(example = "enum")]
    pub kind: String,
    /// An enum's options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    pub description: String,
    /// A secret shows as `[redacted]` when set, and empty when not.
    pub value: Json,
    pub default: Json,
    pub source: SettingSource,
    /// Set by the environment or the config file, so it cannot change here.
    pub pinned: bool,
    #[schema(example = "NEON_SETTINGS__AUTH__REGISTRATION")]
    pub env_var: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SettingList {
    /// In key order.
    pub settings: Vec<Setting>,
}

fn setting_list(state: &AppState) -> SettingList {
    SettingList {
        settings: state
            .settings
            .entries()
            .into_iter()
            .map(|entry| Setting {
                key: entry.key.to_owned(),
                kind: entry.kind.to_string(),
                options: match entry.kind {
                    Kind::Enum(options) => Some(options.iter().map(|o| (*o).to_owned()).collect()),
                    _ => None,
                },
                description: entry.description.to_owned(),
                value: entry.value,
                default: entry.default,
                pinned: entry.source.is_pinned(),
                source: match entry.source {
                    Source::Env => SettingSource::Env,
                    Source::Yaml => SettingSource::Yaml,
                    Source::Database => SettingSource::Database,
                    Source::Default => SettingSource::Default,
                },
                env_var: entry.env_var,
            })
            .collect(),
    }
}

#[utoipa::path(
    get,
    path = "/admin/settings",
    tag = "admin",
    summary = "List runtime settings",
    description = "Every setting with its value, default and where the value comes from.",
    responses(
        (status = OK, description = "The settings.", body = SettingList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_settings(
    State(state): State<AppState>,
    Admin(_): Admin,
) -> JsonBody<SettingList> {
    JsonBody(setting_list(&state))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSettings {
    /// By key: the new value, as JSON of the setting's type, or null to go back to the
    /// default.
    #[schema(value_type = Object, example = json!({"auth.registration": "open", "auth.invite_ttl": "3d"}))]
    pub changes: BTreeMap<String, Option<Json>>,
}

#[utoipa::path(
    patch,
    path = "/admin/settings",
    tag = "admin",
    summary = "Change runtime settings",
    description = "Applies the changes together, at once and without a restart, or none of \
                   them: every value is checked, then the rules across settings. Settings \
                   pinned by the environment or the config file cannot change here. Secret \
                   settings need `secrets.keys`, as they are kept encrypted.",
    request_body = UpdateSettings,
    responses(
        (status = OK, description = "The settings after the change.", body = SettingList),
        (status = BAD_REQUEST, description = "invalid_settings, with each problem.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn update_settings(
    State(state): State<AppState>,
    Admin(admin): Admin,
    JsonBody(body): JsonBody<UpdateSettings>,
) -> Result<JsonBody<SettingList>, ApiError> {
    let changes = body.changes.into_iter().map(|(key, value)| {
        let change = value.map_or(Change::Reset, Change::Set);
        (key, change)
    });
    state.settings.update(changes, Some(admin.user.id)).await?;
    Ok(JsonBody(setting_list(&state)))
}
