//! Redirect URIs: which a client may register, and which of them an authorization request
//! may name.
//!
//! A client registers each redirect URI it will use, and a request names one of them
//! exactly, character for character, so a code goes nowhere its client did not register.
//! The one exception is the port of a loopback address over http, which a program on the
//! person's machine picks as it starts (RFC 8252, section 7.3). Three kinds are allowed:
//!
//! - `https`, to any host;
//! - `http` to this machine alone: `127.0.0.1`, `[::1]` or `localhost`, on any port;
//! - a scheme of a native app's own (RFC 8252, section 7.1), such as
//!   `com.example.app:/callback`, but never one the browser or the system runs or reads
//!   itself, such as `javascript:`, `data:` or `file:`.
//!
//! None may hold a fragment, a user name or password, spaces or control characters.

use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

/// The longest redirect URI, in bytes.
pub const MAX_REDIRECT_URI: usize = 2048;

/// Schemes no redirect may use: the browser's own, the network's, and those that start a
/// program of the system's.
const REFUSED_SCHEMES: &[&str] = &[
    "about",
    "afp",
    "applescript",
    "blob",
    "callto",
    "data",
    "facetime",
    "file",
    "filesystem",
    "ftp",
    "hcp",
    "help",
    "intent",
    "its",
    "jar",
    "javascript",
    "mailto",
    "mk",
    "nfs",
    "res",
    "search",
    "shell",
    "sms",
    "smb",
    "steam",
    "tel",
    "vbscript",
    "view-source",
    "ws",
    "wss",
];

/// Families of schemes no redirect may use, by the start of their name.
const REFUSED_PREFIXES: &[&str] = &[
    "chrome",
    "edge",
    "itms",
    "microsoft-",
    "ms-",
    "search-",
    "windows",
    "x-",
];

/// What is wrong with a redirect URI a client registers or names, if anything.
pub fn check_redirect_uri(raw: &str) -> Result<(), String> {
    if raw.len() > MAX_REDIRECT_URI {
        return Err(format!(
            "a redirect URI has at most {MAX_REDIRECT_URI} characters"
        ));
    }
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a redirect URI holds no spaces or control characters".to_owned());
    }
    let url = Url::parse(raw).map_err(|_| format!("{raw:?} is not an absolute URI"))?;
    if url.fragment().is_some() {
        return Err(format!(
            "{raw:?} has a fragment, which a redirect URI may not"
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{raw:?} holds a user name or password"));
    }
    match url.scheme() {
        "https" => {
            if url.host_str().is_none_or(str::is_empty) {
                return Err(format!("{raw:?} names no host"));
            }
        }
        "http" => {
            if !url.host().as_ref().is_some_and(is_loopback) {
                return Err(format!(
                    "{raw:?}: http goes to this machine alone, 127.0.0.1, [::1] or localhost; \
                     use https elsewhere"
                ));
            }
        }
        scheme if refused(scheme) => {
            return Err(format!("{raw:?}: a redirect URI may not use {scheme}:"));
        }
        _ => {}
    }
    Ok(())
}

fn refused(scheme: &str) -> bool {
    REFUSED_SCHEMES.contains(&scheme)
        || REFUSED_PREFIXES
            .iter()
            .any(|prefix| scheme.starts_with(prefix))
}

fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => *name == "localhost",
        Host::Ipv4(ip) => *ip == Ipv4Addr::LOCALHOST,
        Host::Ipv6(ip) => *ip == Ipv6Addr::LOCALHOST,
    }
}

/// Whether `asked` is one of the client's `registered` redirect URIs: the same text, or,
/// over http to this machine, the same but for its port.
pub fn redirect_matches<S: AsRef<str>>(registered: &[S], asked: &str) -> bool {
    if registered.iter().any(|uri| uri.as_ref() == asked) {
        return true;
    }
    let Some(asked) = loopback_parts(asked) else {
        return false;
    };
    registered
        .iter()
        .filter_map(|uri| loopback_parts(uri.as_ref()))
        .any(|uri| uri == asked)
}

/// A loopback http URI's parts but its port: its host, path and query.
fn loopback_parts(raw: &str) -> Option<(String, String, Option<String>)> {
    let url = Url::parse(raw).ok()?;
    let host = url.host()?;
    (url.scheme() == "http" && is_loopback(&host)).then(|| {
        (
            host.to_string(),
            url.path().to_owned(),
            url.query().map(str::to_owned),
        )
    })
}

/// Where a redirect URI goes, as the consent page names it: its scheme, host and port, a
/// host with letters of other alphabets written in ASCII, so that a lookalike shows as one.
/// An app's own scheme without a host is its scheme alone.
pub fn redirect_host(raw: &str) -> String {
    let Ok(url) = Url::parse(raw) else {
        return raw.to_owned();
    };
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) if !host.is_empty() => {
            format!("{}://{host}:{port}", url.scheme())
        }
        (Some(host), None) if !host.is_empty() => format!("{}://{host}", url.scheme()),
        _ => format!("{}:", url.scheme()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_goes_anywhere() {
        for uri in [
            "https://app.example.com/callback",
            "https://app.example.com:8443/oauth/callback?client=cli",
            "https://claude.ai/api/mcp/auth_callback",
            "https://127.0.0.1/callback",
        ] {
            assert_eq!(check_redirect_uri(uri), Ok(()), "{uri}");
        }
    }

    #[test]
    fn http_goes_to_this_machine_alone() {
        for uri in [
            "http://127.0.0.1/callback",
            "http://127.0.0.1:33418/callback",
            "http://[::1]:8080/cb",
            "http://localhost:6274/oauth/callback",
            "http://LOCALHOST/cb",
        ] {
            assert_eq!(check_redirect_uri(uri), Ok(()), "{uri}");
        }
        for uri in [
            "http://app.example.com/callback",
            "http://localhost.example.com/cb",
            "http://127.0.0.2/cb",
            "http://10.0.0.1/cb",
            "http://0.0.0.0/cb",
            "http://[::2]/cb",
        ] {
            let refused = check_redirect_uri(uri).expect_err(uri);
            assert!(refused.contains("this machine alone"), "{uri}: {refused}");
        }
    }

    #[test]
    fn native_apps_use_schemes_of_their_own() {
        for uri in [
            "com.example.app:/oauth2redirect",
            "com.example.app://callback",
            "cursor://anysphere.cursor-retrieval/oauth/callback",
            "vscode://vscode.github-authentication/did-authenticate",
        ] {
            assert_eq!(check_redirect_uri(uri), Ok(()), "{uri}");
        }
    }

    #[test]
    fn schemes_the_browser_or_system_runs_are_refused() {
        for uri in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,hi",
            "file:///etc/passwd",
            "vbscript:msgbox",
            "about:blank",
            "blob:https://app.example.com/1",
            "ws://127.0.0.1/cb",
            "wss://app.example.com/cb",
            "ftp://files.example.com/cb",
            "mailto:someone@example.com",
            "tel:123",
            "intent://scan/#Intent;end",
            "view-source:https://app.example.com",
            "ms-msdt:/id",
            "search-ms:query=x",
            "x-callback:/cb",
            "chrome-extension://abc/cb",
            "windows-feedback:/cb",
            "itms-services://?action=download",
        ] {
            assert!(check_redirect_uri(uri).is_err(), "{uri}");
        }
    }

    #[test]
    fn fragments_credentials_spaces_and_bad_syntax_are_refused() {
        let long = format!("https://app.example.com/{}", "a".repeat(MAX_REDIRECT_URI));
        for (uri, why) in [
            ("https://app.example.com/cb#frag", "fragment"),
            ("https://app.example.com/cb#", "fragment"),
            ("https://user:pass@app.example.com/cb", "user name"),
            ("https://user@app.example.com/cb", "user name"),
            ("http://user@127.0.0.1/cb", "user name"),
            (" https://app.example.com/cb", "spaces"),
            ("https://app.example.com/c b", "spaces"),
            ("https://app.example.com/cb\n", "spaces"),
            ("/callback", "absolute"),
            ("app.example.com/cb", "absolute"),
            ("", "absolute"),
            ("https://", "absolute"),
            (long.as_str(), "at most"),
        ] {
            let refused = check_redirect_uri(uri).expect_err(uri);
            assert!(refused.contains(why), "{uri:?}: {refused}");
        }
    }

    #[test]
    fn requests_name_a_registered_uri_exactly() {
        let registered = [
            "https://app.example.com/callback",
            "com.example.app:/oauth2redirect",
        ];

        assert!(redirect_matches(
            &registered,
            "https://app.example.com/callback"
        ));
        assert!(redirect_matches(
            &registered,
            "com.example.app:/oauth2redirect"
        ));
        for asked in [
            "https://app.example.com/callback/",
            "https://app.example.com/Callback",
            "https://APP.example.com/callback",
            "https://app.example.com/callback?x=1",
            "https://app.example.com:443/callback",
            "https://app.example.com:8443/callback",
            "https://evil.example/callback",
            "com.example.app:/other",
            "",
        ] {
            assert!(!redirect_matches(&registered, asked), "{asked}");
        }
    }

    #[test]
    fn a_loopback_uri_matches_on_any_port() {
        let registered = ["http://127.0.0.1/callback", "http://localhost:3000/cb?x=1"];

        for asked in [
            "http://127.0.0.1/callback",
            "http://127.0.0.1:51004/callback",
            "http://localhost:4000/cb?x=1",
            "http://localhost/cb?x=1",
        ] {
            assert!(redirect_matches(&registered, asked), "{asked}");
        }
        for asked in [
            "http://127.0.0.1:51004/other",
            "http://localhost:4000/cb",
            "http://localhost:4000/cb?x=2",
            "http://[::1]:51004/callback",
            "http://localhost:51004/callback",
            "https://127.0.0.1:51004/callback",
        ] {
            assert!(!redirect_matches(&registered, asked), "{asked}");
        }
        assert!(
            !redirect_matches(&["https://127.0.0.1/cb"], "https://127.0.0.1:9/cb"),
            "only http to loopback may change its port"
        );
    }

    #[test]
    fn the_host_shown_is_ascii() {
        assert_eq!(
            redirect_host("https://app.example.com/callback?x=1"),
            "https://app.example.com"
        );
        assert_eq!(
            redirect_host("http://127.0.0.1:51004/callback"),
            "http://127.0.0.1:51004"
        );
        assert_eq!(
            redirect_host("https://cl\u{0430}ude.ai/cb"),
            "https://xn--clude-5ve.ai"
        );
        assert_eq!(
            redirect_host("com.example.app:/oauth2redirect"),
            "com.example.app:"
        );
        assert_eq!(
            redirect_host("cursor://anysphere.cursor-retrieval/oauth"),
            "cursor://anysphere.cursor-retrieval"
        );
    }
}
