//! What the integration tests share: fake programs on the `PATH`, and a plain HTTP client.

#![allow(dead_code, reason = "each test binary uses a part of it")]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

/// A token the tests give the updater: obviously not a secret, and long enough.
pub const TOKEN: &str = "test-token-not-a-secret-0123456789";

/// Writes `script` as the executable `name` in `dir`.
pub fn program(dir: &Path, name: &str, script: &str) {
    let path = dir.join(name);
    std::fs::write(&path, script).expect("writing a fake program");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("making a fake program executable");
}

/// `dir` first, then the system's directories, where `sh`, `cat` and `sleep` are.
pub fn path(dir: &Path) -> String {
    format!("{}:/usr/bin:/bin", dir.display())
}

/// The lines of the file `name` in `dir`, none while it is missing.
pub fn lines(dir: &Path, name: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(name))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Polls `check` until it gives something, for ten seconds at most.
pub fn eventually<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// An answer: its status code and its body.
#[derive(Debug)]
pub struct Answer {
    pub status: u16,
    pub body: String,
}

impl Answer {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.body))
    }
}

/// Sends one HTTP/1.1 request to `addr` and reads the answer to its end.
pub fn request(addr: &str, method: &str, path: &str, token: Option<&str>, body: &str) -> Answer {
    let mut stream = TcpStream::connect(addr).expect("connecting to the updater");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: updater\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(token) = token {
        head.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    stream
        .write_all(format!("{head}\r\n{body}").as_bytes())
        .expect("sending the request");
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .expect("reading the answer");
    let (head, body) = answer
        .split_once("\r\n\r\n")
        .expect("an answer with a head and a body");
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|status| status.parse().ok())
        .expect("a status code");
    Answer {
        status,
        body: body.to_owned(),
    }
}
