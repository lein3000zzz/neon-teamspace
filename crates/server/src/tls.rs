//! TLS for the server's outgoing HTTP: rustls on ring, TLS 1.2 or later, checked against
//! the bundled web PKI roots, so it works in minimal images without a system CA store.

use std::net::IpAddr;
use std::sync::Arc;

use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use url::{Host, Url};

/// The configuration of the server's HTTP/1.1 clients: outgoing webhooks, the S3 store, and
/// the release check.
pub(crate) fn http_client_config() -> ClientConfig {
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

/// The name a server's certificate must have: the URL's host.
pub(crate) fn server_name(url: &Url) -> Option<ServerName<'static>> {
    match url.host()? {
        Host::Domain(name) => ServerName::try_from(name.trim_end_matches('.').to_owned()).ok(),
        Host::Ipv4(ip) => Some(ServerName::IpAddress(IpAddr::V4(ip).into())),
        Host::Ipv6(ip) => Some(ServerName::IpAddress(IpAddr::V6(ip).into())),
    }
}
