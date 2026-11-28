//! The HTTP/1.1 client of the release check and the updater: one request per connection,
//! over TLS for `https`, as [`crate::tls`] sets it up, with a redirect an answer like any
//! other. Connecting may take [`CONNECT_TIMEOUT`]; the whole exchange, and the most of an
//! answer that is read, are each request's own.

use std::time::Duration;

use axum::http::header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST};
use axum::http::{HeaderName, HeaderValue, Method, Request, StatusCode};
use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt as _, Full};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use neon_domain::duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_util::task::AbortOnDropHandle;
use url::{Host, Url};

/// How long connecting may take, TLS included.
pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A request to send.
pub(super) struct Outgoing<'a> {
    pub method: Method,
    pub url: &'a Url,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    /// A JSON body.
    pub json: Option<Bytes>,
    /// How long the whole exchange may take.
    pub timeout: Duration,
    /// The most of the answer's body that is read; more fails the request.
    pub max_answer: usize,
}

/// What came back.
#[derive(Debug)]
pub(super) struct Answer {
    pub status: StatusCode,
    pub body: Bytes,
}

/// Why a request was not answered. The messages name the URL's host, never its path or
/// the headers sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(super) enum FetchError {
    #[error("the URL is not http or https with a host")]
    Url,
    #[error("connecting to {0} failed")]
    Connect(String),
    #[error("the TLS handshake with {0} failed")]
    Tls(String),
    #[error("no answer within {}", duration::format(*.0))]
    Timeout(Duration),
    #[error("the answer is larger than {} KiB", .0 / 1024)]
    TooLarge(usize),
    #[error("the exchange with {0} failed")]
    Protocol(String),
}

/// Sends requests; see the module docs.
#[derive(Clone)]
pub(super) struct Client {
    tls: TlsConnector,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            tls: TlsConnector::from(std::sync::Arc::new(crate::tls::http_client_config())),
        }
    }
}

impl Client {
    pub(super) async fn send(&self, request: Outgoing<'_>) -> Result<Answer, FetchError> {
        let timeout = request.timeout;
        tokio::time::timeout(timeout, self.exchange(request))
            .await
            .unwrap_or(Err(FetchError::Timeout(timeout)))
    }

    async fn exchange(&self, request: Outgoing<'_>) -> Result<Answer, FetchError> {
        let url = request.url;
        let secure = match url.scheme() {
            "https" => true,
            "http" => false,
            _ => return Err(FetchError::Url),
        };
        let host = url.host_str().ok_or(FetchError::Url)?.to_owned();
        let port = url.port_or_known_default().ok_or(FetchError::Url)?;
        let connect = async {
            match url.host() {
                Some(Host::Domain(name)) => TcpStream::connect((name, port)).await,
                Some(Host::Ipv4(ip)) => TcpStream::connect((ip, port)).await,
                Some(Host::Ipv6(ip)) => TcpStream::connect((ip, port)).await,
                None => Err(std::io::ErrorKind::InvalidInput.into()),
            }
        };
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| FetchError::Timeout(CONNECT_TIMEOUT))?
            .map_err(|_| FetchError::Connect(host.clone()))?;
        if !secure {
            return round_trip(tcp, request, &host).await;
        }
        let name = crate::tls::server_name(url).ok_or(FetchError::Url)?;
        let tls = tokio::time::timeout(CONNECT_TIMEOUT, self.tls.connect(name, tcp))
            .await
            .map_err(|_| FetchError::Timeout(CONNECT_TIMEOUT))?
            .map_err(|_| FetchError::Tls(host.clone()))?;
        round_trip(tls, request, &host).await
    }
}

/// Sends the request on a connection, and reads the status and the body.
async fn round_trip<S>(stream: S, request: Outgoing<'_>, host: &str) -> Result<Answer, FetchError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let failed = || FetchError::Protocol(host.to_owned());
    let (mut sender, connection) = http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
        .await
        .map_err(|_| failed())?;
    // The connection's task is aborted however the exchange ends, a timeout that drops this
    // future included.
    let _driving = AbortOnDropHandle::new(tokio::spawn(connection));
    let url = request.url;
    let target = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    };
    let host_header = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let mut built = Request::builder()
        .method(request.method)
        .uri(target)
        .header(HOST, host_header)
        .header(CONNECTION, "close");
    for (name, value) in request.headers {
        built = built.header(name, value);
    }
    let body = match request.json {
        Some(json) => {
            built = built
                .header(CONTENT_TYPE, "application/json")
                .header(CONTENT_LENGTH, json.len());
            json
        }
        None => Bytes::new(),
    };
    let built = built.body(Full::new(body)).map_err(|_| failed())?;
    let response = sender.send_request(built).await.map_err(|_| failed())?;
    let status = response.status();
    let mut body = response.into_body();
    let mut read = BytesMut::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| failed())?;
        if let Ok(data) = frame.into_data() {
            if read.len() + data.len() > request.max_answer {
                return Err(FetchError::TooLarge(request.max_answer));
            }
            read.extend_from_slice(&data);
        }
    }
    Ok(Answer {
        status,
        body: read.freeze(),
    })
}
