//! GitHub's releases and the updater, each played by a receiver on a free port of this
//! machine's loopback: nothing here reaches GitHub.

use axum::http::Method;
use neon_app::updates::{Releases, ReleasesError, Updater, UpdaterEndpoint, UpdaterError};
use neon_domain::settings::Secret;
use neon_domain::updates::{UpdaterState, UpdaterStatus};
use semver::Version;
use serde_json::json;
use time::macros::datetime;
use url::Url;

use super::client::{Client, FetchError, Outgoing};
use super::*;
use crate::webhooks::receiver::{Receiver, Reply};

/// A token the updater takes, obviously fake.
const TOKEN: &str = "fake-updater-token-for-the-tests-only";

fn json_reply(status: u16, body: &serde_json::Value) -> Reply {
    Reply {
        status,
        headers: vec![("content-type", "application/json".to_owned())],
        body: body.to_string().into_bytes(),
    }
}

fn listed(tag: &str, prerelease: bool, draft: bool) -> serde_json::Value {
    json!({
        "tag_name": tag,
        "name": format!("Neon Teamspace {tag}"),
        "draft": draft,
        "prerelease": prerelease,
        "html_url": format!("https://github.com/lein3000zzz/neon-teamspace/releases/tag/{tag}"),
        "published_at": if draft { serde_json::Value::Null } else { json!("2026-11-28T09:00:00Z") },
        "body": "What changed.",
        "assets": [],
    })
}

/// A URL on a port of loopback that nothing listens on.
fn closed_port() -> Url {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = listener.local_addr().expect("bound");
    drop(listener);
    Url::parse(&format!("http://{address}")).expect("a URL")
}

fn github(receiver: &Receiver, path: &str) -> GitHubReleases {
    GitHubReleases::new(&Url::parse(&receiver.url(path)).expect("a URL"))
}

#[tokio::test]
async fn github_lists_the_releases_of_the_repository_without_credentials() {
    let receiver = Receiver::start().await;
    receiver.always(json_reply(
        200,
        &json!([
            listed("v0.3.0-rc.1", true, false),
            listed("v0.2.0", false, false),
            listed("v0.9.0", false, true),
            listed("nightly", true, false),
            {
                "tag_name": "v0.1.9",
                "draft": false,
                "prerelease": false,
                "html_url": "http://github.com/lein3000zzz/neon-teamspace/releases/tag/v0.1.9",
                "published_at": "2026-11-01T09:00:00Z",
            },
        ]),
    ));

    let releases = github(&receiver, "/mirror/").list().await.expect("listed");

    let versions: Vec<(String, bool, bool)> = releases
        .iter()
        .map(|release| {
            (
                release.version.to_string(),
                release.prerelease,
                release.draft,
            )
        })
        .collect();
    assert_eq!(
        versions,
        [
            ("0.3.0-rc.1".to_owned(), true, false),
            ("0.2.0".to_owned(), false, false),
            ("0.9.0".to_owned(), false, true),
        ],
        "a tag that is no version, and a page that is no https URL, are left out"
    );
    assert_eq!(
        releases[1].url.as_str(),
        "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v0.2.0"
    );
    assert_eq!(
        releases[1].published_at,
        Some(datetime!(2026-11-28 09:00 UTC))
    );
    assert_eq!(releases[2].published_at, None);

    let asked = receiver.received();
    assert_eq!(asked.len(), 1);
    let request = &asked[0];
    assert_eq!(request.method, Method::GET);
    assert_eq!(
        request.path,
        "/mirror/repos/lein3000zzz/neon-teamspace/releases"
    );
    assert_eq!(request.query.as_deref(), Some("per_page=30"));
    assert_eq!(
        request.header("user-agent"),
        Some(concat!("NeonTeamspace/", env!("CARGO_PKG_VERSION")))
    );
    assert_eq!(
        request.header("accept"),
        Some("application/vnd.github+json")
    );
    assert_eq!(request.header("x-github-api-version"), Some("2022-11-28"));
    assert_eq!(request.header("authorization"), None);
    assert_eq!(request.header("cookie"), None);
}

#[tokio::test]
async fn github_s_refusals_say_why() {
    let cases = [
        (
            json_reply(403, &json!({"message": "API rate limit exceeded"})),
            "GitHub answered 403: it limits how often one address asks, so the next check \
             tries again",
        ),
        (
            json_reply(429, &json!({})),
            "GitHub answered 429: it limits how often one address asks, so the next check \
             tries again",
        ),
        (
            json_reply(404, &json!({"message": "Not Found"})),
            "GitHub answered 404: it shows no repository lein3000zzz/neon-teamspace",
        ),
        (Reply::status(502), "GitHub answered 502"),
        (Reply::status(301), "GitHub answered 301"),
        (
            json_reply(200, &json!({"releases": []})),
            "GitHub's answer is not a list of releases",
        ),
    ];
    for (reply, why) in cases {
        let receiver = Receiver::start().await;
        receiver.always(reply);

        assert_eq!(
            github(&receiver, "").list().await,
            Err(ReleasesError(why.to_owned()))
        );
    }
}

#[tokio::test]
async fn github_s_answer_is_bounded() {
    let receiver = Receiver::start().await;
    receiver.always(Reply {
        status: 200,
        headers: Vec::new(),
        body: vec![b' '; 4 * 1024 * 1024 + 1],
    });

    assert_eq!(
        github(&receiver, "").list().await,
        Err(ReleasesError(
            "GitHub could not be asked: the answer is larger than 4096 KiB".to_owned()
        ))
    );
}

#[tokio::test]
async fn github_that_cannot_be_reached_says_so() {
    let error = GitHubReleases::new(&closed_port())
        .list()
        .await
        .expect_err("no answer");

    assert_eq!(
        error,
        ReleasesError("GitHub could not be asked: connecting to 127.0.0.1 failed".to_owned())
    );
}

fn endpoint(receiver: &Receiver, path: &str) -> UpdaterEndpoint {
    UpdaterEndpoint {
        url: Url::parse(&receiver.url(path)).expect("a URL"),
        token: Secret::new(TOKEN),
    }
}

fn status_body() -> serde_json::Value {
    json!({
        "state": "updating",
        "version": "0.2.1",
        "log": "pulling ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1\n",
        "started_at": "2026-11-28T09:00:00Z",
        "updater_version": "0.2.0",
    })
}

fn updating() -> UpdaterStatus {
    UpdaterStatus {
        state: UpdaterState::Updating,
        version: Some("0.2.1".to_owned()),
        error: None,
        log: Some("pulling ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1\n".to_owned()),
        started_at: Some(datetime!(2026-11-28 09:00 UTC)),
        finished_at: None,
        updater_version: "0.2.0".to_owned(),
    }
}

#[tokio::test]
async fn the_updater_is_asked_its_status_with_the_token() {
    let receiver = Receiver::start().await;
    receiver.always(json_reply(200, &status_body()));

    let status = UpdaterClient::default()
        .status(&endpoint(&receiver, "/updater/"))
        .await
        .expect("a status");

    assert_eq!(status, updating());
    let asked = receiver.received();
    assert_eq!(asked[0].method, Method::GET);
    assert_eq!(asked[0].path, "/updater/v1/status");
    assert_eq!(
        asked[0].header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

#[tokio::test]
async fn the_updater_is_asked_to_update_to_a_version() {
    let receiver = Receiver::start().await;
    receiver.always(json_reply(202, &status_body()));

    let status = UpdaterClient::default()
        .update(
            &endpoint(&receiver, ""),
            &Version::parse("0.2.1").expect("a version"),
        )
        .await
        .expect("a status");

    assert_eq!(status, updating());
    let asked = receiver.received();
    assert_eq!(asked[0].method, Method::POST);
    assert_eq!(asked[0].path, UPDATE_PATH);
    assert_eq!(asked[0].header("content-type"), Some("application/json"));
    assert_eq!(asked[0].json(), json!({"version": "0.2.1"}));
    assert_eq!(
        asked[0].header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

#[tokio::test]
async fn the_updater_s_refusals_are_told_apart() {
    let error = |code: &str, message: &str| json!({"error": {"code": code, "message": message}});
    let cases = [
        (
            json_reply(401, &error("unauthenticated", "a token is needed")),
            UpdaterError::Refused,
        ),
        (
            json_reply(409, &error("update_in_progress", "an update is under way")),
            UpdaterError::Busy,
        ),
        (
            json_reply(400, &error("invalid_version", "0.2 is not a version")),
            UpdaterError::Rejected("0.2 is not a version".to_owned()),
        ),
        (
            Reply::status(400),
            UpdaterError::Rejected("it refused the request".to_owned()),
        ),
        (
            json_reply(500, &error("internal", "docker is not running")),
            UpdaterError::Unreachable("the updater answered 500: docker is not running".to_owned()),
        ),
        (
            Reply::status(503),
            UpdaterError::Unreachable("the updater answered 503".to_owned()),
        ),
    ];
    for (reply, refused) in cases {
        let receiver = Receiver::start().await;
        receiver.always(reply);

        assert_eq!(
            UpdaterClient::default()
                .update(
                    &endpoint(&receiver, ""),
                    &Version::parse("0.2.1").expect("a version")
                )
                .await,
            Err(refused)
        );
    }
}

#[tokio::test]
async fn an_answer_that_is_not_a_status_is_no_answer() {
    let receiver = Receiver::start().await;
    receiver.always(json_reply(200, &json!({"state": "sleeping"})));
    let at = endpoint(&receiver, "");

    assert_eq!(
        UpdaterClient::default().status(&at).await,
        Err(UpdaterError::Unreachable(format!(
            "the updater at {} answered something other than its status",
            at.url
        )))
    );
}

#[tokio::test]
async fn an_updater_that_does_not_listen_does_not_answer() {
    let at = UpdaterEndpoint {
        url: closed_port(),
        token: Secret::new(TOKEN),
    };

    let error = UpdaterClient::default()
        .status(&at)
        .await
        .expect_err("no answer");

    assert_eq!(
        error,
        UpdaterError::Unreachable(format!(
            "the updater at {} did not answer: connecting to 127.0.0.1 failed",
            at.url
        ))
    );
    assert!(!error.to_string().contains(TOKEN));
}

#[test]
fn the_running_build_is_the_workspace_s() {
    let running = running();

    assert_eq!(running.version.to_string(), env!("CARGO_PKG_VERSION"));
    assert_eq!(running.development, cfg!(debug_assertions));
}

#[tokio::test]
async fn an_answer_that_takes_too_long_is_none() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let url =
        Url::parse(&format!("http://{}", listener.local_addr().expect("bound"))).expect("a URL");
    // Takes the connection, and never answers on it.
    let silent = tokio::spawn(async move {
        let (_connection, _) = listener.accept().await.expect("a connection");
        std::future::pending::<()>().await;
    });
    let wait = std::time::Duration::from_millis(200);

    let error = Client::default()
        .send(Outgoing {
            method: Method::GET,
            url: &url,
            headers: Vec::new(),
            json: None,
            timeout: wait,
            max_answer: 1024,
        })
        .await
        .expect_err("no answer");

    assert_eq!(error, FetchError::Timeout(wait));
    assert_eq!(error.to_string(), "no answer within 200ms");
    silent.abort();
}
