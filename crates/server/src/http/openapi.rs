//! The OpenAPI description, built from the same router that serves the API.

use utoipa::openapi::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi as _};

use super::error::ErrorBody;
use super::web::SESSION_COOKIE;

const DESCRIPTION: &str = "The HTTP API of Neon Teamspace.\n\n\
Every error response has the same body: `{\"error\": {\"code\": string, \"message\": string}}`. \
`code` is stable and meant for programs, such as `not_found` or `invalid_json`; \
`message` says what went wrong, for people. A request with several wrong fields, such as \
settings, also lists each in `problems`. A rate-limited request answers 429 with \
`Retry-After`.\n\n\
A request authenticates with the session cookie that `POST /api/v1/auth/login` sets, or with \
`Authorization: Bearer nw_…`, an API token. A token's scopes decide what it may do: `read` \
for safe methods, `write` for the others, `admin` for `/api/v1/admin`. Deleting a workspace \
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
        (name = "teamspaces", description = "A workspace's teamspaces and their members."),
        (name = "pages", description = "Pages and their trees, sharing, and the trash."),
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
                        "An API token, nw_ and 43 characters, made under /api/v1/account/tokens.",
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
