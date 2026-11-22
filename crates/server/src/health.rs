//! `neon health`: asks the admin listener whether the server is ready, for container health
//! checks in images without a shell or curl.
//!
//! One plain HTTP/1.1 request on a fresh connection: the admin listener's answers are short
//! and their status is all that counts, so no HTTP client is needed.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use neon_domain::duration;
use neon_observability::config::AdminConfig;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

/// How long the check waits for an answer. `/readyz` gives the database two seconds.
pub const TIMEOUT: Duration = Duration::from_secs(3);

/// The most of an answer that is read. The admin listener's probes answer a few bytes.
const MAX_RESPONSE: u64 = 4096;

/// What to ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// `/readyz`: whether the server takes traffic.
    Ready,
    /// `/livez`: whether the process is up.
    Live,
}

impl Probe {
    fn path(self) -> &'static str {
        match self {
            Self::Ready => "/readyz",
            Self::Live => "/livez",
        }
    }
}

/// Asks the admin listener `config` describes. Succeeds on a `200`, and otherwise fails with
/// the status and the reason the listener gave, or why it could not be asked.
pub async fn check(config: &AdminConfig, probe: Probe) -> anyhow::Result<()> {
    if !config.enabled {
        bail!(
            "the admin listener is off (observability.admin.enabled), so there is nothing to ask"
        );
    }
    ask(reachable(config.addr), probe.path(), TIMEOUT).await
}

/// The address to connect to for a listener bound to `addr`: one bound to every address is
/// reached on loopback.
fn reachable(addr: SocketAddr) -> SocketAddr {
    let ip = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, addr.port())
}

async fn ask(addr: SocketAddr, path: &str, timeout: Duration) -> anyhow::Result<()> {
    let response = tokio::time::timeout(timeout, get(addr, path))
        .await
        .map_err(|_| {
            anyhow!(
                "{path} on {addr} did not answer within {}",
                duration::format(timeout)
            )
        })??;
    let (status, reason) = parse(&response).with_context(|| format!("{path} on {addr}"))?;
    if status == 200 {
        Ok(())
    } else {
        bail!("{path} on {addr} answered {status}: {reason}")
    }
}

/// Sends `GET path` and reads the answer until the listener closes the connection, as
/// `Connection: close` asks.
async fn get(addr: SocketAddr, path: &str) -> anyhow::Result<Vec<u8>> {
    let mut stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("connecting to the admin listener on {addr}"))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .with_context(|| format!("asking {addr} for {path}"))?;
    let mut response = Vec::new();
    stream
        .take(MAX_RESPONSE)
        .read_to_end(&mut response)
        .await
        .with_context(|| format!("reading the answer to {path} from {addr}"))?;
    Ok(response)
}

/// The status code and the body, as text, of an HTTP/1.x answer.
fn parse(response: &[u8]) -> anyhow::Result<(u16, String)> {
    let text = String::from_utf8_lossy(response);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status_line = head.lines().next().unwrap_or_default();
    let status = status_line
        .strip_prefix("HTTP/1.")
        .and_then(|rest| rest.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| anyhow!("the answer is not HTTP"))?;
    Ok((status, body.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use neon_observability::Telemetry;
    use neon_observability::admin::{self, Readiness};
    use tokio::net::TcpListener;

    use super::*;

    const LOOPBACK: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

    /// Starts an admin listener on a free port with `readiness`, and returns its configuration.
    async fn admin(telemetry: &Telemetry, readiness: Readiness) -> AdminConfig {
        let mut config = AdminConfig {
            enabled: true,
            addr: LOOPBACK,
        };
        config.addr = admin::spawn(&config, telemetry, readiness)
            .await
            .expect("the admin listener starts")
            .expect("it is enabled");
        config
    }

    fn message(result: anyhow::Result<()>) -> String {
        format!("{:#}", result.expect_err("the check fails"))
    }

    #[tokio::test]
    async fn a_ready_server_passes_both_probes() {
        let telemetry = Telemetry::noop();
        let config = admin(&telemetry, Readiness::always()).await;

        assert!(check(&config, Probe::Ready).await.is_ok());
        assert!(check(&config, Probe::Live).await.is_ok());
        telemetry.shutdown().run().await.expect("stops");
    }

    #[tokio::test]
    async fn a_server_that_is_not_ready_fails_with_its_reason_but_is_live() {
        let telemetry = Telemetry::noop();
        let config = admin(
            &telemetry,
            Readiness::new(|| async { Err("the database is unreachable") }),
        )
        .await;

        let error = message(check(&config, Probe::Ready).await);

        assert_eq!(
            error,
            format!(
                "/readyz on {} answered 503: the database is unreachable",
                config.addr
            )
        );
        assert!(check(&config, Probe::Live).await.is_ok());
        telemetry.shutdown().run().await.expect("stops");
    }

    #[tokio::test]
    async fn a_stopped_server_fails_both_probes() {
        let telemetry = Telemetry::noop();
        let config = admin(&telemetry, Readiness::always()).await;
        telemetry.shutdown().run().await.expect("stops");

        for probe in [Probe::Ready, Probe::Live] {
            let error = message(check(&config, probe).await);

            assert!(
                error.starts_with(&format!(
                    "connecting to the admin listener on {}: ",
                    config.addr
                )),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn a_listener_that_never_answers_times_out() {
        let listener = TcpListener::bind(LOOPBACK).await.expect("a free port");
        let addr = listener.local_addr().expect("bound");

        let error = message(ask(addr, "/readyz", Duration::from_millis(200)).await);

        assert_eq!(
            error,
            format!("/readyz on {addr} did not answer within 200ms")
        );
        drop(listener);
    }

    #[tokio::test]
    async fn an_answer_that_is_not_http_fails() {
        let listener = TcpListener::bind(LOOPBACK).await.expect("a free port");
        let addr = listener.local_addr().expect("bound");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accepts");
            // Read the request first: closing with it unread would reset the connection.
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).await.expect("reads");
                request.push(byte[0]);
            }
            stream
                .write_all(b"SSH-2.0-OpenSSH\r\n")
                .await
                .expect("sends");
        });

        let error = message(ask(addr, "/readyz", TIMEOUT).await);

        assert_eq!(error, format!("/readyz on {addr}: the answer is not HTTP"));
        server.await.expect("the server task ran");
    }

    #[tokio::test]
    async fn a_disabled_listener_cannot_be_asked() {
        let config = AdminConfig {
            enabled: false,
            ..AdminConfig::default()
        };

        let error = message(check(&config, Probe::Ready).await);

        assert!(error.contains("observability.admin.enabled"), "{error}");
    }

    #[test]
    fn a_listener_on_every_address_is_asked_on_loopback() {
        let any_v4: SocketAddr = "0.0.0.0:9090".parse().expect("an address");
        let any_v6: SocketAddr = "[::]:9090".parse().expect("an address");
        let private: SocketAddr = "10.0.0.5:9090".parse().expect("an address");

        assert_eq!(reachable(any_v4).to_string(), "127.0.0.1:9090");
        assert_eq!(reachable(any_v6).to_string(), "[::1]:9090");
        assert_eq!(reachable(private), private);
    }

    #[test]
    fn answers_are_parsed_by_their_status_line() {
        assert_eq!(
            parse(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok").expect("parses"),
            (200, "ok".to_owned())
        );
        assert_eq!(
            parse(b"HTTP/1.0 503 Service Unavailable\r\n\r\nstarting\n").expect("parses"),
            (503, "starting".to_owned())
        );
        assert!(parse(b"").is_err());
        assert!(parse(b"HTTP/1.1 abc\r\n\r\n").is_err());
        assert!(parse(b"HTTP/2 200\r\n\r\n").is_err());
    }
}
