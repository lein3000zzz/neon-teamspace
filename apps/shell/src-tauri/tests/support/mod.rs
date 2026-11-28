//! A server on loopback that answers every request with one canned answer, as the tests need.

#![allow(dead_code, reason = "each test binary uses a part of it")]

use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// What an HTTP server answers to bytes that are not HTTP.
const BAD_REQUEST: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nconnection: close\r\ncontent-length: 0\r\n\r\n";

/// How a canned server answers.
#[derive(Clone)]
pub enum Answer {
    /// These bytes, then the connection closes.
    Bytes(Vec<u8>),
    /// Nothing at all, for as long as the test runs.
    Silence,
}

impl Answer {
    /// An HTTP/1.1 answer with the status line, headers and body given.
    pub fn http(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Self {
        let mut bytes = format!("HTTP/1.1 {status}\r\nconnection: close\r\n").into_bytes();
        for (name, value) in headers {
            bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        {
            bytes.extend_from_slice(format!("content-length: {}\r\n", body.len()).as_bytes());
        }
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(body);
        Self::Bytes(bytes)
    }

    /// `200` with JSON.
    pub fn json(body: &str) -> Self {
        Self::http(
            "200 OK",
            &[("content-type", "application/json")],
            body.as_bytes(),
        )
    }

    /// The answer of a Neon Teamspace server named `name`.
    pub fn neon(name: &str) -> Self {
        Self::json(&format!(
            r#"{{"product":"neon-teamspace","name":"{name}","version":"0.1.0"}}"#
        ))
    }
}

/// A running canned server.
pub struct Server {
    pub origin: String,
    requests: mpsc::UnboundedReceiver<String>,
}

impl Server {
    /// The request lines of the requests the server has had and no test has read yet.
    pub fn requests_so_far(&mut self) -> Vec<String> {
        std::iter::from_fn(|| self.requests.try_recv().ok()).collect()
    }

    /// The request line of the next request, such as `GET /api/v1/meta HTTP/1.1`.
    pub async fn next_request(&mut self) -> String {
        tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
            .await
            .expect("a request within five seconds")
            .expect("the server is running")
    }
}

/// Starts a server on a free port of 127.0.0.1 that gives every request `answer`.
pub async fn serve(answer: Answer) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    let (sender, requests) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let answer = answer.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                    // Not HTTP, such as a TLS handshake: answered as HTTP servers do.
                    if !request[0].is_ascii_uppercase() {
                        let _ = stream.write_all(BAD_REQUEST).await;
                        return;
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let _ = sender.send(text.lines().next().unwrap_or_default().to_owned());
                match answer {
                    Answer::Bytes(bytes) => {
                        let _ = stream.write_all(&bytes).await;
                        let _ = stream.shutdown().await;
                    }
                    Answer::Silence => tokio::time::sleep(Duration::from_secs(60)).await,
                }
            });
        }
    });
    Server { origin, requests }
}

/// Starts a server on a free port of 127.0.0.1 that closes each connection once the first
/// bytes arrive, as some servers do with bytes they cannot read: its origin.
pub async fn serve_closing() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = [0; 1024];
                let _ = stream.read(&mut buffer).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    origin
}

/// A port of 127.0.0.1 where nothing listens.
pub async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    listener.local_addr().expect("an address").port()
}
