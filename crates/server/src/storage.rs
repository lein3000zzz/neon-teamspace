//! The HTTP client the S3 blob store sends its requests through: hyper's pooled client over
//! HTTP/1.1, with TLS 1.2 or later from rustls on ring for `https` endpoints, checked
//! against the bundled web PKI roots with the endpoint's host as the certificate's name.
//!
//! The store's own retries and signing are `object_store`'s; this only carries requests.
//! Connecting may take [`CONNECT_TIMEOUT`], and an answer's headers [`ANSWER_TIMEOUT`]; a
//! body streams for as long as it takes, as files may be large. Idle connections are kept
//! for [`IDLE_TIMEOUT`], so the requests of one page's images share a few.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use axum::http::Uri;
use http_body_util::BodyExt as _;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::{TokioExecutor, TokioIo};
use object_store::client::{
    ClientOptions, HttpClient, HttpConnector, HttpError, HttpErrorKind, HttpRequest,
    HttpRequestBody, HttpResponse, HttpResponseBody, HttpService,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_util::either::Either;

/// How long connecting to the store may take, TLS included.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the store may take to answer a request, until its headers.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// How long an idle connection to the store is kept for the next request.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Makes the client the S3 store sends its requests through; see the module docs.
#[derive(Debug, Default)]
pub struct S3Connector;

impl HttpConnector for S3Connector {
    fn connect(&self, _options: &ClientOptions) -> object_store::Result<HttpClient> {
        let roots = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
        let mut config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap_or_else(|_| unreachable!("ring supports the default protocol versions"))
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let dialer = Dialer {
            tls: TlsConnector::from(Arc::new(config)),
        };
        let client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(IDLE_TIMEOUT)
            .build(dialer);
        Ok(HttpClient::new(S3Http { client }))
    }
}

/// Sends a request, and gives its answer as the store reads it.
#[derive(Debug)]
struct S3Http {
    client: Client<Dialer, HttpRequestBody>,
}

#[async_trait]
impl HttpService for S3Http {
    async fn call(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let answer = tokio::time::timeout(ANSWER_TIMEOUT, self.client.request(request))
            .await
            .map_err(|elapsed| HttpError::new(HttpErrorKind::Timeout, elapsed))?
            .map_err(|error| {
                let kind = if error.is_connect() {
                    HttpErrorKind::Connect
                } else {
                    HttpErrorKind::Request
                };
                HttpError::new(kind, error)
            })?;
        Ok(answer.map(|body| {
            HttpResponseBody::new(
                body.map_err(|error| HttpError::new(HttpErrorKind::Interrupted, error)),
            )
        }))
    }
}

/// Opens connections to the store: TCP, then TLS for `https`.
#[derive(Clone)]
struct Dialer {
    tls: TlsConnector,
}

impl std::fmt::Debug for Dialer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dialer").finish_non_exhaustive()
    }
}

impl tower::Service<Uri> for Dialer {
    type Response = TokioIo<Stream>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<TokioIo<Stream>>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let tls = self.tls.clone();
        Box::pin(async move {
            let secure = match uri.scheme_str() {
                Some("https") => true,
                Some("http") => false,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "the store's URL is not http or https",
                    ));
                }
            };
            let host = uri
                .host()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no host"))?
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned();
            let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
            let connect = async {
                let tcp = TcpStream::connect((host.as_str(), port)).await?;
                tcp.set_nodelay(true)?;
                if !secure {
                    return Ok(Stream(Either::Left(tcp)));
                }
                let name = ServerName::try_from(host.clone())
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
                let tls = tls.connect(name, tcp).await?;
                Ok(Stream(Either::Right(Box::new(tls))))
            };
            tokio::time::timeout(CONNECT_TIMEOUT, connect)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connecting timed out"))?
                .map(TokioIo::new)
        })
    }
}

/// A connection to the store, in the clear or over TLS.
struct Stream(Either<TcpStream, Box<TlsStream<TcpStream>>>);

impl Connection for Stream {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }
}
