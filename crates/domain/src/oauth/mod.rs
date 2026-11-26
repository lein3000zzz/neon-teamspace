//! The OAuth 2.1 authorization server that MCP clients and other programs connect through:
//! the rules of its requests, with no I/O.
//!
//! - **Clients** register themselves (RFC 7591) and are all public: none gets a secret,
//!   as a program on someone's machine could not keep one, and PKCE stands in for it.
//!   What a client says of itself, its name above all, is its own word.
//! - **Authorization** is the code flow with PKCE, S256 alone. A person signed in at a
//!   browser chooses the workspace, whom the client acts as, themselves or one of their
//!   agents, and its scopes. That consent is a [`Grant`].
//! - **Tokens** are opaque: an access token of [`ACCESS_TOKEN_PREFIX`], an hour by default,
//!   and a refresh token of [`REFRESH_TOKEN_PREFIX`], 30 days by default, which works once
//!   and is replaced by a new one each time. A code or refresh token used twice ends its
//!   grant, as one of the two uses was a copy.
//!
//! Everything here is a rule over plain values: hashing, randomness and storage belong to
//! the use cases and the repositories.

mod client;
mod consent;
mod redirect;
mod request;

use std::time::Duration;

use time::OffsetDateTime;
use uuid::Uuid;

pub use client::{
    AuthMethod, ClientMetadata, GRANT_TYPES, RESPONSE_TYPES, Registered, check_registration,
    registration_source,
};
pub use consent::{
    GrantStanding, Identity, InvalidChoice, Modes, chosen_scopes, narrowed, offered_scopes,
    refreshed_scopes,
};
pub use redirect::redirect_host;
pub use request::{
    AuthorizationRequest, CheckedRequest, RequestProblem, check_request, check_verifier,
    parse_client_id,
};

use crate::auth::Scope;

/// Starts every OAuth access token, told apart from an API token's `nw_` at a glance.
pub const ACCESS_TOKEN_PREFIX: &str = "nwo_";

/// Starts every OAuth refresh token.
pub const REFRESH_TOKEN_PREFIX: &str = "nwr_";

/// How long an authorization code works: once, within a minute.
pub const CODE_TTL: Duration = Duration::from_secs(60);

/// An access token's lifetime unless the settings say otherwise, and its bounds.
pub const DEFAULT_ACCESS_TTL: Duration = Duration::from_secs(3600);
pub const ACCESS_TTL_BOUNDS: (Duration, Duration) =
    (Duration::from_secs(5 * 60), Duration::from_secs(24 * 3600));

/// A refresh token's lifetime unless the settings say otherwise, and its bounds.
pub const DEFAULT_REFRESH_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
pub const REFRESH_TTL_BOUNDS: (Duration, Duration) = (
    Duration::from_secs(3600),
    Duration::from_secs(90 * 24 * 3600),
);

/// The path of the authorization endpoint, which sends the browser on to the consent page.
pub const AUTHORIZE_PATH: &str = "/api/v1/oauth/authorize";
/// The path of the token endpoint.
pub const TOKEN_PATH: &str = "/api/v1/oauth/token";
/// The path of dynamic client registration.
pub const REGISTER_PATH: &str = "/api/v1/oauth/register";
/// The path of token revocation (RFC 7009).
pub const REVOKE_PATH: &str = "/api/v1/oauth/revoke";
/// The web app's consent page, where the authorization endpoint sends the browser.
pub const CONSENT_PATH: &str = "/oauth/consent";
/// The authorization server's metadata (RFC 8414).
pub const SERVER_METADATA_PATH: &str = "/.well-known/oauth-authorization-server";
/// The protected resource metadata (RFC 9728), of the MCP endpoint at the root.
pub const RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";
/// The MCP endpoint's metadata where RFC 9728 places it too: its path after the
/// well-known one.
pub const MCP_RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource/mcp";
/// The REST API's metadata, its path after the well-known one.
pub const API_RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource/api/v1";

text_enum! {
    /// What a token is for (RFC 8707): the resources of this server a client may name.
    /// Both are this server, which checks every token itself, so a token for either works
    /// at both, within its workspace and scopes.
    Audience {
        /// The MCP endpoint, `/mcp`, the default.
        Mcp => "mcp",
        /// The REST API, `/api/v1`.
        Api => "api",
    }
}

impl Audience {
    /// Its path below the server's public URL.
    pub fn path(self) -> &'static str {
        match self {
            Self::Mcp => "/mcp",
            Self::Api => "/api/v1",
        }
    }
}

text_enum! {
    /// The error codes of OAuth (RFC 6749, 7591, 8707), as its endpoints answer them.
    ErrorCode {
        InvalidRequest => "invalid_request",
        InvalidClient => "invalid_client",
        InvalidGrant => "invalid_grant",
        UnsupportedGrantType => "unsupported_grant_type",
        InvalidScope => "invalid_scope",
        AccessDenied => "access_denied",
        UnsupportedResponseType => "unsupported_response_type",
        InvalidTarget => "invalid_target",
        InvalidRedirectUri => "invalid_redirect_uri",
        InvalidClientMetadata => "invalid_client_metadata",
        TemporarilyUnavailable => "temporarily_unavailable",
    }
}

/// An OAuth error: its code, and a description for whoever reads the client's logs. The
/// description never holds a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {description}")]
pub struct Problem {
    pub code: ErrorCode,
    pub description: String,
}

impl Problem {
    pub fn new(code: ErrorCode, description: impl Into<String>) -> Self {
        Self {
            code,
            description: description.into(),
        }
    }
}

/// Where the server's endpoints are, below its public URL: an origin, such as
/// `https://neon.example.com`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    base: String,
}

impl Endpoints {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    /// The issuer, which the metadata and every authorization response name (RFC 9207).
    pub fn issuer(&self) -> &str {
        &self.base
    }

    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn authorization(&self) -> String {
        self.at(AUTHORIZE_PATH)
    }

    pub fn token(&self) -> String {
        self.at(TOKEN_PATH)
    }

    pub fn registration(&self) -> String {
        self.at(REGISTER_PATH)
    }

    pub fn revocation(&self) -> String {
        self.at(REVOKE_PATH)
    }

    /// The URL of a resource, as a resource indicator names it.
    pub fn resource(&self, audience: Audience) -> String {
        self.at(audience.path())
    }

    /// The resource a resource indicator names, with or without a slash at its end.
    pub fn audience_of(&self, indicator: &str) -> Option<Audience> {
        let indicator = indicator.strip_suffix('/').unwrap_or(indicator);
        Audience::ALL
            .iter()
            .copied()
            .find(|audience| self.resource(*audience) == indicator)
    }
}

/// A registered client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub id: Uuid,
    /// What it calls itself: its own word.
    pub name: String,
    /// Where it may be sent back to, as it registered them.
    pub redirect_uris: Vec<String>,
    pub created_at: OffsetDateTime,
    /// When a person last connected it, or it last used a token.
    pub last_used_at: Option<OffsetDateTime>,
}

/// A person's consent: one client, acting in one workspace as one identity, the person or
/// one of their agents, with scopes. Consenting again to the same client, workspace and
/// identity replaces its scopes. Its tokens act within it, and stop when it is revoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub id: Uuid,
    pub client_id: Uuid,
    /// Who consented, and answers for it.
    pub person_id: Uuid,
    /// Whom the client acts as: the person, or one of their agents.
    pub user_id: Uuid,
    pub workspace_id: Uuid,
    pub scopes: Vec<Scope>,
    pub audience: Audience,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
}

impl Grant {
    /// Whether the client acts as one of the person's agents rather than as them.
    pub fn acts_as_agent(&self) -> bool {
        self.user_id != self.person_id
    }
}

text_enum! {
    /// The two tokens of a grant.
    TokenKind {
        /// Sent with each request, for an hour by default.
        Access => "access",
        /// Exchanged once for a new access token and a new refresh token.
        Refresh => "refresh",
    }
}

/// The scopes as a `scope` parameter writes them: their names, space-separated.
pub fn scope_text(scopes: &[Scope]) -> String {
    scopes
        .iter()
        .map(|scope| scope.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_sit_below_the_public_url() {
        let endpoints = Endpoints::new("https://neon.example.com/");

        assert_eq!(endpoints.issuer(), "https://neon.example.com");
        assert_eq!(
            endpoints.authorization(),
            "https://neon.example.com/api/v1/oauth/authorize"
        );
        assert_eq!(
            endpoints.token(),
            "https://neon.example.com/api/v1/oauth/token"
        );
        assert_eq!(
            endpoints.registration(),
            "https://neon.example.com/api/v1/oauth/register"
        );
        assert_eq!(
            endpoints.revocation(),
            "https://neon.example.com/api/v1/oauth/revoke"
        );
        assert_eq!(
            endpoints.resource(Audience::Mcp),
            "https://neon.example.com/mcp"
        );
        assert_eq!(
            endpoints.resource(Audience::Api),
            "https://neon.example.com/api/v1"
        );
    }

    #[test]
    fn each_resources_metadata_follows_the_well_known_path() {
        for (path, audience) in [
            (MCP_RESOURCE_METADATA_PATH, Audience::Mcp),
            (API_RESOURCE_METADATA_PATH, Audience::Api),
        ] {
            assert_eq!(path, format!("{RESOURCE_METADATA_PATH}{}", audience.path()));
        }
    }

    #[test]
    fn only_the_mcp_endpoint_and_the_api_are_audiences() {
        let endpoints = Endpoints::new("https://neon.example.com");

        for (indicator, audience) in [
            ("https://neon.example.com/mcp", Some(Audience::Mcp)),
            ("https://neon.example.com/mcp/", Some(Audience::Mcp)),
            ("https://neon.example.com/api/v1", Some(Audience::Api)),
            ("https://neon.example.com/api/v1/", Some(Audience::Api)),
            ("https://neon.example.com", None),
            ("https://neon.example.com/", None),
            ("https://neon.example.com/mcp//", None),
            ("https://neon.example.com/api", None),
            ("https://neon.example.com/api/v1/pages", None),
            ("http://neon.example.com/mcp", None),
            ("https://neon.example.com.evil.example/mcp", None),
            ("https://evil.example/mcp", None),
            ("https://NEON.example.com/mcp", None),
            ("", None),
        ] {
            assert_eq!(endpoints.audience_of(indicator), audience, "{indicator:?}");
        }
    }

    #[test]
    fn scopes_are_written_space_separated() {
        assert_eq!(scope_text(&[Scope::Read, Scope::Write]), "read write");
        assert_eq!(scope_text(&[]), "");
    }

    #[test]
    fn a_grant_acts_as_an_agent_when_its_identity_is_not_the_person() {
        let at = OffsetDateTime::UNIX_EPOCH;
        let mut grant = Grant {
            id: Uuid::from_u128(1),
            client_id: Uuid::from_u128(2),
            person_id: Uuid::from_u128(3),
            user_id: Uuid::from_u128(3),
            workspace_id: Uuid::from_u128(4),
            scopes: vec![Scope::Read],
            audience: Audience::Mcp,
            created_at: at,
            updated_at: at,
            last_used_at: None,
        };

        assert!(!grant.acts_as_agent());
        grant.user_id = Uuid::from_u128(5);
        assert!(grant.acts_as_agent());
    }

    #[test]
    fn the_prefixes_are_not_an_api_tokens() {
        for prefix in [ACCESS_TOKEN_PREFIX, REFRESH_TOKEN_PREFIX] {
            assert!(!prefix.starts_with(crate::auth::token::API_TOKEN_PREFIX));
        }
        assert_ne!(ACCESS_TOKEN_PREFIX, REFRESH_TOKEN_PREFIX);
    }

    #[test]
    fn the_default_lifetimes_are_within_their_bounds() {
        let (least, most) = ACCESS_TTL_BOUNDS;
        assert!((least..=most).contains(&DEFAULT_ACCESS_TTL));
        let (least, most) = REFRESH_TTL_BOUNDS;
        assert!((least..=most).contains(&DEFAULT_REFRESH_TTL));
    }
}
