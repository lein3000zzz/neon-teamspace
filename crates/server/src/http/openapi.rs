//! The OpenAPI description, built from the same router that serves the API.

use utoipa::openapi::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi as _};

use super::error::ErrorBody;
use super::web::SESSION_COOKIE;

const DESCRIPTION: &str = "The HTTP API of Neon Teamspace.\n\n\
Every error response has the same body: `{\"error\": {\"code\": string, \"message\": string}}`. \
`code` is stable and meant for programs, such as `not_found` or `invalid_json`; \
`message` says what went wrong, for people. A body that is not JSON answers \
`400 invalid_json`; JSON with a field missing, unknown or of the wrong type answers \
`400 invalid_input`, its message naming the field. A request with several wrong fields, \
such as settings, also lists each in `problems`, and a refused claim lists who holds the \
row in `holders`. A rate-limited request answers 429 with `Retry-After`.\n\n\
A request authenticates with the session cookie that `POST /api/v1/auth/login` sets, or with \
`Authorization: Bearer nw_…`, an API token. A token's scopes decide what it may do: `read` \
for safe methods, `write` for the others, `admin` for `/api/v1/admin`. A token bound to a \
workspace works on that workspace's routes and sockets alone, and is refused on another's, \
on `/api/v1/account` and on `/api/v1/admin` with `token_bound`; its `admin` scope adds that \
workspace's admin work. An agent's token is always bound to the agent's workspace and acts as \
the agent, which may do only what both it and its owner may. An OAuth access token, \
`nwo_…` from `POST /api/v1/oauth/token`, is sent the same way and works as a token bound to \
the workspace its person chose, acting as them or as one of their agents. The OAuth \
endpoints for clients, `/api/v1/oauth/register`, `/token` and `/revoke`, answer errors in \
OAuth's own shape, `{\"error\": code, \"error_description\": text}`, and allow CORS from \
any origin. Deleting a workspace \
and transferring its ownership need a browser session. A request that \
changes something and carries no `Authorization` header must come from the server's own \
origin: its `Origin`, or else its `Referer`, must match the public URL.";

#[derive(utoipa::OpenApi)]
#[openapi(
    info(title = "Neon Teamspace API", description = DESCRIPTION),
    components(schemas(ErrorBody)),
    modifiers(&Security),
    tags(
        (name = "meta", description = "What the server is."),
        (name = "auth", description = "Signing up, in and out, and the mailed links. Open to anyone."),
        (name = "account", description = "The signed-in user's own account, their favorite pages, and the invitations into workspaces waiting for them."),
        (name = "workspaces", description = "Workspaces, their members, invitations and invite links."),
        (name = "agents", description = "A workspace's agents and integrations, members that act for a person there, and agents' API tokens."),
        (name = "oauth", description = "The OAuth 2.1 authorization server MCP clients and other programs connect through: client registration, the authorization code flow with PKCE, tokens, revocation, the consent page's requests, and connected apps. Its metadata is at `/.well-known/oauth-authorization-server` and `/.well-known/oauth-protected-resource`, which the docs describe."),
        (name = "webhooks", description = "Outgoing webhooks, which tell a URL, signed, of what happens to the rows an identity is assigned to, with their delivery logs; and incoming webhooks, which act as an integration on a database or page with what is posted to their URL, `POST /hooks/in/{token}`, which the docs describe."),
        (name = "teamspaces", description = "A workspace's teamspaces and their members."),
        (name = "pages", description = "Pages and their trees, sharing, and the trash."),
        (name = "comments", description = "Threads of comments on pages, on a page as a whole or on a range of its text, their replies, following a page's comments, and what a mention may name. Open pages' editors hear that a thread changed over `GET /ws/pages/{id}`, which the docs describe."),
        (name = "notifications", description = "The inbox: mentions, replies, comments on followed pages, assignments and invitations, each checked again against what its person may see; how one wants mail of it, and the link that stops that mail. The app follows it live at `GET /ws/inbox`, which the docs describe."),
        (name = "databases", description = "Databases: their properties and views, and their rows with their values. A database and its rows are pages too. Their changes are followed live at `GET /ws/databases/{id}`, which the docs describe."),
        (name = "admin", description = "Accounts, invites, requests to join, settings and workspaces' limits, for instance admins."),
    )
)]
struct ApiDoc;

/// The two ways to authenticate.
struct Security;

impl Modify for Security {
    fn modify(&self, openapi: &mut OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session_cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                SESSION_COOKIE,
                "The browser session, set by POST /api/v1/auth/login. Named \
                 __Host-neon_session when the server's public URL is https.",
            ))),
        );
        components.add_security_scheme(
            "api_token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "An API token, nw_ and 43 characters, made under /api/v1/account/tokens, or an \
                         OAuth access token, nwo_ and 43 characters, from /api/v1/oauth/token.",
                    ))
                    .build(),
            ),
        );
    }
}

/// The parts of the description that no route contributes: its info, tags and shared
/// schemas. The routes add their own entries as they register.
pub(super) fn base() -> OpenApi {
    ApiDoc::openapi()
}

/// The description of every route.
pub fn spec() -> OpenApi {
    super::api().into_openapi()
}

/// The description as committed in `api/openapi.json`: pretty-printed with a trailing
/// newline. Maps in it are sorted, so the output only changes when the API does.
pub fn spec_json() -> String {
    render(&spec())
}

pub(super) fn render(spec: &OpenApi) -> String {
    let mut json = spec
        .to_pretty_json()
        .expect("the description serializes: every key in it is a string");
    json.push('\n');
    json
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_names_the_api_its_version_and_license() {
        let info = spec().info;

        assert_eq!(info.title, "Neon Teamspace API");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        let license = info.license.expect("a license");
        assert_eq!(
            license.identifier.as_deref(),
            Some("PolyForm-Noncommercial-1.0.0")
        );
        assert!(
            info.description
                .is_some_and(|d| d.contains(r#"{"error": {"code""#))
        );
    }

    #[test]
    fn error_shape_is_a_shared_schema() {
        let components = spec().components.expect("components");

        assert!(components.schemas.contains_key("ErrorBody"));
        assert!(components.schemas.contains_key("ErrorDetail"));
    }

    #[test]
    fn routes_are_documented_under_the_api_prefix() {
        let paths: Vec<String> = spec().paths.paths.into_keys().collect();

        assert!(
            paths.iter().all(|path| path.starts_with("/api/v1/")),
            "{paths:?}"
        );
        for path in [
            "/api/v1/meta",
            "/api/v1/auth/login",
            "/api/v1/account/me",
            "/api/v1/admin/settings",
            "/api/v1/workspaces",
            "/api/v1/workspaces/{id}/teamspaces",
            "/api/v1/workspace-invite-links/join",
            "/api/v1/admin/workspaces",
            "/api/v1/workspaces/{id}/pages",
            "/api/v1/pages/{id}",
            "/api/v1/teamspaces/{id}/pages",
            "/api/v1/pages/{id}/shares",
            "/api/v1/account/favorites",
            "/api/v1/workspaces/{id}/databases",
            "/api/v1/databases/{id}",
            "/api/v1/databases/{id}/query",
            "/api/v1/databases/{id}/rows/{unique_id}",
            "/api/v1/rows/{id}/move",
        ] {
            assert!(paths.iter().any(|known| known == path), "{path} is missing");
        }
    }

    #[test]
    fn every_operation_has_an_id_of_its_own() {
        let mut ids: Vec<String> = spec()
            .paths
            .paths
            .into_values()
            .flat_map(|item| {
                [
                    item.get,
                    item.put,
                    item.post,
                    item.delete,
                    item.patch,
                    item.head,
                    item.options,
                    item.trace,
                ]
            })
            .flatten()
            .map(|operation| operation.operation_id.expect("an operation id"))
            .collect();
        let all = ids.len();
        ids.sort();
        ids.dedup();

        assert_eq!(ids.len(), all, "operation ids repeat: {ids:?}");
    }

    #[test]
    fn both_ways_to_authenticate_are_described() {
        let components = spec().components.expect("components");

        assert!(components.security_schemes.contains_key("session_cookie"));
        assert!(components.security_schemes.contains_key("api_token"));
    }

    #[test]
    fn output_is_stable() {
        let json = spec_json();

        assert_eq!(json, spec_json());
        assert!(json.ends_with("}\n"));
    }
}
