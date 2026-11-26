//! Dynamic client registration (RFC 7591): what a client may say of itself.
//!
//! Every client is public, with no secret, and uses the code flow with PKCE and refresh
//! tokens. Metadata this server does not use is ignored, as the RFC says; metadata that
//! asks for another flow is refused.

use std::net::{IpAddr, Ipv6Addr};

use super::redirect::check_redirect_uri;
use super::{ErrorCode, Problem};

/// The longest client name kept, in characters. Longer names are cut.
pub const MAX_CLIENT_NAME: usize = 100;

/// The most redirect URIs a client registers.
pub const MAX_REDIRECT_URIS: usize = 10;

/// What a client that gives no name is called.
pub const UNNAMED_CLIENT: &str = "Unnamed app";

/// The grant types every client gets.
pub const GRANT_TYPES: [&str; 2] = ["authorization_code", "refresh_token"];

/// The one response type.
pub const RESPONSE_TYPES: [&str; 1] = ["code"];

text_enum! {
    /// How a client authenticates at the token endpoint: never, as every client here is
    /// public. A client that asks for another method is registered with this one, as RFC
    /// 7591 allows, and told so in the answer.
    AuthMethod {
        None => "none",
    }
}

/// What a client sends to register, the parts this server reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientMetadata {
    pub client_name: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    pub grant_types: Option<Vec<String>>,
    pub response_types: Option<Vec<String>>,
    pub token_endpoint_auth_method: Option<String>,
}

/// A client as it registers: its name, made fit to show, and its redirect URIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    pub name: String,
    pub redirect_uris: Vec<String>,
}

/// Checks what a client says of itself, and gives what is kept of it.
pub fn check_registration(metadata: &ClientMetadata) -> Result<Registered, Problem> {
    let invalid = |description: &str| Problem::new(ErrorCode::InvalidClientMetadata, description);
    let redirect_uris = metadata.redirect_uris.clone().unwrap_or_default();
    if redirect_uris.is_empty() || redirect_uris.len() > MAX_REDIRECT_URIS {
        return Err(Problem::new(
            ErrorCode::InvalidRedirectUri,
            format!("a client registers 1 to {MAX_REDIRECT_URIS} redirect_uris"),
        ));
    }
    for uri in &redirect_uris {
        check_redirect_uri(uri)
            .map_err(|refusal| Problem::new(ErrorCode::InvalidRedirectUri, refusal))?;
    }
    if let Some(grant_types) = &metadata.grant_types {
        if !grant_types
            .iter()
            .any(|grant| grant == "authorization_code")
        {
            return Err(invalid("a client here uses the authorization_code grant"));
        }
        if let Some(other) = grant_types
            .iter()
            .find(|grant| !GRANT_TYPES.contains(&grant.as_str()))
        {
            return Err(invalid(&format!(
                "the grant types are authorization_code and refresh_token, not {other:?}"
            )));
        }
    }
    if let Some(response_types) = &metadata.response_types
        && response_types.iter().any(|response| response != "code")
    {
        return Err(invalid("the one response type is code"));
    }
    Ok(Registered {
        name: client_name(metadata.client_name.as_deref()),
        redirect_uris,
    })
}

/// A client's name as kept: trimmed, without control characters or the invisible ones
/// that reorder or hide text, at most [`MAX_CLIENT_NAME`] characters, and
/// [`UNNAMED_CLIENT`] when nothing is left.
pub fn client_name(given: Option<&str>) -> String {
    let cleaned: String = given
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control() && !invisible(*c))
        .collect();
    let name: String = cleaned.trim().chars().take(MAX_CLIENT_NAME).collect();
    let name = name.trim_end();
    if name.is_empty() {
        UNNAMED_CLIENT.to_owned()
    } else {
        name.to_owned()
    }
}

/// Characters that change how text around them reads without showing: bidirectional
/// controls, zero-width ones and the byte order mark.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{061c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2069}'
            | '\u{feff}'
    )
}

/// Where a client registered from, by which its registering is rate limited and its share
/// of the clients nobody connected is counted: an IPv4 address, or the /64 network of an
/// IPv6 address, as one machine, home or office chooses its addresses within a /64 at will.
/// An IPv4 address mapped into IPv6 is the IPv4 address; text that is no address stays as
/// it is.
pub fn registration_source(ip: &str) -> String {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.to_string(),
        Ok(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let network = Ipv6Addr::from(u128::from(v6) & (u128::MAX << 64));
                format!("{network}/64")
            }
        },
        Err(_) => ip.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(redirects: &[&str]) -> ClientMetadata {
        ClientMetadata {
            client_name: Some("Claude Code".into()),
            redirect_uris: Some(redirects.iter().map(|uri| (*uri).to_owned()).collect()),
            ..ClientMetadata::default()
        }
    }

    #[test]
    fn a_public_client_registers() {
        let registered = check_registration(&ClientMetadata {
            grant_types: Some(vec!["authorization_code".into(), "refresh_token".into()]),
            response_types: Some(vec!["code".into()]),
            token_endpoint_auth_method: Some("none".into()),
            ..metadata(&["http://localhost:51004/callback"])
        });

        assert_eq!(
            registered,
            Ok(Registered {
                name: "Claude Code".into(),
                redirect_uris: vec!["http://localhost:51004/callback".into()],
            })
        );
    }

    #[test]
    fn another_auth_method_is_replaced_by_none() {
        let registered = check_registration(&ClientMetadata {
            token_endpoint_auth_method: Some("client_secret_basic".into()),
            ..metadata(&["https://app.example.com/cb"])
        });

        assert!(registered.is_ok());
        assert_eq!(AuthMethod::None.as_str(), "none");
    }

    #[test]
    fn it_registers_one_to_ten_good_redirect_uris() {
        let eleven: Vec<String> = (0..=MAX_REDIRECT_URIS)
            .map(|n| format!("https://app.example.com/cb{n}"))
            .collect();
        let eleven: Vec<&str> = eleven.iter().map(String::as_str).collect();
        for (meta, why) in [
            (
                ClientMetadata {
                    redirect_uris: None,
                    ..metadata(&[])
                },
                "1 to 10",
            ),
            (metadata(&[]), "1 to 10"),
            (metadata(&eleven), "1 to 10"),
            (
                metadata(&["https://app.example.com/cb", "javascript:alert(1)"]),
                "javascript",
            ),
            (metadata(&["http://app.example.com/cb"]), "this machine"),
        ] {
            let refused = check_registration(&meta).expect_err(why);
            assert_eq!(refused.code, ErrorCode::InvalidRedirectUri, "{why}");
            assert!(refused.description.contains(why), "{refused}");
        }
        assert!(check_registration(&metadata(&eleven[..MAX_REDIRECT_URIS])).is_ok());
    }

    #[test]
    fn other_flows_are_refused() {
        for (grants, responses) in [
            (Some(vec!["client_credentials"]), None),
            (Some(vec!["refresh_token"]), None),
            (Some(vec!["authorization_code", "implicit"]), None),
            (Some(vec!["authorization_code", "password"]), None),
            (None, Some(vec!["token"])),
            (None, Some(vec!["code", "id_token"])),
        ] {
            let meta = ClientMetadata {
                grant_types: grants
                    .clone()
                    .map(|list| list.into_iter().map(str::to_owned).collect()),
                response_types: responses
                    .clone()
                    .map(|list| list.into_iter().map(str::to_owned).collect()),
                ..metadata(&["https://app.example.com/cb"])
            };
            let refused = check_registration(&meta).expect_err("refused");
            assert_eq!(
                refused.code,
                ErrorCode::InvalidClientMetadata,
                "{grants:?} {responses:?}"
            );
        }
        let code_only = ClientMetadata {
            grant_types: Some(vec!["authorization_code".into()]),
            ..metadata(&["https://app.example.com/cb"])
        };
        assert!(check_registration(&code_only).is_ok());
    }

    #[test]
    fn a_source_is_an_ipv4_address_or_the_64_of_an_ipv6_one() {
        assert_eq!(registration_source("198.51.100.7"), "198.51.100.7");
        assert_eq!(registration_source("::ffff:198.51.100.7"), "198.51.100.7");
        assert_eq!(
            registration_source("2001:db8:1:2:aaaa:bbbb:cccc:dddd"),
            "2001:db8:1:2::/64"
        );
        assert_eq!(
            registration_source("2001:db8:1:2::1"),
            registration_source("2001:db8:1:2:ffff:ffff:ffff:ffff")
        );
        assert_ne!(
            registration_source("2001:db8:1:2::1"),
            registration_source("2001:db8:1:3::1")
        );
        assert_eq!(registration_source("::1"), "::/64");
        assert_eq!(registration_source("unknown"), "unknown");
    }

    #[test]
    fn names_are_made_fit_to_show() {
        assert_eq!(client_name(Some("  Claude Code ")), "Claude Code");
        assert_eq!(client_name(None), UNNAMED_CLIENT);
        assert_eq!(client_name(Some(" \u{200b} ")), UNNAMED_CLIENT);
        assert_eq!(client_name(Some("Line\nbreak")), "Linebreak");
        assert_eq!(
            client_name(Some("evil\u{202e}txt.exe\u{2066}")),
            "eviltxt.exe"
        );
        assert_eq!(client_name(Some("\u{feff}Cursor")), "Cursor");
        let long = "n".repeat(MAX_CLIENT_NAME + 20);
        assert_eq!(client_name(Some(&long)).chars().count(), MAX_CLIENT_NAME);
        assert_eq!(
            client_name(Some(&format!("{} x", "n".repeat(MAX_CLIENT_NAME - 1)))),
            "n".repeat(MAX_CLIENT_NAME - 1),
            "no space is left at the end of a cut name"
        );
    }
}
