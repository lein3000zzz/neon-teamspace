//! The [`WebhookTransport`] port over HTTP/1.1, with hyper on rustls.
//!
//! Each call resolves the URL's host once, through a [`Resolve`], within
//! [`CONNECT_TIMEOUT`], and checks every address it gets with the address filter; any one
//! refused refuses the call. It then connects to
//! the first address that answers, through a [`Dial`], so the connection goes to an address
//! that was checked and never to one a second lookup might bring. `https` URLs speak TLS
//! 1.2 or later, checked against the bundled web PKI roots, with the URL's host as the name
//! the certificate must have. The request is a `POST` with `Connection: close` and a fixed
//! `User-Agent`; a redirect is an answer like any other, never followed. Connecting may take
//! [`CONNECT_TIMEOUT`], the whole exchange [`TOTAL_TIMEOUT`], and at most [`MAX_ANSWER`]
//! bytes of the answer's body are read.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::header::{CONNECTION, CONTENT_LENGTH, HOST, USER_AGENT as USER_AGENT_HEADER};
use axum::http::{HeaderName, HeaderValue, Method, Request};
use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt as _, Full};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use neon_app::webhooks::transport::{
    Answer, CONNECT_TIMEOUT, MAX_ANSWER, Outgoing, SendError, TOTAL_TIMEOUT, USER_AGENT,
    WebhookTransport, excerpt,
};
use neon_domain::webhooks::{check_ip, literal_ip};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_util::task::AbortOnDropHandle;
use tracing::debug;
use url::{Host, Url};

/// Finds the addresses of a host name.
#[async_trait]
pub trait Resolve: Send + Sync + 'static {
    async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>>;
}

/// The system's resolver, as `getaddrinfo` answers.
pub struct SystemResolver;

#[async_trait]
impl Resolve for SystemResolver {
    async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        Ok(tokio::net::lookup_host((host, port)).await?.collect())
    }
}

/// Opens a TCP connection to one address.
#[async_trait]
pub trait Dial: Send + Sync + 'static {
    async fn dial(&self, address: SocketAddr) -> io::Result<TcpStream>;
}

/// Connects as the operating system does.
pub struct SystemDialer;

#[async_trait]
impl Dial for SystemDialer {
    async fn dial(&self, address: SocketAddr) -> io::Result<TcpStream> {
        TcpStream::connect(address).await
    }
}

/// Sends webhooks; see the module docs.
pub struct HttpTransport {
    resolver: Arc<dyn Resolve>,
    dialer: Arc<dyn Dial>,
    tls: TlsConnector,
    connect_timeout: Duration,
    total_timeout: Duration,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new(Arc::new(SystemResolver), Arc::new(SystemDialer))
    }
}

impl HttpTransport {
    /// A transport that resolves with `resolver` and connects with `dialer`.
    pub fn new(resolver: Arc<dyn Resolve>, dialer: Arc<dyn Dial>) -> Self {
        Self {
            resolver,
            dialer,
            tls: TlsConnector::from(Arc::new(crate::tls::http_client_config())),
            connect_timeout: CONNECT_TIMEOUT,
            total_timeout: TOTAL_TIMEOUT,
        }
    }

    /// Bounds connecting, and the whole exchange, by others than [`CONNECT_TIMEOUT`] and
    /// [`TOTAL_TIMEOUT`].
    #[cfg(test)]
    pub fn timeouts(mut self, connect: Duration, total: Duration) -> Self {
        self.connect_timeout = connect;
        self.total_timeout = total;
        self
    }

    /// The addresses a URL's host stands for, resolved once within the time connecting may
    /// take, every one checked.
    async fn addresses(
        &self,
        url: &Url,
        allow_private: bool,
    ) -> Result<Vec<SocketAddr>, SendError> {
        let port = url.port_or_known_default().ok_or(SendError::Unresolvable)?;
        let addresses = match literal_ip(url) {
            Some(ip) => vec![SocketAddr::new(ip, port)],
            None => {
                let host = url.host_str().ok_or(SendError::Unresolvable)?;
                // A name that takes longer than connecting may does not resolve.
                tokio::time::timeout(self.connect_timeout, self.resolver.resolve(host, port))
                    .await
                    .map_err(|_| SendError::Unresolvable)?
                    .map_err(|_| SendError::Unresolvable)?
            }
        };
        if addresses.is_empty() {
            return Err(SendError::Unresolvable);
        }
        for address in &addresses {
            check_ip(address.ip(), allow_private).map_err(SendError::Refused)?;
        }
        Ok(addresses)
    }

    /// A connection to the first of `addresses` that answers.
    async fn connect(&self, addresses: &[SocketAddr]) -> Result<TcpStream, SendError> {
        let attempt = async {
            for address in addresses {
                match self.dialer.dial(*address).await {
                    Ok(stream) => return Ok(stream),
                    Err(error) => debug!(%error, "a webhook address did not answer"),
                }
            }
            Err(SendError::Connect)
        };
        tokio::time::timeout(self.connect_timeout, attempt)
            .await
            .map_err(|_| SendError::Timeout)?
    }

    async fn exchange(&self, request: Outgoing, allow_private: bool) -> Result<Answer, SendError> {
        let url = request.url.clone();
        let addresses = self.addresses(&url, allow_private).await?;
        let stream = self.connect(&addresses).await?;
        if url.scheme() == "https" {
            let name = server_name(&url).ok_or(SendError::Tls)?;
            let tls = tokio::time::timeout(self.connect_timeout, self.tls.connect(name, stream))
                .await
                .map_err(|_| SendError::Timeout)?
                .map_err(|error| {
                    debug!(%error, "a webhook's TLS handshake failed");
                    SendError::Tls
                })?;
            post(tls, request).await
        } else {
            post(stream, request).await
        }
    }
}

#[async_trait]
impl WebhookTransport for HttpTransport {
    async fn check(&self, url: &Url, allow_private: bool) -> Result<(), SendError> {
        self.addresses(url, allow_private).await.map(|_| ())
    }

    async fn send(&self, request: Outgoing, allow_private: bool) -> Result<Answer, SendError> {
        tokio::time::timeout(self.total_timeout, self.exchange(request, allow_private))
            .await
            .unwrap_or(Err(SendError::Timeout))
    }
}

/// The name a TLS certificate must have: the URL's host.
fn server_name(url: &Url) -> Option<ServerName<'static>> {
    match url.host()? {
        Host::Domain(name) => ServerName::try_from(name.trim_end_matches('.').to_owned()).ok(),
        Host::Ipv4(ip) => Some(ServerName::IpAddress(IpAddr::V4(ip).into())),
        Host::Ipv6(ip) => Some(ServerName::IpAddress(IpAddr::V6(ip).into())),
    }
}

/// The `Host` header of a URL: its host, and its port when it is not the scheme's own.
fn host_header(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

/// Sends the request on a connection, and reads the status and the start of the body.
async fn post<S>(stream: S, request: Outgoing) -> Result<Answer, SendError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) = http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
        .await
        .map_err(|error| {
            debug!(%error, "a webhook's HTTP handshake failed");
            SendError::Protocol
        })?;
    // The connection's task is aborted however the exchange ends, a timeout that drops this
    // future included, so that a receiver that stops reading cannot keep it alive.
    let _driving = AbortOnDropHandle::new(tokio::spawn(connection));
    let target = match request.url.query() {
        Some(query) => format!("{}?{query}", request.url.path()),
        None => request.url.path().to_owned(),
    };
    let mut built = Request::builder()
        .method(Method::POST)
        .uri(target)
        .header(HOST, host_header(&request.url).ok_or(SendError::Protocol)?)
        .header(USER_AGENT_HEADER, USER_AGENT)
        .header(CONNECTION, "close")
        .header(CONTENT_LENGTH, request.body.len());
    for (name, value) in &request.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| SendError::Protocol)?;
        let value = HeaderValue::from_str(value).map_err(|_| SendError::Protocol)?;
        built = built.header(name, value);
    }
    let built = built
        .body(Full::new(request.body))
        .map_err(|_| SendError::Protocol)?;
    let response = sender.send_request(built).await.map_err(|error| {
        debug!(%error, "a webhook's request failed");
        SendError::Connect
    })?;
    let status = response.status().as_u16();
    let mut body = response.into_body();
    let mut read = BytesMut::new();
    while read.len() < MAX_ANSWER {
        match body.frame().await {
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    let room = MAX_ANSWER - read.len();
                    read.extend_from_slice(&data[..data.len().min(room)]);
                }
            }
            // An answer cut short still has its status.
            Some(Err(_)) | None => break,
        }
    }
    Ok(Answer {
        status,
        excerpt: excerpt(&read),
    })
}

#[cfg(test)]
pub(crate) mod receiver;
#[cfg(test)]
mod tests;
