//! A server's address as someone types it, normalised to the origin the app loads.
//!
//! - Without a scheme the address is `https://`. Only `https://` and `http://` are taken.
//! - `http://` is taken only for a server on this device or a private network, where it is
//!   [`ServerAddress::is_encrypted`] false and the picker warns before using it: a loopback,
//!   private, shared (CGNAT) or link-local IP address, or a name that only resolves inside such
//!   a network (`localhost`, `.localhost`, `.local`, `.home.arpa`, `.internal`). The rule reads
//!   the address alone and never resolves the name, so what it allows does not change with DNS.
//! - A user name or password in the address is refused: signing in happens on the server's
//!   own pages.
//! - The path, query and fragment are dropped, the default port and the host's trailing dots
//!   too, and the host is lower-cased, with an international name in its ASCII (punycode) form,
//!   so that one server always has one address.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

/// The longest address taken, in bytes.
pub const MAX_ADDRESS_LEN: usize = 2048;

/// The path of the server's metadata, which says it is a Neon Teamspace server.
const META_PATH: &str = "api/v1/meta";

/// Name suffixes that only resolve inside a private network or on this device.
const PRIVATE_NAME_SUFFIXES: [&str; 4] = [".localhost", ".local", ".home.arpa", ".internal"];

/// A server's origin: a scheme of `https` or `http`, a host and maybe a port, nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    /// The origin with a `/` path, such as `https://neon.example.com/`.
    url: Url,
}

/// Why an address cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("Enter the server's address, such as neon.example.com.")]
    Empty,
    #[error("That address is longer than {MAX_ADDRESS_LEN} characters.")]
    TooLong,
    #[error("That is not a web address ({0}).")]
    Invalid(String),
    #[error(
        "Only https:// addresses can be used, or http:// for a server on this device or your own \
         network; this one starts with {0}:."
    )]
    Scheme(String),
    #[error(
        "Leave the user name and password out of the address: you sign in on the server's own \
         page."
    )]
    Credentials,
    #[error(
        "{0} needs https://. Plain http:// is only for a server on this device or your own \
         network, such as 192.168.1.20 or nas.local, because anyone between you and the server \
         could read what you send, your password included."
    )]
    InsecurePublic(String),
}

impl AddressError {
    /// A stable name for the picker and the tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TooLong => "too_long",
            Self::Invalid(_) => "invalid",
            Self::Scheme(_) => "scheme",
            Self::Credentials => "credentials",
            Self::InsecurePublic(_) => "insecure_public",
        }
    }
}

impl ServerAddress {
    /// Normalises what someone typed or pasted, as the module's documentation sets out.
    pub fn parse(input: &str) -> Result<Self, AddressError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(AddressError::Empty);
        }
        if input.len() > MAX_ADDRESS_LEN {
            return Err(AddressError::TooLong);
        }
        let mut url = if has_scheme(input) {
            Url::parse(input)
        } else {
            Url::parse(&format!("https://{input}"))
        }
        .map_err(|error| AddressError::Invalid(error.to_string()))?;

        match url.scheme() {
            "https" | "http" => {}
            other => return Err(AddressError::Scheme(other.to_owned())),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(AddressError::Credentials);
        }
        if url.port() == Some(0) {
            return Err(AddressError::Invalid("port 0".to_owned()));
        }
        if let Some(Host::Domain(domain)) = url.host() {
            let trimmed = domain.trim_end_matches('.');
            if trimmed.is_empty() {
                return Err(AddressError::Invalid("no server name".to_owned()));
            }
            if trimmed.len() != domain.len() {
                let trimmed = trimmed.to_owned();
                url.set_host(Some(&trimmed))
                    .map_err(|error| AddressError::Invalid(error.to_string()))?;
            }
        }
        url.set_path("/");
        url.set_query(None);
        url.set_fragment(None);

        let address = Self { url };
        if !address.is_encrypted() && !address.is_private() {
            return Err(AddressError::InsecurePublic(address.host()));
        }
        Ok(address)
    }

    /// The origin, such as `https://neon.example.com` or `http://192.168.1.20:3000`.
    pub fn origin(&self) -> String {
        self.url.origin().ascii_serialization()
    }

    /// The origin's root page, which the app loads.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// `GET /api/v1/meta` on the server.
    pub fn meta_url(&self) -> Url {
        let mut url = self.url.clone();
        url.set_path(META_PATH);
        url
    }

    /// Whether the address is `https://`.
    pub fn is_encrypted(&self) -> bool {
        self.url.scheme() == "https"
    }

    /// Whether the host is this device or on a private network, by the module's rule.
    pub fn is_private(&self) -> bool {
        match self.url.host() {
            Some(Host::Ipv4(ip)) => is_private_ip(IpAddr::V4(ip)),
            Some(Host::Ipv6(ip)) => is_private_ip(IpAddr::V6(ip)),
            Some(Host::Domain(domain)) => is_private_name(domain),
            None => false,
        }
    }

    /// The host and port, as the address shows them.
    fn host(&self) -> String {
        let host = self.url.host_str().unwrap_or_default();
        match self.url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        }
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.origin())
    }
}

/// Whether the input starts with a scheme rather than a host. `localhost:3000` and
/// `neon.example.com:8443/` are a host and a port; `http:x`, `ftp://x` and `mailto:x` start
/// with a scheme.
fn has_scheme(input: &str) -> bool {
    let Some((scheme, rest)) = input.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    let is_scheme = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !is_scheme {
        return false;
    }
    if rest.starts_with("//") {
        return true;
    }
    let port_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let port = &rest[..port_end];
    port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit())
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_private_ipv4(ip),
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(mapped) => is_private_ipv4(mapped),
            None => is_private_ipv6(ip),
        },
    }
}

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    // 100.64.0.0/10, the shared address space of carrier-grade NAT and of mesh VPNs.
    let shared = a == 100 && (b & 0xc0) == 64;
    ip.is_loopback() || ip.is_private() || ip.is_link_local() || shared
}

fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    // fc00::/7, unique local addresses, and fe80::/10, link-local ones.
    let unique_local = (first & 0xfe00) == 0xfc00;
    let link_local = (first & 0xffc0) == 0xfe80;
    ip.is_loopback() || unique_local || link_local
}

fn is_private_name(domain: &str) -> bool {
    domain == "localhost"
        || PRIVATE_NAME_SUFFIXES
            .iter()
            .any(|suffix| domain.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(input: &str) -> String {
        ServerAddress::parse(input)
            .unwrap_or_else(|error| panic!("{input:?} was refused: {error}"))
            .origin()
    }

    fn refusal(input: &str) -> AddressError {
        match ServerAddress::parse(input) {
            Ok(address) => panic!("{input:?} was taken as {address}"),
            Err(error) => error,
        }
    }

    #[test]
    fn an_address_without_a_scheme_is_https() {
        assert_eq!(origin("neon.example.com"), "https://neon.example.com");
        assert_eq!(
            origin("neon.example.com:8443"),
            "https://neon.example.com:8443"
        );
        assert_eq!(
            origin("neon.example.com:8443/w/acme"),
            "https://neon.example.com:8443"
        );
        assert_eq!(origin("localhost:3000"), "https://localhost:3000");
        assert_eq!(origin("192.168.1.20"), "https://192.168.1.20");
        assert_eq!(origin("[::1]:8080"), "https://[::1]:8080");
    }

    #[test]
    fn surrounding_space_is_ignored() {
        assert_eq!(origin("  neon.example.com \n"), "https://neon.example.com");
    }

    #[test]
    fn the_path_query_and_fragment_are_dropped() {
        let address = ServerAddress::parse("https://neon.example.com/w/acme/p/1?x=1#top").unwrap();
        assert_eq!(address.url().as_str(), "https://neon.example.com/");
        assert_eq!(
            address.meta_url().as_str(),
            "https://neon.example.com/api/v1/meta"
        );
    }

    #[test]
    fn scheme_and_host_are_lower_cased_and_the_default_port_dropped() {
        assert_eq!(
            origin("HTTPS://Neon.Example.COM:443/"),
            "https://neon.example.com"
        );
        assert_eq!(origin("http://LOCALHOST:80"), "http://localhost");
        assert_eq!(
            origin("https://neon.example.com:80"),
            "https://neon.example.com:80"
        );
    }

    #[test]
    fn trailing_dots_are_dropped_from_the_host() {
        assert_eq!(
            origin("https://neon.example.com./"),
            "https://neon.example.com"
        );
        assert_eq!(origin("neon.example.com.."), "https://neon.example.com");
        assert_eq!(refusal("https://./").code(), "invalid");
    }

    #[test]
    fn an_international_name_is_kept_in_its_ascii_form() {
        assert_eq!(
            origin("https://bücher.example"),
            "https://xn--bcher-kva.example"
        );
        assert_eq!(origin("BÜCHER.example"), "https://xn--bcher-kva.example");
        // A Cyrillic "е" in place of the Latin one is another name altogether.
        let lookalike = origin("nеon.example.com");
        assert!(lookalike.starts_with("https://xn--"), "{lookalike}");
        assert_ne!(lookalike, "https://neon.example.com");
    }

    #[test]
    fn ip_literals_are_normalised() {
        assert_eq!(origin("http://127.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(origin("http://0x7f.0.0.1"), "http://127.0.0.1");
        assert_eq!(origin("http://[0:0::1]:8080"), "http://[::1]:8080");
        assert_eq!(origin("https://[2001:DB8::1]"), "https://[2001:db8::1]");
    }

    #[test]
    fn a_scheme_other_than_http_or_https_is_refused() {
        for (input, scheme) in [
            ("ftp://neon.example.com", "ftp"),
            ("ws://neon.example.com", "ws"),
            ("file:///etc/hosts", "file"),
            ("javascript:alert(1)", "javascript"),
            ("mailto:admin@example.com", "mailto"),
            ("data:text/html,hi", "data"),
            ("tauri://localhost", "tauri"),
        ] {
            assert_eq!(
                refusal(input),
                AddressError::Scheme(scheme.to_owned()),
                "{input}"
            );
        }
    }

    #[test]
    fn credentials_are_refused() {
        assert_eq!(
            refusal("https://ada@neon.example.com"),
            AddressError::Credentials
        );
        assert_eq!(
            refusal("https://ada:secret@neon.example.com"),
            AddressError::Credentials
        );
        assert_eq!(
            refusal("https://:secret@neon.example.com"),
            AddressError::Credentials
        );
    }

    #[test]
    fn what_is_not_an_address_is_refused() {
        assert_eq!(refusal(""), AddressError::Empty);
        assert_eq!(refusal("   "), AddressError::Empty);
        assert_eq!(
            refusal(&"a".repeat(MAX_ADDRESS_LEN + 1)),
            AddressError::TooLong
        );
        for input in [
            "https://",
            "neon example.com",
            "https://neon.example.com:99999",
            "https://neon.example.com:0",
            "https://[::1",
            "https://[fe80::1%25en0]",
            "http://300.1.1.1",
        ] {
            assert_eq!(refusal(input).code(), "invalid", "{input}");
        }
    }

    #[test]
    fn http_is_taken_for_loopback_and_private_networks() {
        for input in [
            "http://localhost:8080",
            "http://neon.localhost",
            "http://127.0.0.1",
            "http://127.8.9.10:3000",
            "http://[::1]:3000",
            "http://10.0.0.5",
            "http://172.16.0.1",
            "http://172.31.255.255",
            "http://192.168.1.20:3000",
            "http://169.254.10.10",
            "http://100.64.0.1",
            "http://100.127.255.254",
            "http://[fd12:3456::1]",
            "http://[fe80::1]",
            "http://[::ffff:192.168.1.20]",
            "http://nas.local",
            "http://neon.home.arpa",
            "http://neon.corp.internal",
            "http://NAS.LOCAL.",
        ] {
            let address = ServerAddress::parse(input)
                .unwrap_or_else(|error| panic!("{input} was refused: {error}"));
            assert!(!address.is_encrypted(), "{input}");
            assert!(address.is_private(), "{input}");
        }
    }

    #[test]
    fn http_is_refused_for_everything_else() {
        for (input, host) in [
            ("http://neon.example.com", "neon.example.com"),
            ("http://neon.example.com:8080", "neon.example.com:8080"),
            ("http://8.8.8.8", "8.8.8.8"),
            ("http://172.32.0.1", "172.32.0.1"),
            ("http://100.128.0.1", "100.128.0.1"),
            ("http://[2001:db8::1]", "[2001:db8::1]"),
            ("http://[::ffff:8.8.8.8]", "[::ffff:808:808]"),
            ("http://0.0.0.0", "0.0.0.0"),
            ("http://local", "local"),
            ("http://notlocal", "notlocal"),
            ("http://nas.local.example.com", "nas.local.example.com"),
            ("http://localhost.example.com", "localhost.example.com"),
            ("http://myinternal", "myinternal"),
        ] {
            assert_eq!(
                refusal(input),
                AddressError::InsecurePublic(host.to_owned()),
                "{input}"
            );
        }
    }

    #[test]
    fn https_is_taken_anywhere() {
        for input in [
            "https://neon.example.com",
            "https://192.168.1.20",
            "https://nas.local",
        ] {
            assert!(
                ServerAddress::parse(input).unwrap().is_encrypted(),
                "{input}"
            );
        }
    }

    #[test]
    fn a_host_and_port_is_not_mistaken_for_a_scheme() {
        assert!(!has_scheme("localhost:3000"));
        assert!(!has_scheme("neon.example.com:8443/x"));
        assert!(!has_scheme("neon.example.com:8443?x"));
        assert!(!has_scheme("192.168.1.20:3000"));
        assert!(!has_scheme("[::1]:3000"));
        assert!(has_scheme("http:neon.example.com"));
        assert!(has_scheme("https://neon.example.com"));
        assert!(has_scheme("mailto:ada@example.com"));
        assert!(has_scheme("neon:"));
    }
}
