//! The Android app's question to GitHub for the latest release, against servers on loopback
//! that answer as GitHub's API does, or fails to.

mod support;

use std::time::Duration;

use neon_shell::releases::{self, MAX_ANSWER_BYTES, ReleaseError, ReleaseOptions};
use support::{Answer, closed_port, serve};

const RELEASE: &str = r#"{
  "tag_name": "v0.2.0",
  "name": "v0.2.0",
  "draft": false,
  "prerelease": false,
  "assets": [
    {
      "name": "Neon-Teamspace_0.2.0_android.apk",
      "browser_download_url": "https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.2.0/Neon-Teamspace_0.2.0_android.apk"
    }
  ]
}"#;

fn options(origin: &str) -> ReleaseOptions {
    ReleaseOptions {
        url: format!("{origin}/repos/lein3000zzz/neon-teamspace/releases/latest")
            .parse()
            .expect("a URL"),
        timeout: Duration::from_secs(2),
    }
}

#[tokio::test]
async fn the_latest_release_is_asked_for_without_credentials() {
    let mut server = serve(Answer::json(RELEASE)).await;
    let release = releases::latest(&options(&server.origin)).await.unwrap();
    assert_eq!(release.version.to_string(), "0.2.0");
    assert!(release.apk.is_some());
    let head = server.next_head().await.to_ascii_lowercase();
    assert!(
        head.starts_with("get /repos/lein3000zzz/neon-teamspace/releases/latest http/1.1\n"),
        "{head}"
    );
    assert!(
        head.contains("\naccept: application/vnd.github+json\n"),
        "{head}"
    );
    assert!(
        head.contains("\nx-github-api-version: 2022-11-28\n"),
        "{head}"
    );
    assert!(
        head.contains(&format!(
            "\nuser-agent: neonteamspaceapp/{}\n",
            env!("CARGO_PKG_VERSION")
        )),
        "{head}"
    );
    for header in ["authorization", "cookie"] {
        assert!(!head.contains(&format!("\n{header}:")), "{head}");
    }
}

#[tokio::test]
async fn githubs_refusals_are_told_apart() {
    for (status, expected) in [
        ("404 Not Found", ReleaseError::NoRelease),
        ("403 Forbidden", ReleaseError::RateLimited),
        ("429 Too Many Requests", ReleaseError::RateLimited),
        ("502 Bad Gateway", ReleaseError::Status { status: 502 }),
    ] {
        let server = serve(Answer::http(status, &[], b"{}")).await;
        let error = releases::latest(&options(&server.origin))
            .await
            .unwrap_err();
        assert_eq!(error, expected, "{status}");
    }
}

#[tokio::test]
async fn a_redirect_away_from_githubs_api_is_not_followed() {
    let server = serve(Answer::http(
        "301 Moved Permanently",
        &[("location", "https://evil.example/releases/latest")],
        b"",
    ))
    .await;
    let error = releases::latest(&options(&server.origin))
        .await
        .unwrap_err();
    assert_eq!(error, ReleaseError::Status { status: 301 });
}

#[tokio::test]
async fn an_answer_too_large_is_not_read() {
    let large = format!(
        r#"{{"tag_name":"v0.2.0","body":"{}"}}"#,
        "x".repeat(MAX_ANSWER_BYTES)
    );
    // With no length to read beforehand, it is cut off as it arrives: chunks of 64 KiB.
    let mut chunked = b"HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n".to_vec();
    for chunk in large.as_bytes().chunks(64 * 1024) {
        chunked.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        chunked.extend_from_slice(chunk);
        chunked.extend_from_slice(b"\r\n");
    }
    chunked.extend_from_slice(b"0\r\n\r\n");
    for answer in [Answer::json(&large), Answer::Bytes(chunked)] {
        let server = serve(answer).await;
        let error = releases::latest(&options(&server.origin))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "unreadable", "{error}");
        assert!(
            error.to_string().contains("larger than 1024 KiB"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn a_server_that_keeps_the_app_waiting_is_given_up_on() {
    let server = serve(Answer::Silence).await;
    let error = releases::latest(&options(&server.origin))
        .await
        .unwrap_err();
    assert_eq!(error, ReleaseError::TimedOut { seconds: 2 });
}

#[tokio::test]
async fn nothing_listening_is_unreachable() {
    let port = closed_port().await;
    let error = releases::latest(&options(&format!("http://127.0.0.1:{port}")))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "unreachable", "{error}");
    assert!(
        error
            .to_string()
            .starts_with("The app could not reach GitHub"),
        "{error}"
    );
}
