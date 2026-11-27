//! What browsers see of authentication: the session cookie, the origin check on requests
//! that change something, and the client address rate limits count.
//!
//! - The cookie is `neon_session`, or `__Host-neon_session` when `server.public_url` is
//!   https, so that a sibling subdomain cannot plant one. It is `HttpOnly`, `SameSite=Lax`,
//!   `Path=/`, `Secure` over https, and lasts until the session's absolute expiry.
//! - A request with an unsafe method and no `Authorization` header must come from the
//!   server's own origin: its `Origin`, or else its `Referer`, must match
//!   `server.public_url`, or the `Host` it was sent to when that is unset. Bearer requests
//!   are exempt, as browsers never add that header on their own, and so are posts to an
//!   incoming webhook's URL, under `/hooks/in/`, which take no cookie at all: the token in
//!   the path is what says who sends them. When a browser sends neither header,
//!   `SameSite=Lax` on the cookie is the remaining guard: it keeps the cookie off cross-site
//!   posts.
//! - A WebSocket upgrade, a `GET`, must come from the same origin as well, whenever it
//!   sends `Origin`, as browsers do.
//! - OAuth's registration, token and revocation endpoints are spared the origin check:
//!   clients in browsers of any origin call them, and they never read the session cookie,
//!   only what the request itself carries.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::header::{AUTHORIZATION, COOKIE, HOST, ORIGIN, REFERER};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ipnet::IpNet;
use neon_domain::oauth::{REGISTER_PATH, REVOKE_PATH, TOKEN_PATH};
use tracing::warn;
use url::Url;

use super::AppState;
use super::error::ApiError;

/// The paths that never read the session cookie, which other origins may post to.
const COOKIELESS_PATHS: [&str; 3] = [REGISTER_PATH, TOKEN_PATH, REVOKE_PATH];

/// The session cookie's name over plain http.
pub const SESSION_COOKIE: &str = "neon_session";

/// Its name over https.
pub const SECURE_SESSION_COOKIE: &str = "__Host-neon_session";

/// How the server is reached, from the bootstrap configuration.
#[derive(Debug, Clone)]
pub struct WebConfig {
    /// `server.public_url` as an origin, such as `https://neon.example.com`.
    origin: Option<String>,
    secure: bool,
    trusted_proxies: Vec<IpNet>,
}

impl WebConfig {
    pub fn new(public_url: Option<&Url>, trusted_proxies: Vec<IpNet>) -> Self {
        Self {
            origin: public_url.map(|url| url.origin().ascii_serialization()),
            secure: public_url.is_some_and(|url| url.scheme() == "https"),
            trusted_proxies,
        }
    }

    /// `server.public_url`'s origin, such as `https://neon.example.com`, when it is set.
    pub fn configured_origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    pub fn cookie_name(&self) -> &'static str {
        if self.secure {
            SECURE_SESSION_COOKIE
        } else {
            SESSION_COOKIE
        }
    }

    /// A `Set-Cookie` value for a session token that lasts `max_age`.
    pub fn session_cookie(&self, token: &str, max_age: Duration) -> HeaderValue {
        self.cookie(token, max_age.as_secs())
    }

    /// A `Set-Cookie` value that removes the session cookie.
    pub fn clear_cookie(&self) -> HeaderValue {
        self.cookie("", 0)
    }

    fn cookie(&self, value: &str, max_age: u64) -> HeaderValue {
        let secure = if self.secure { "; Secure" } else { "" };
        let cookie = format!(
            "{}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}",
            self.cookie_name()
        );
        let mut header = HeaderValue::from_str(&cookie)
            .unwrap_or_else(|_| HeaderValue::from_static("neon_session=; Max-Age=0"));
        header.set_sensitive(true);
        header
    }

    /// The session token from the request's cookies, if it sent one.
    pub fn session_token<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str> {
        let name = self.cookie_name();
        headers
            .get_all(COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|cookies| cookies.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .find_map(|(key, value)| (key == name && !value.is_empty()).then_some(value))
    }

    /// The client's address: the last `X-Forwarded-For` entry when the peer is a trusted
    /// proxy, else the peer itself. The last entry is the one the proxy wrote, and the only
    /// one it vouches for: an address with a port gives the port up, and an entry that is
    /// still no address counts the proxy itself, never an entry before it, which the
    /// client could have written. Without a peer, as in tests that call the router directly,
    /// it is `unknown`.
    pub fn client_ip(&self, peer: Option<IpAddr>, headers: &HeaderMap) -> String {
        let Some(peer) = peer.map(canonical) else {
            return "unknown".to_owned();
        };
        let trusted = self
            .trusted_proxies
            .iter()
            .any(|range| range.contains(&peer));
        let forwarded = trusted
            .then(|| {
                let last = headers.get_all("x-forwarded-for").iter().next_back()?;
                let entry = last.to_str().ok()?.rsplit(',').next()?;
                forwarded_address(entry.trim())
            })
            .flatten();
        forwarded.map_or(peer, canonical).to_string()
    }

    /// The server's origin as clients reach it: `server.public_url`, or, unset, the host
    /// the request was sent to, over https when a trusted proxy says it forwarded https and
    /// http otherwise.
    pub fn public_origin(&self, peer: Option<IpAddr>, headers: &HeaderMap) -> String {
        if let Some(own) = &self.origin {
            return own.clone();
        }
        let trusted = peer.map(canonical).is_some_and(|peer| {
            self.trusted_proxies
                .iter()
                .any(|range| range.contains(&peer))
        });
        let https = trusted
            && headers
                .get("x-forwarded-proto")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|proto| proto.trim().eq_ignore_ascii_case("https"));
        let scheme = if https { "https" } else { "http" };
        headers
            .get(HOST)
            .and_then(|host| host.to_str().ok())
            .and_then(|host| Url::parse(&format!("{scheme}://{host}")).ok())
            .filter(|url| {
                url.path() == "/"
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            })
            .map_or_else(
                || format!("{scheme}://localhost"),
                |url| url.origin().ascii_serialization(),
            )
    }

    /// [`public_origin`](Self::public_origin) as a request has it: from the address it came
    /// from and its headers.
    pub(crate) fn request_origin(&self, parts: &Parts) -> String {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip());
        self.public_origin(peer, &parts.headers)
    }

    /// Whether `origin`, as an `Origin` header has it, is the server's own.
    pub(crate) fn same_origin(&self, origin: &str, headers: &HeaderMap) -> bool {
        if let Some(own) = &self.origin {
            return origin.eq_ignore_ascii_case(own);
        }
        // Without a public URL, the host the browser sent the request to is the server's.
        let Some(host) = headers.get(HOST).and_then(|host| host.to_str().ok()) else {
            return false;
        };
        let Ok(url) = Url::parse(origin) else {
            return false;
        };
        let authority = match (url.host_str(), url.port()) {
            (Some(name), Some(port)) => format!("{name}:{port}"),
            (Some(name), None) => name.to_owned(),
            (None, _) => return false,
        };
        authority.eq_ignore_ascii_case(host)
    }
}

/// The address of an `X-Forwarded-For` entry: an IP address, or one with a port, as
/// `198.51.100.1:5555` or `[2001:db8::1]:443`.
fn forwarded_address(entry: &str) -> Option<IpAddr> {
    entry
        .parse::<IpAddr>()
        .ok()
        .or_else(|| entry.parse::<SocketAddr>().ok().map(|addr| addr.ip()))
}

/// An IPv4 address mapped into IPv6 is the IPv4 address, so it meets the same ranges and
/// the same rate limits.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// The address the request came from, as [`WebConfig::client_ip`] works it out.
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip());
        Ok(Self(state.web.client_ip(peer, &parts.headers)))
    }
}

/// Refuses a WebSocket upgrade from another origin, so a page elsewhere cannot open a
/// connection that carries the visitor's cookie (cross-site WebSocket hijacking): the
/// same-origin check of requests that change something does not cover upgrades, which are
/// `GET`s. Browsers always send `Origin` with an upgrade. A client that sends none is no
/// browser, and has no visitor's cookie to misuse.
pub(super) struct SameOriginSocket;

impl FromRequestParts<AppState> for SameOriginSocket {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let Some(origin) = parts.headers.get(ORIGIN) else {
            return Ok(Self);
        };
        let origin = origin.to_str().unwrap_or_default();
        if state.web.same_origin(origin, &parts.headers) {
            return Ok(Self);
        }
        warn!(origin, "cross-origin WebSocket upgrade refused");
        Err(
            ApiError::forbidden("WebSocket connections from another origin are refused")
                .into_response(),
        )
    }
}

/// Refuses requests that change something from another origin; see the module docs.
pub(super) async fn same_origin_only(
    State(web): State<Arc<WebConfig>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method();
    let safe = method.is_safe();
    if safe
        || request.headers().contains_key(AUTHORIZATION)
        || request.uri().path().starts_with(super::hooks::PREFIX)
        || COOKIELESS_PATHS.contains(&request.uri().path())
    {
        return next.run(request).await;
    }
    let headers = request.headers();
    // A header that is there but unreadable counts as another origin.
    let origin = match headers.get(ORIGIN) {
        Some(value) => Some(value.to_str().unwrap_or_default().to_owned()),
        None => headers.get(REFERER).map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|referer| Url::parse(referer).ok())
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_default()
        }),
    };
    match origin {
        Some(origin) if !web.same_origin(&origin, headers) => {
            warn!(origin, "cross-origin request refused");
            ApiError::forbidden("cross-origin requests that change something are refused")
                .into_response()
        }
        _ => next.run(request).await,
    }
}

/// The server's origin as the request's client reaches it: see
/// [`WebConfig::public_origin`].
pub(crate) struct RequestOrigin(pub(crate) String);

impl FromRequestParts<AppState> for RequestOrigin {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(state.web.request_origin(parts)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web(public_url: Option<&str>) -> WebConfig {
        let url = public_url.map(|url| Url::parse(url).expect("a URL"));
        WebConfig::new(url.as_ref(), crate::config::default_trusted_proxies())
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(*name, HeaderValue::from_str(value).expect("a header value"));
        }
        headers
    }

    #[test]
    fn https_cookies_are_host_prefixed_and_secure() {
        let secure = web(Some("https://neon.example.com"));
        let plain = web(Some("http://neon.internal"));
        let local = web(None);

        assert_eq!(
            secure.session_cookie("abc", Duration::from_secs(60)),
            "__Host-neon_session=abc; Path=/; Max-Age=60; HttpOnly; SameSite=Lax; Secure"
        );
        assert_eq!(
            plain.session_cookie("abc", Duration::from_secs(60)),
            "neon_session=abc; Path=/; Max-Age=60; HttpOnly; SameSite=Lax"
        );
        assert_eq!(
            local.clear_cookie(),
            "neon_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"
        );
        assert!(secure.clear_cookie().is_sensitive());
    }

    #[test]
    fn the_token_is_read_from_the_cookie_of_the_right_name() {
        let plain = web(None);
        let secure = web(Some("https://neon.example.com"));
        let sent = headers(&[
            ("cookie", "theme=dark; neon_session=abc"),
            ("cookie", "__Host-neon_session=xyz"),
        ]);

        assert_eq!(plain.session_token(&sent), Some("abc"));
        assert_eq!(secure.session_token(&sent), Some("xyz"));
        assert_eq!(
            plain.session_token(&headers(&[("cookie", "neon_session=")])),
            None
        );
        assert_eq!(plain.session_token(&HeaderMap::new()), None);
    }

    #[test]
    fn forwarded_addresses_count_only_from_trusted_proxies() {
        let trusted = vec!["10.0.0.2/32".parse().expect("a range")];
        let web = WebConfig::new(None, trusted);
        let forwarded = headers(&[("x-forwarded-for", "198.51.100.1, 203.0.113.9")]);
        let proxy: IpAddr = "10.0.0.2".parse().unwrap();
        let neighbour: IpAddr = "10.0.0.3".parse().unwrap();
        let stranger: IpAddr = "192.0.2.50".parse().unwrap();
        let mapped: IpAddr = "::ffff:192.0.2.50".parse().unwrap();

        assert_eq!(web.client_ip(Some(proxy), &forwarded), "203.0.113.9");
        assert_eq!(web.client_ip(Some(neighbour), &forwarded), "10.0.0.3");
        assert_eq!(web.client_ip(Some(stranger), &forwarded), "192.0.2.50");
        assert_eq!(web.client_ip(Some(mapped), &HeaderMap::new()), "192.0.2.50");
        assert_eq!(web.client_ip(Some(proxy), &HeaderMap::new()), "10.0.0.2");
        assert_eq!(web.client_ip(None, &forwarded), "unknown");
    }

    #[test]
    fn only_the_entry_the_proxy_wrote_counts() {
        let web = WebConfig::new(None, vec!["10.0.0.2/32".parse().expect("a range")]);
        let proxy: IpAddr = "10.0.0.2".parse().unwrap();
        let from =
            |value: &str| web.client_ip(Some(proxy), &headers(&[("x-forwarded-for", value)]));

        // An address with a port is the address.
        assert_eq!(from("203.0.113.7, 198.51.100.1:5555"), "198.51.100.1");
        assert_eq!(from("203.0.113.9, [2001:db8::1]:443"), "2001:db8::1");
        // What is no address counts the proxy, never what the client wrote before it.
        assert_eq!(from("203.0.113.8, unknown"), "10.0.0.2");
        assert_eq!(from("203.0.113.8, "), "10.0.0.2");
        assert_eq!(from("203.0.113.10, 198.51.100.2"), "198.51.100.2");
        // Of several headers, the last is the proxy's.
        let two = headers(&[
            ("x-forwarded-for", "203.0.113.11"),
            ("x-forwarded-for", "garbage"),
        ]);
        assert_eq!(web.client_ip(Some(proxy), &two), "10.0.0.2");
    }

    #[test]
    fn by_default_only_a_proxy_on_this_machine_is_believed() {
        let web = web(None);
        let forwarded = headers(&[("x-forwarded-for", "203.0.113.9")]);

        for local in ["127.0.0.1", "::1"] {
            let peer: IpAddr = local.parse().unwrap();
            assert_eq!(web.client_ip(Some(peer), &forwarded), "203.0.113.9");
        }
        for network in ["10.0.0.2", "172.17.0.5", "192.168.1.20", "fd00::2"] {
            let peer: IpAddr = network.parse().unwrap();
            assert_eq!(web.client_ip(Some(peer), &forwarded), network, "{network}");
        }
    }

    #[test]
    fn the_public_origin_is_the_public_url_or_else_where_the_request_went() {
        let public = web(Some("https://neon.example.com"));
        let local = web(None);
        let host = headers(&[("host", "127.0.0.1:3000"), ("x-forwarded-proto", "https")]);
        let proxy: IpAddr = "127.0.0.1".parse().unwrap();
        let stranger: IpAddr = "192.0.2.50".parse().unwrap();

        assert_eq!(
            public.public_origin(Some(stranger), &host),
            "https://neon.example.com"
        );
        assert_eq!(
            local.public_origin(Some(proxy), &host),
            "https://127.0.0.1:3000",
            "a trusted proxy says it forwarded https"
        );
        assert_eq!(
            local.public_origin(Some(stranger), &host),
            "http://127.0.0.1:3000",
            "anyone else is not believed"
        );
        assert_eq!(
            local.public_origin(None, &headers(&[("host", "Neon.Example.com:80")])),
            "http://neon.example.com"
        );
        for odd in ["evil.example/path", "user@evil.example", "", "a b"] {
            assert_eq!(
                local.public_origin(None, &headers(&[("host", odd)])),
                "http://localhost",
                "{odd:?}"
            );
        }
        assert_eq!(
            local.public_origin(None, &HeaderMap::new()),
            "http://localhost"
        );
    }

    #[test]
    fn origins_match_the_public_url_or_else_the_host() {
        let public = web(Some("https://neon.example.com"));
        let local = web(None);
        let host = headers(&[("host", "localhost:5173")]);

        assert!(public.same_origin("https://neon.example.com", &host));
        assert!(public.same_origin("https://NEON.example.com", &host));
        assert!(!public.same_origin("http://neon.example.com", &host));
        assert!(!public.same_origin("https://evil.example", &host));
        assert!(!public.same_origin("null", &host));
        assert!(local.same_origin("http://localhost:5173", &host));
        assert!(!local.same_origin("http://localhost:8080", &host));
        assert!(!local.same_origin("http://localhost:5173", &HeaderMap::new()));
    }
}
