//! TLS for the server's outgoing HTTP: rustls on ring, TLS 1.2 or later, checked against
//! the bundled web PKI roots, so it works in minimal images without a system CA store.

use std::sync::Arc;

use tokio_rustls::rustls::{ClientConfig, RootCertStore};

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
