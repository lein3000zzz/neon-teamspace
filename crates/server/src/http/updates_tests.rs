//! `/api/v1/admin/updates` over HTTP, with GitHub and the updater replaced by fakes: who
//! may read, check and update, the answers' shape, and each refusal's code.

use std::sync::Mutex;

use async_trait::async_trait;
use axum::Router;
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::{Method, StatusCode};
use neon_app::settings::Change;
use neon_app::updates::{Releases, ReleasesError, Updater, UpdaterEndpoint, UpdaterError};
use neon_domain::updates::{Release, Running, UpdaterState, UpdaterStatus};
use semver::Version;
use serde_json::{Value, json};
use sqlx::PgPool;
use url::Url;

use super::auth_tests::{admin, call, sign_in, user};
use super::tests::state_for_updates;
use super::*;

/// A token the setting takes, obviously fake.
const TOKEN: &str = "fake-updater-token-for-the-tests-only";

/// GitHub, listing 0.3.0-rc.1, 0.2.0 and 0.1.0.
struct FakeReleases;

#[async_trait]
impl Releases for FakeReleases {
    async fn list(&self) -> Result<Vec<Release>, ReleasesError> {
        Ok(["0.3.0-rc.1", "0.2.0", "0.1.0"]
            .into_iter()
            .map(|version| Release {
                version: Version::parse(version).expect("a version"),
                prerelease: version.contains('-'),
                draft: false,
                url: Url::parse(&format!(
                    "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v{version}"
                ))
                .expect("a URL"),
                published_at: Some(time::macros::datetime!(2026-11-28 09:00 UTC)),
            })
            .collect())
    }
}

/// The updater: what it answers to an update, and the versions it was asked for.
struct FakeUpdater {
    update: Mutex<Result<UpdaterStatus, UpdaterError>>,
    asked: Mutex<Vec<String>>,
}

impl FakeUpdater {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            update: Mutex::new(Ok(status(UpdaterState::Updating))),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn answer(&self, answer: Result<UpdaterStatus, UpdaterError>) {
        *self.update.lock().expect("unpoisoned") = answer;
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("unpoisoned").clone()
    }
}

#[async_trait]
impl Updater for FakeUpdater {
    async fn status(&self, _at: &UpdaterEndpoint) -> Result<UpdaterStatus, UpdaterError> {
        Ok(status(UpdaterState::Idle))
    }

    async fn update(
        &self,
        at: &UpdaterEndpoint,
        version: &Version,
    ) -> Result<UpdaterStatus, UpdaterError> {
        assert_eq!(at.token.expose(), TOKEN);
        self.asked
            .lock()
            .expect("unpoisoned")
            .push(version.to_string());
        self.update.lock().expect("unpoisoned").clone()
    }
}

fn status(state: UpdaterState) -> UpdaterStatus {
    UpdaterStatus {
        state,
        version: (state != UpdaterState::Idle).then(|| "0.2.0".to_owned()),
        error: None,
        log: (state != UpdaterState::Idle).then(|| "pulling the images\n".to_owned()),
        started_at: None,
        finished_at: None,
        updater_version: "0.2.0".to_owned(),
    }
}

struct Server {
    app: Router,
    state: AppState,
    updater: Arc<FakeUpdater>,
    session: String,
}

impl Server {
    /// A server that runs 0.1.0, as a release, with its admin signed in.
    async fn start(pool: PgPool) -> Self {
        let updater = FakeUpdater::new();
        let state = state_for_updates(
            pool,
            Arc::new(FakeReleases),
            Arc::clone(&updater) as Arc<dyn Updater>,
            Running {
                version: Version::parse("0.1.0").expect("a version"),
                development: false,
            },
        )
        .await;
        admin(&state).await;
        let app = router(state.clone(), false);
        let session = sign_in(&app, "admin@example.com").await;
        Self {
            app,
            state,
            updater,
            session,
        }
    }

    async fn set_up_updater(&self) {
        self.state
            .settings
            .update(
                [(
                    "updates.updater_token".to_owned(),
                    Change::Set(json!(TOKEN)),
                )],
                None,
            )
            .await
            .expect("saved");
    }

    async fn as_admin(&self, method: Method, uri: &str, body: Option<Value>) -> auth_tests::Answer {
        call(
            &self.app,
            method,
            uri,
            &[(COOKIE.as_str(), &self.session)],
            body,
        )
        .await
    }

    /// An API token of the admin's with `scopes`.
    async fn token(&self, scopes: &[&str]) -> String {
        let created = self
            .as_admin(
                Method::POST,
                "/api/v1/account/tokens",
                Some(json!({"name": "ops", "scopes": scopes})),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        format!(
            "Bearer {}",
            created.body["token"].as_str().expect("a token")
        )
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn admins_see_the_version_and_check_for_the_newest_release(pool: PgPool) {
    let server = Server::start(pool).await;

    let before = server
        .as_admin(Method::GET, "/api/v1/admin/updates", None)
        .await;
    assert_eq!(before.status, StatusCode::OK, "{}", before.body);
    assert_eq!(
        before.body,
        json!({
            "current": "0.1.0",
            "development": false,
            "check": true,
            "prereleases": false,
            "latest": null,
            "newer": false,
            "checked_at": null,
            "error": null,
            "updater": {"set_up": false, "status": null, "error": null},
        })
    );

    let checked = server
        .as_admin(Method::POST, "/api/v1/admin/updates/check", None)
        .await;
    assert_eq!(checked.status, StatusCode::OK, "{}", checked.body);
    assert_eq!(
        checked.body["latest"],
        json!({
            "version": "0.2.0",
            "prerelease": false,
            "url": "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v0.2.0",
            "published_at": "2026-11-28T09:00:00Z",
        })
    );
    assert_eq!(checked.body["newer"], true);
    assert!(checked.body["checked_at"].is_string(), "{}", checked.body);

    server
        .state
        .settings
        .update(
            [("updates.prereleases".to_owned(), Change::Set(json!(true)))],
            None,
        )
        .await
        .expect("saved");
    let listed = server
        .as_admin(Method::GET, "/api/v1/admin/updates", None)
        .await;
    assert_eq!(listed.body["latest"]["version"], "0.3.0-rc.1");
    assert_eq!(listed.body["latest"]["prerelease"], true);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn only_instance_admins_see_check_and_update(pool: PgPool) {
    let server = Server::start(pool).await;
    user(&server.state, "bob@example.com").await;
    let bob = sign_in(&server.app, "bob@example.com").await;
    let bob = [(COOKIE.as_str(), bob.as_str())];

    for (method, uri, body) in [
        (Method::GET, "/api/v1/admin/updates", None),
        (Method::POST, "/api/v1/admin/updates/check", None),
        (
            Method::POST,
            "/api/v1/admin/updates/apply",
            Some(json!({"version": "0.2.0"})),
        ),
    ] {
        let refused = call(&server.app, method.clone(), uri, &bob, body.clone()).await;
        assert_eq!(
            (refused.status, refused.code()),
            (StatusCode::FORBIDDEN, "forbidden"),
            "{uri}"
        );
        let anonymous = call(&server.app, method, uri, &[], body).await;
        assert_eq!(
            (anonymous.status, anonymous.code()),
            (StatusCode::UNAUTHORIZED, "unauthenticated"),
            "{uri}"
        );
    }
    assert!(server.updater.asked().is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_admin_s_token_reads_and_checks_but_only_a_session_updates(pool: PgPool) {
    let server = Server::start(pool).await;
    server.set_up_updater().await;
    let token = server.token(&["read", "write", "admin"]).await;
    let token = [(AUTHORIZATION.as_str(), token.as_str())];

    let read = call(
        &server.app,
        Method::GET,
        "/api/v1/admin/updates",
        &token,
        None,
    )
    .await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    let checked = call(
        &server.app,
        Method::POST,
        "/api/v1/admin/updates/check",
        &token,
        None,
    )
    .await;
    assert_eq!(checked.status, StatusCode::OK, "{}", checked.body);

    let refused = call(
        &server.app,
        Method::POST,
        "/api/v1/admin/updates/apply",
        &token,
        Some(json!({"version": "0.2.0"})),
    )
    .await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    assert_eq!(
        refused.body["error"]["message"],
        "updating the server needs a browser session: an API token cannot"
    );
    assert!(server.updater.asked().is_empty());

    let reader = server.token(&["read"]).await;
    let reader = [(AUTHORIZATION.as_str(), reader.as_str())];
    let refused = call(
        &server.app,
        Method::GET,
        "/api/v1/admin/updates",
        &reader,
        None,
    )
    .await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::FORBIDDEN, "insufficient_scope")
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_admin_at_a_browser_has_the_updater_update_to_a_newer_release(pool: PgPool) {
    let server = Server::start(pool).await;
    server.set_up_updater().await;
    server
        .as_admin(Method::POST, "/api/v1/admin/updates/check", None)
        .await;

    let listed = server
        .as_admin(Method::GET, "/api/v1/admin/updates", None)
        .await;
    assert_eq!(
        listed.body["updater"],
        json!({
            "set_up": true,
            "status": {
                "state": "idle",
                "version": null,
                "error": null,
                "log": null,
                "started_at": null,
                "finished_at": null,
                "updater_version": "0.2.0",
            },
            "error": null,
        })
    );

    let applied = server
        .as_admin(
            Method::POST,
            "/api/v1/admin/updates/apply",
            Some(json!({"version": "0.2.0"})),
        )
        .await;
    assert_eq!(applied.status, StatusCode::ACCEPTED, "{}", applied.body);
    assert_eq!(applied.body["updater"]["status"]["state"], "updating");
    assert_eq!(applied.body["updater"]["status"]["version"], "0.2.0");
    assert_eq!(
        applied.body["updater"]["status"]["log"],
        "pulling the images\n"
    );
    assert_eq!(applied.body["latest"]["version"], "0.2.0");
    assert_eq!(server.updater.asked(), ["0.2.0"]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn each_refusal_of_an_update_has_its_code(pool: PgPool) {
    let server = Server::start(pool).await;
    server
        .as_admin(Method::POST, "/api/v1/admin/updates/check", None)
        .await;
    let apply = |version: Value| {
        server.as_admin(
            Method::POST,
            "/api/v1/admin/updates/apply",
            Some(json!({ "version": version })),
        )
    };

    let refused = apply(json!("0.2.0")).await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::CONFLICT, "no_updater")
    );

    server.set_up_updater().await;
    for version in ["0.1.0", "0.2.5", "0.3.0-rc.1", "latest"] {
        let refused = apply(json!(version)).await;
        assert_eq!(
            (refused.status, refused.code()),
            (StatusCode::BAD_REQUEST, "update_not_allowed"),
            "{version}: {}",
            refused.body
        );
    }

    server.updater.answer(Err(UpdaterError::Busy));
    let refused = apply(json!("0.2.0")).await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::CONFLICT, "update_in_progress")
    );

    server.updater.answer(Err(UpdaterError::Unreachable(
        "the updater at http://updater:8080/ did not answer: connecting to updater failed"
            .to_owned(),
    )));
    let refused = apply(json!("0.2.0")).await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::BAD_GATEWAY, "updater_unreachable")
    );
    assert_eq!(
        refused.body["error"]["message"],
        "the updater at http://updater:8080/ did not answer: connecting to updater failed"
    );

    server.updater.answer(Err(UpdaterError::Refused));
    let refused = apply(json!("0.2.0")).await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::BAD_GATEWAY, "updater_unreachable")
    );
    assert!(
        !refused.body.to_string().contains(TOKEN),
        "{}",
        refused.body
    );

    let refused = apply(json!(2)).await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::BAD_REQUEST, "invalid_input")
    );
    let refused = server
        .as_admin(
            Method::POST,
            "/api/v1/admin/updates/apply",
            Some(json!({"version": "0.2.0", "force": true})),
        )
        .await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::BAD_REQUEST, "invalid_input")
    );
    assert_eq!(server.updater.asked(), ["0.2.0", "0.2.0", "0.2.0"]);
}
