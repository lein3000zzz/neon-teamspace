//! Authorization requests: the parameters a client sends the browser to the authorization
//! endpoint with, checked before anyone is asked to consent.

use uuid::Uuid;

use super::redirect::{check_redirect_uri, redirect_matches};
use super::{Audience, Client, Endpoints, ErrorCode};
use crate::auth::Scope;

/// The longest `state` a client may send, in bytes.
pub const MAX_STATE: usize = 1024;

/// An authorization request as its client sent it: each parameter, if it was there.
/// Parameters this server does not know are left out, as RFC 6749 says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthorizationRequest {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub resource: Option<String>,
}

impl AuthorizationRequest {
    /// The request of a query's parameters, each named at most once (RFC 6749, section
    /// 3.1).
    pub fn from_pairs<K: AsRef<str>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self, RequestProblem> {
        let mut request = Self::default();
        for (key, value) in pairs {
            let field = match key.as_ref() {
                "response_type" => &mut request.response_type,
                "client_id" => &mut request.client_id,
                "redirect_uri" => &mut request.redirect_uri,
                "code_challenge" => &mut request.code_challenge,
                "code_challenge_method" => &mut request.code_challenge_method,
                "scope" => &mut request.scope,
                "state" => &mut request.state,
                "resource" => &mut request.resource,
                _ => continue,
            };
            if field.replace(value.into()).is_some() {
                return Err(RequestProblem::DuplicateParameter);
            }
        }
        Ok(request)
    }

    /// The parameters it was sent with, in a fixed order, to send the browser on with.
    pub fn pairs(&self) -> Vec<(&'static str, &str)> {
        [
            ("response_type", &self.response_type),
            ("client_id", &self.client_id),
            ("redirect_uri", &self.redirect_uri),
            ("code_challenge", &self.code_challenge),
            ("code_challenge_method", &self.code_challenge_method),
            ("scope", &self.scope),
            ("state", &self.state),
            ("resource", &self.resource),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.as_deref().map(|value| (key, value)))
        .collect()
    }
}

text_enum! {
    /// Why an authorization request does not work. The consent page names each; none is
    /// ever sent back to a client's redirect URI on its own, as anyone may register one.
    RequestProblem {
        /// A parameter was sent more than once.
        DuplicateParameter => "duplicate_parameter",
        MissingClientId => "missing_client_id",
        /// No client is registered with that id.
        UnknownClient => "unknown_client",
        MissingRedirectUri => "missing_redirect_uri",
        /// The redirect URI is not one the client registered.
        RedirectUriMismatch => "redirect_uri_mismatch",
        /// The response type is not `code`.
        UnsupportedResponseType => "unsupported_response_type",
        /// No PKCE challenge, or one of a method other than S256.
        PkceRequired => "pkce_required",
        /// The PKCE challenge is not a SHA-256 in base64url.
        InvalidCodeChallenge => "invalid_code_challenge",
        /// A scope that does not exist.
        InvalidScope => "invalid_scope",
        /// A state too long, or of characters other than printable ASCII.
        InvalidState => "invalid_state",
        /// A resource that is not this server's MCP endpoint or API.
        InvalidTarget => "invalid_target",
    }
}

impl RequestProblem {
    /// The OAuth error it is.
    pub fn code(self) -> ErrorCode {
        match self {
            Self::UnknownClient => ErrorCode::InvalidClient,
            Self::UnsupportedResponseType => ErrorCode::UnsupportedResponseType,
            Self::InvalidScope => ErrorCode::InvalidScope,
            Self::InvalidTarget => ErrorCode::InvalidTarget,
            Self::DuplicateParameter
            | Self::MissingClientId
            | Self::MissingRedirectUri
            | Self::RedirectUriMismatch
            | Self::PkceRequired
            | Self::InvalidCodeChallenge
            | Self::InvalidState => ErrorCode::InvalidRequest,
        }
    }

    /// What is wrong, for whoever reads it.
    pub fn description(self) -> &'static str {
        match self {
            Self::DuplicateParameter => "a parameter is sent more than once",
            Self::MissingClientId => "the request names no client_id",
            Self::UnknownClient => "no client is registered with this client_id",
            Self::MissingRedirectUri => "the request names no redirect_uri",
            Self::RedirectUriMismatch => "the redirect_uri is not one the client registered",
            Self::UnsupportedResponseType => "the response_type is code",
            Self::PkceRequired => "PKCE is required, with the S256 method",
            Self::InvalidCodeChallenge => {
                "the code_challenge is a SHA-256 of the verifier, in base64url without padding"
            }
            Self::InvalidScope => "the scopes are read, write and admin",
            Self::InvalidState => "the state has at most 1024 characters, each printable ASCII",
            Self::InvalidTarget => {
                "the resource is this server's MCP endpoint, /mcp, or its API, /api/v1"
            }
        }
    }
}

/// An authorization request that holds: its client, where the code goes, the PKCE
/// challenge its exchange must answer, the scopes asked for, if any, the client's state,
/// and the resource the tokens are for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedRequest {
    pub client_id: Uuid,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub scopes: Vec<Scope>,
    pub state: Option<String>,
    pub audience: Audience,
}

/// The client a request names, to look up.
pub fn parse_client_id(client_id: Option<&str>) -> Result<Uuid, RequestProblem> {
    let text = client_id.ok_or(RequestProblem::MissingClientId)?;
    Uuid::try_parse(text).map_err(|_| RequestProblem::UnknownClient)
}

/// Checks a request of `client`, the one its `client_id` names, in the order RFC 6749
/// lists the parameters: the redirect URI first, then everything else.
pub fn check_request(
    request: &AuthorizationRequest,
    client: &Client,
    endpoints: &Endpoints,
) -> Result<CheckedRequest, RequestProblem> {
    let redirect_uri = request
        .redirect_uri
        .as_deref()
        .ok_or(RequestProblem::MissingRedirectUri)?;
    // One registered before a rule was added is held to it too.
    if !redirect_matches(&client.redirect_uris, redirect_uri)
        || check_redirect_uri(redirect_uri).is_err()
    {
        return Err(RequestProblem::RedirectUriMismatch);
    }
    if request.response_type.as_deref() != Some("code") {
        return Err(RequestProblem::UnsupportedResponseType);
    }
    let code_challenge = request
        .code_challenge
        .as_deref()
        .ok_or(RequestProblem::PkceRequired)?;
    check_challenge(request.code_challenge_method.as_deref(), code_challenge)?;
    let scopes = match request.scope.as_deref() {
        None => Vec::new(),
        Some(text) => parse_scope(text).ok_or(RequestProblem::InvalidScope)?,
    };
    if let Some(state) = &request.state
        && !valid_state(state)
    {
        return Err(RequestProblem::InvalidState);
    }
    let audience = match request.resource.as_deref() {
        None => Audience::Mcp,
        Some(indicator) => endpoints
            .audience_of(indicator)
            .ok_or(RequestProblem::InvalidTarget)?,
    };
    Ok(CheckedRequest {
        client_id: client.id,
        redirect_uri: redirect_uri.to_owned(),
        code_challenge: code_challenge.to_owned(),
        scopes,
        state: request.state.clone(),
        audience,
    })
}

/// A PKCE challenge of the S256 method, the one method allowed: `plain` would send the
/// verifier itself through the browser.
pub fn check_challenge(method: Option<&str>, challenge: &str) -> Result<(), RequestProblem> {
    if method != Some("S256") {
        return Err(RequestProblem::PkceRequired);
    }
    // A SHA-256, 32 bytes, in base64url without padding.
    if challenge.len() == 43 && challenge.bytes().all(is_base64url) {
        Ok(())
    } else {
        Err(RequestProblem::InvalidCodeChallenge)
    }
}

fn is_base64url(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

/// Whether a code verifier is one RFC 7636 allows: 43 to 128 unreserved characters.
pub fn check_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte))
}

/// The scopes a `scope` parameter names, space-separated: each once, in [`Scope::ALL`]
/// order. `None` when it names one that does not exist; empty when it names none.
pub fn parse_scope(text: &str) -> Option<Vec<Scope>> {
    let mut scopes = text
        .split(' ')
        .filter(|name| !name.is_empty())
        .map(Scope::parse)
        .collect::<Option<Vec<_>>>()?;
    scopes.sort();
    scopes.dedup();
    Some(scopes)
}

/// A state of printable ASCII (RFC 6749, appendix A.5), short enough to keep.
fn valid_state(state: &str) -> bool {
    state.len() <= MAX_STATE && state.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::*;

    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    fn client() -> Client {
        Client {
            id: Uuid::from_u128(7),
            name: "Example CLI".into(),
            redirect_uris: vec![
                "https://app.example.com/callback".into(),
                "http://127.0.0.1/callback".into(),
            ],
            created_at: OffsetDateTime::UNIX_EPOCH,
            last_used_at: None,
        }
    }

    fn endpoints() -> Endpoints {
        Endpoints::new("https://neon.example.com")
    }

    fn request() -> AuthorizationRequest {
        AuthorizationRequest {
            response_type: Some("code".into()),
            client_id: Some(Uuid::from_u128(7).to_string()),
            redirect_uri: Some("https://app.example.com/callback".into()),
            code_challenge: Some(CHALLENGE.into()),
            code_challenge_method: Some("S256".into()),
            scope: Some("read write".into()),
            state: Some("af0ifjsldkj".into()),
            resource: Some("https://neon.example.com/mcp".into()),
        }
    }

    fn check(request: &AuthorizationRequest) -> Result<CheckedRequest, RequestProblem> {
        check_request(request, &client(), &endpoints())
    }

    #[test]
    fn a_whole_request_holds() {
        assert_eq!(
            check(&request()),
            Ok(CheckedRequest {
                client_id: Uuid::from_u128(7),
                redirect_uri: "https://app.example.com/callback".into(),
                code_challenge: CHALLENGE.into(),
                scopes: vec![Scope::Read, Scope::Write],
                state: Some("af0ifjsldkj".into()),
                audience: Audience::Mcp,
            })
        );
    }

    #[test]
    fn scope_state_and_resource_may_be_left_out() {
        let checked = check(&AuthorizationRequest {
            scope: None,
            state: None,
            resource: None,
            ..request()
        })
        .expect("holds");

        assert_eq!(checked.scopes, []);
        assert_eq!(checked.state, None);
        assert_eq!(checked.audience, Audience::Mcp);
        let api = check(&AuthorizationRequest {
            resource: Some("https://neon.example.com/api/v1".into()),
            ..request()
        });
        assert_eq!(api.map(|checked| checked.audience), Ok(Audience::Api));
    }

    #[test]
    fn the_redirect_uri_comes_first_and_must_be_registered() {
        let wrong = AuthorizationRequest {
            response_type: Some("token".into()),
            ..request()
        };
        for (redirect, problem) in [
            (None, RequestProblem::MissingRedirectUri),
            (
                Some("https://app.example.com/other"),
                RequestProblem::RedirectUriMismatch,
            ),
            (
                Some("https://evil.example/callback"),
                RequestProblem::RedirectUriMismatch,
            ),
        ] {
            let asked = AuthorizationRequest {
                redirect_uri: redirect.map(str::to_owned),
                ..wrong.clone()
            };
            assert_eq!(check(&asked), Err(problem), "{redirect:?}");
        }
        let any_port = AuthorizationRequest {
            redirect_uri: Some("http://127.0.0.1:50211/callback".into()),
            ..request()
        };
        assert!(check(&any_port).is_ok());
    }

    #[test]
    fn a_registered_uri_that_breaks_a_rule_is_refused() {
        let mut old = client();
        old.redirect_uris.push("javascript:alert(1)".into());
        let asked = AuthorizationRequest {
            redirect_uri: Some("javascript:alert(1)".into()),
            ..request()
        };

        assert_eq!(
            check_request(&asked, &old, &endpoints()),
            Err(RequestProblem::RedirectUriMismatch)
        );
    }

    #[test]
    fn only_the_code_flow_is_offered() {
        for response_type in [None, Some("token"), Some("code id_token"), Some("CODE")] {
            let asked = AuthorizationRequest {
                response_type: response_type.map(str::to_owned),
                ..request()
            };
            assert_eq!(
                check(&asked),
                Err(RequestProblem::UnsupportedResponseType),
                "{response_type:?}"
            );
        }
    }

    #[test]
    fn pkce_is_s256_alone() {
        for (method, challenge, problem) in [
            (Some("S256"), None, RequestProblem::PkceRequired),
            (None, Some(CHALLENGE), RequestProblem::PkceRequired),
            (Some("plain"), Some(CHALLENGE), RequestProblem::PkceRequired),
            (Some("s256"), Some(CHALLENGE), RequestProblem::PkceRequired),
            (
                Some("S256"),
                Some("too-short"),
                RequestProblem::InvalidCodeChallenge,
            ),
            (
                Some("S256"),
                Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM="),
                RequestProblem::InvalidCodeChallenge,
            ),
            (
                Some("S256"),
                Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw+cM"),
                RequestProblem::InvalidCodeChallenge,
            ),
        ] {
            let asked = AuthorizationRequest {
                code_challenge_method: method.map(str::to_owned),
                code_challenge: challenge.map(str::to_owned),
                ..request()
            };
            assert_eq!(check(&asked), Err(problem), "{method:?} {challenge:?}");
        }
    }

    #[test]
    fn scopes_must_exist() {
        for (scope, scopes) in [
            ("read", Some(vec![Scope::Read])),
            ("write read write", Some(vec![Scope::Read, Scope::Write])),
            ("  admin  read ", Some(vec![Scope::Read, Scope::Admin])),
            ("", Some(vec![])),
            ("read delete", None),
            ("Read", None),
            ("openid", None),
            ("read,write", None),
        ] {
            assert_eq!(parse_scope(scope), scopes, "{scope:?}");
            let asked = AuthorizationRequest {
                scope: Some(scope.into()),
                ..request()
            };
            assert_eq!(
                check(&asked).map(|checked| checked.scopes),
                scopes.ok_or(RequestProblem::InvalidScope),
                "{scope:?}"
            );
        }
    }

    #[test]
    fn the_state_is_printable_ascii_and_bounded() {
        for (state, holds) in [
            ("a".repeat(MAX_STATE), true),
            ("a b~!{}\"'<>".to_owned(), true),
            (String::new(), true),
            ("a".repeat(MAX_STATE + 1), false),
            ("line\nbreak".to_owned(), false),
            ("tab\tstop".to_owned(), false),
            ("caf\u{e9}".to_owned(), false),
        ] {
            let asked = AuthorizationRequest {
                state: Some(state.clone()),
                ..request()
            };
            assert_eq!(
                check(&asked).is_ok(),
                holds,
                "{state:?}: {:?}",
                check(&asked)
            );
            if !holds {
                assert_eq!(check(&asked), Err(RequestProblem::InvalidState));
            }
        }
    }

    #[test]
    fn the_resource_is_this_servers() {
        for resource in [
            "https://neon.example.com",
            "https://neon.example.com/api",
            "https://other.example.com/mcp",
            "urn:example:resource",
        ] {
            let asked = AuthorizationRequest {
                resource: Some(resource.into()),
                ..request()
            };
            assert_eq!(
                check(&asked),
                Err(RequestProblem::InvalidTarget),
                "{resource}"
            );
        }
    }

    #[test]
    fn the_client_id_is_a_uuid() {
        assert_eq!(parse_client_id(None), Err(RequestProblem::MissingClientId));
        assert_eq!(
            parse_client_id(Some("unknown")),
            Err(RequestProblem::UnknownClient)
        );
        assert_eq!(
            parse_client_id(Some("00000000-0000-0000-0000-000000000007")),
            Ok(Uuid::from_u128(7))
        );
    }

    #[test]
    fn verifiers_are_43_to_128_unreserved_characters() {
        assert!(check_verifier(&"a".repeat(43)));
        assert!(check_verifier(&"a".repeat(128)));
        assert!(check_verifier(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        ));
        assert!(check_verifier(&format!("{}.~-_", "a".repeat(40))));
        assert!(!check_verifier(&"a".repeat(42)));
        assert!(!check_verifier(&"a".repeat(129)));
        assert!(!check_verifier(&format!("{}+", "a".repeat(43))));
        assert!(!check_verifier(&format!("{} ", "a".repeat(43))));
    }

    #[test]
    fn parameters_are_read_once_each() {
        let read = AuthorizationRequest::from_pairs([
            ("client_id", "c"),
            ("prompt", "consent"),
            ("state", "s"),
        ])
        .expect("read");
        assert_eq!(read.client_id.as_deref(), Some("c"));
        assert_eq!(read.state.as_deref(), Some("s"));
        assert_eq!(read.pairs(), [("client_id", "c"), ("state", "s")]);
        assert_eq!(
            AuthorizationRequest::from_pairs([("scope", "read"), ("scope", "write")]),
            Err(RequestProblem::DuplicateParameter)
        );
        let unknown =
            AuthorizationRequest::from_pairs([("prompt", "a"), ("prompt", "b")]).expect("read");
        assert_eq!(
            unknown,
            AuthorizationRequest::default(),
            "unknown parameters are left out"
        );
    }

    #[test]
    fn every_problem_has_an_oauth_code_and_a_description() {
        for problem in RequestProblem::ALL {
            assert!(!problem.description().is_empty());
            let code = problem.code();
            assert!(
                matches!(
                    code,
                    ErrorCode::InvalidRequest
                        | ErrorCode::InvalidClient
                        | ErrorCode::UnsupportedResponseType
                        | ErrorCode::InvalidScope
                        | ErrorCode::InvalidTarget
                ),
                "{problem}"
            );
        }
    }
}
