//! A webhook receiver on this machine's loopback, for tests: it keeps every request it gets
//! and answers as told.

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use super::Resolve;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Long enough for any delivery here; reaching it fails the test.
pub(crate) const PATIENCE: Duration = Duration::from_secs(20);

/// A request the receiver got.
#[derive(Debug, Clone)]
pub(crate) struct Received {
    pub(crate) method: Method,
    pub(crate) path: String,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Bytes,
}

impl Received {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("a JSON body")
    }
}

/// How the receiver answers one request.
#[derive(Debug, Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

#[derive(Default)]
struct Shared {
    received: Mutex<Vec<Received>>,
    script: Mutex<VecDeque<Reply>>,
    default: Mutex<Option<Reply>>,
    arrived: Notify,
}

pub(crate) struct Receiver {
    addr: SocketAddr,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl Receiver {
    /// A receiver that answers 200 unless told otherwise.
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let addr = listener.local_addr().expect("bound");
        let shared = Arc::new(Shared::default());
        let router = Router::new()
            .fallback(receive)
            .with_state(Arc::clone(&shared));
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Self { addr, shared, task }
    }

    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Its URL at `path`, on its loopback address.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// Answers the next requests with these, in order, then as before.
    pub(crate) fn script(&self, replies: impl IntoIterator<Item = Reply>) {
        self.shared.script.lock().expect("lock").extend(replies);
    }

    /// Answers every request with this once the script runs out.
    pub(crate) fn always(&self, reply: Reply) {
        *self.shared.default.lock().expect("lock") = Some(reply);
    }

    pub(crate) fn received(&self) -> Vec<Received> {
        self.shared.received.lock().expect("lock").clone()
    }

    /// The requests once there are at least `count`.
    pub(crate) async fn wait_for(&self, count: usize) -> Vec<Received> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let arrived = self.shared.arrived.notified();
            let received = self.received();
            if received.len() >= count {
                return received;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "the receiver got {} requests, not {count}",
                received.len()
            );
            let _ = tokio::time::timeout(left, arrived).await;
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn receive(
    State(shared): State<Arc<Shared>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    shared.received.lock().expect("lock").push(Received {
        method,
        path: uri.path().to_owned(),
        headers,
        body,
    });
    shared.arrived.notify_waiters();
    let reply = shared
        .script
        .lock()
        .expect("lock")
        .pop_front()
        .or_else(|| shared.default.lock().expect("lock").clone())
        .unwrap_or_else(|| Reply::status(200));
    let mut response = (
        StatusCode::from_u16(reply.status).unwrap_or(StatusCode::OK),
        reply.body,
    )
        .into_response();
    for (name, value) in reply.headers {
        if let Ok(value) = value.parse() {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

/// Answers each lookup with the next list of addresses, then the last one again, and counts
/// the lookups.
pub(crate) struct ScriptedResolver {
    answers: Mutex<VecDeque<Vec<SocketAddr>>>,
    asked: Mutex<Vec<(String, u16)>>,
}

impl ScriptedResolver {
    pub(crate) fn new(answers: Vec<Vec<SocketAddr>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            asked: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn asked(&self) -> Vec<(String, u16)> {
        self.asked.lock().expect("lock").clone()
    }
}

#[async_trait]
impl Resolve for ScriptedResolver {
    async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        self.asked
            .lock()
            .expect("lock")
            .push((host.to_owned(), port));
        let mut answers = self.answers.lock().expect("lock");
        let answer = if answers.len() > 1 {
            answers.pop_front()
        } else {
            answers.front().cloned()
        };
        answer.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such host"))
    }
}
