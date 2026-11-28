//! The origin lock's rule: where a URL belongs.
//!
//! The webview loads the app's own origin, which serves the bundled picker, and the origins of
//! the servers opened from the picker since the app started, until one is removed from the
//! list. The one in use is where the window is; the others stay open to the lock because the
//! webview's history keeps their pages, and Android's back gesture walks that history without
//! asking the app. Two URLs have the same origin when their scheme, host and port are the same,
//! as the web's same-origin policy has it, after the URL parser's normalisation: the host
//! lower-cased and in ASCII, the default port left out. So `https://neon.example.com:443/x` is
//! the server `https://neon.example.com`, while `http://neon.example.com`,
//! `https://neon.example.com:8443`, `https://neon.example.com.` and
//! `https://www.neon.example.com` are other origins. A user name or password in the URL
//! changes nothing; the host after the `@` is what counts.
//!
//! What another origin gets:
//! - an `http` or `https` page, and a `mailto:` or `tel:` link, opens in the system's browser
//!   or the app that handles it, and the webview stays where it is;
//! - anything else, such as `file:`, `data:`, `javascript:` or another app's scheme, is
//!   refused.
//!
//! `blob:` URLs have the origin of the page that made them, so a file the server's page
//! saves loads as the server's. `about:blank` and `about:srcdoc` have no origin of their own
//! and load where they are asked for.
//!
//! Which page may go to the app's own origin, and how often something opens outside, is the
//! lock's state, in [`crate::lock`](mod@crate::lock).

use std::fmt;

use url::Url;

use crate::address::ServerAddress;

/// What happens to a navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// It loads in the webview: the server's origin, or the app's.
    Load(Place),
    /// The webview stays; the system opens the URL in the browser, or the app for its scheme.
    OpenOutside,
    /// The webview stays and nothing opens.
    Refuse,
}

/// Which of the two origins a URL that loads is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    App,
    Server,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Load(Place::App) => "loaded, the app's own page",
            Self::Load(Place::Server) => "loaded, on the server's origin",
            Self::OpenOutside => "opened outside the app",
            Self::Refuse => "refused",
        })
    }
}

/// A scheme, a host and a port: what the same-origin policy compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl Origin {
    /// An origin on its scheme's default port: 443 for https, 80 for http, none for the others.
    pub fn new(scheme: &str, host: &str) -> Self {
        Self {
            scheme: scheme.to_owned(),
            host: host.to_owned(),
            port: match scheme {
                "https" => Some(443),
                "http" => Some(80),
                _ => None,
            },
        }
    }

    /// The URL's origin, when it has a host to have one. For `blob:` URLs, the origin of the
    /// URL inside.
    pub fn of(url: &Url) -> Option<Self> {
        if url.scheme() == "blob" {
            return Url::parse(url.path()).ok().as_ref().and_then(Self::of);
        }
        let host = url.host_str().filter(|host| !host.is_empty())?;
        Some(Self {
            scheme: url.scheme().to_owned(),
            host: host.to_owned(),
            port: url.port_or_known_default(),
        })
    }
}

/// A server's origin, which its address always has: an `https` or `http` scheme and a host.
impl From<&ServerAddress> for Origin {
    fn from(address: &ServerAddress) -> Self {
        let url = address.url();
        Self {
            scheme: url.scheme().to_owned(),
            host: url.host_str().unwrap_or_default().to_owned(),
            port: url.port_or_known_default(),
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme, self.host)?;
        match (self.port, self.scheme.as_str()) {
            (Some(443), "https") | (Some(80), "http") | (None, _) => Ok(()),
            (Some(port), _) => write!(f, ":{port}"),
        }
    }
}

/// Decides where a navigation to `url` goes, with `app` the origin of the app's own pages and
/// `servers` those of the servers opened.
pub fn judge(url: &Url, app: &Origin, servers: &[Origin]) -> Verdict {
    if url.scheme() == "about" {
        return match url.path() {
            "blank" | "srcdoc" => Verdict::Load(Place::App),
            _ => Verdict::Refuse,
        };
    }
    let origin = Origin::of(url);
    if origin
        .as_ref()
        .is_some_and(|origin| servers.contains(origin))
    {
        return Verdict::Load(Place::Server);
    }
    if origin.as_ref() == Some(app) {
        return Verdict::Load(Place::App);
    }
    match url.scheme() {
        "https" | "http" if origin.is_some() => Verdict::OpenOutside,
        "mailto" | "tel" => Verdict::OpenOutside,
        _ => Verdict::Refuse,
    }
}

/// What a log line says of a URL: its origin, never its path or query, which may carry a
/// page's name or a token. An `about:` URL, which has neither, is named whole.
pub fn describe(url: &Url) -> String {
    match Origin::of(url) {
        Some(origin) => origin.to_string(),
        None if url.scheme() == "about" => format!("about:{}", url.path()),
        None => format!("a {}: URL", url.scheme()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: &str = "https://neon.example.com/";

    fn origin(url: &str) -> Origin {
        Origin::of(&Url::parse(url).unwrap()).unwrap()
    }

    fn app() -> Origin {
        origin("tauri://localhost")
    }

    fn verdict(url: &str) -> Verdict {
        judge(&Url::parse(url).unwrap(), &app(), &[origin(SERVER)])
    }

    #[test]
    fn the_servers_origin_loads() {
        for url in [
            "https://neon.example.com/",
            "https://neon.example.com/w/acme/p/1?x=1#top",
            "https://neon.example.com:443/login",
            "HTTPS://NEON.EXAMPLE.COM/",
            "https://neon%2Eexample.com/",
            "https:\\\\neon.example.com\\w",
            "https://ada:secret@neon.example.com/",
            "blob:https://neon.example.com/0b0e2f7a-1c1d-4c4e-9a8b-3f6a5e2d1c0b",
        ] {
            assert_eq!(verdict(url), Verdict::Load(Place::Server), "{url}");
        }
    }

    #[test]
    fn the_apps_own_pages_load() {
        for url in [
            "tauri://localhost/",
            "tauri://localhost/index.html",
            "about:blank",
            "about:srcdoc",
        ] {
            assert_eq!(verdict(url), Verdict::Load(Place::App), "{url}");
        }
        let windows = origin("http://tauri.localhost");
        let url = Url::parse("http://tauri.localhost/index.html").unwrap();
        assert_eq!(judge(&url, &windows, &[]), Verdict::Load(Place::App));
        assert_eq!(judge(&url, &app(), &[]), Verdict::OpenOutside);
    }

    #[test]
    fn other_web_origins_open_outside() {
        for url in [
            "https://example.com/",
            "https://www.neon.example.com/",
            "https://example.neon.com/",
            "https://neon.example.com.evil.example/",
            "https://neon.example.com@evil.example/",
            "https://neon.example.com:8443/",
            "http://neon.example.com/",
            "http://neon.example.com:443/",
            // The trailing dot makes another origin, with cookies of its own.
            "https://neon.example.com./",
            // A Cyrillic "е" in "neon".
            "https://nеon.example.com/",
            "https://93.184.216.34/",
            "https://[2001:db8::1]/",
            "mailto:ada@example.com",
            "tel:+15551234567",
        ] {
            assert_eq!(verdict(url), Verdict::OpenOutside, "{url}");
        }
    }

    #[test]
    fn everything_else_is_refused() {
        for url in [
            "file:///etc/passwd",
            "data:text/html,<script>alert(1)</script>",
            "javascript:alert(1)",
            "blob:https://evil.example/0b0e2f7a",
            "blob:null/0b0e2f7a",
            "about:config",
            "intent://scan/#Intent;scheme=zxing;end",
            "ms-settings:privacy",
            "ftp://example.com/",
            "ws://neon.example.com/ws",
            "tauri://evil/",
            "x-apple.systempreferences:com.apple.preference.security",
        ] {
            assert_eq!(verdict(url), Verdict::Refuse, "{url}");
        }
    }

    #[test]
    fn with_no_server_open_only_the_apps_pages_load() {
        let url = Url::parse(SERVER).unwrap();
        assert_eq!(judge(&url, &app(), &[]), Verdict::OpenOutside);
        let url = Url::parse("tauri://localhost/").unwrap();
        assert_eq!(judge(&url, &app(), &[]), Verdict::Load(Place::App));
    }

    #[test]
    fn every_server_opened_loads_and_no_other() {
        let servers = [
            origin("https://a.example"),
            origin("http://192.168.1.20:3000"),
        ];
        for (url, expected) in [
            ("https://a.example/w/acme", Verdict::Load(Place::Server)),
            ("http://192.168.1.20:3000/", Verdict::Load(Place::Server)),
            ("https://b.example/", Verdict::OpenOutside),
        ] {
            assert_eq!(
                judge(&Url::parse(url).unwrap(), &app(), &servers),
                expected,
                "{url}"
            );
        }
    }

    #[test]
    fn ports_and_ip_literals_compare_after_normalisation() {
        let server = origin("http://192.168.1.20:3000");
        for (url, expected) in [
            ("http://192.168.1.20:3000/x", Verdict::Load(Place::Server)),
            ("http://192.168.1.20/", Verdict::OpenOutside),
            ("http://192.168.1.20:3001/", Verdict::OpenOutside),
            ("https://192.168.1.20:3000/", Verdict::OpenOutside),
            ("http://3232235796:3000/", Verdict::Load(Place::Server)),
            ("http://0xc0.0xa8.1.20:3000/", Verdict::Load(Place::Server)),
            ("http://[::ffff:192.168.1.20]:3000/", Verdict::OpenOutside),
        ] {
            assert_eq!(
                judge(
                    &Url::parse(url).unwrap(),
                    &app(),
                    std::slice::from_ref(&server)
                ),
                expected,
                "{url}"
            );
        }
        let server = origin("http://[::1]:8080");
        let url = Url::parse("http://[0:0:0::1]:8080/").unwrap();
        assert_eq!(
            judge(&url, &app(), std::slice::from_ref(&server)),
            Verdict::Load(Place::Server)
        );
        let url = Url::parse("http://127.0.0.1:8080/").unwrap();
        assert_eq!(
            judge(&url, &app(), std::slice::from_ref(&server)),
            Verdict::OpenOutside
        );
    }

    #[test]
    fn an_international_server_matches_its_unicode_and_ascii_forms() {
        let server = origin("https://xn--bcher-kva.example");
        for url in [
            "https://bücher.example/",
            "https://BÜCHER.example/",
            "https://xn--bcher-kva.example/",
        ] {
            assert_eq!(
                judge(
                    &Url::parse(url).unwrap(),
                    &app(),
                    std::slice::from_ref(&server)
                ),
                Verdict::Load(Place::Server),
                "{url}"
            );
        }
    }

    #[test]
    fn a_servers_origin_is_the_origin_of_its_address() {
        for input in [
            "https://neon.example.com/w/acme",
            "https://neon.example.com:8443",
            "http://192.168.1.20:3000",
            "http://[::1]:8080",
            "https://bücher.example",
            "http://nas.local.",
        ] {
            let address = ServerAddress::parse(input).unwrap();
            let origin = Origin::from(&address);
            assert_eq!(Origin::of(address.url()).as_ref(), Some(&origin), "{input}");
            assert_eq!(origin.to_string(), address.origin(), "{input}");
        }
    }

    #[test]
    fn log_lines_name_the_origin_alone() {
        let url = Url::parse("https://neon.example.com/reset?token=secret").unwrap();
        assert_eq!(describe(&url), "https://neon.example.com");
        let url = Url::parse("http://192.168.1.20:3000/x").unwrap();
        assert_eq!(describe(&url), "http://192.168.1.20:3000");
        let url = Url::parse("mailto:ada@example.com").unwrap();
        assert_eq!(describe(&url), "a mailto: URL");
        let url = Url::parse("blob:https://neon.example.com/abc").unwrap();
        assert_eq!(describe(&url), "https://neon.example.com");
        let url = Url::parse("about:blank#x").unwrap();
        assert_eq!(describe(&url), "about:blank");
    }
}
