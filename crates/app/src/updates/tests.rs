//! The release check and updates, with GitHub and the updater replaced by fakes, the
//! settings by snapshots the tests change, and the time by a clock they move.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use neon_domain::auth::{Credential, Email, Role, Scope, User, UserKind};
use neon_domain::settings::{Pinned, Secret};
use neon_domain::updates::UpdaterState;
use serde_json::{Value as Json, json};
use time::macros::datetime;
use uuid::Uuid;

use super::*;
use crate::clock::ManualTime;
use crate::settings::registry;

const START: OffsetDateTime = datetime!(2026-11-28 09:00 UTC);

/// A token that the setting takes, obviously fake.
const TOKEN: &str = "fake-updater-token-for-the-tests-only";

/// GitHub, answering from a queue that keeps its last answer, and counting the times it
/// was asked. With a gate, each answer waits for a permit of it.
#[derive(Default)]
struct FakeReleases {
    answers: Mutex<VecDeque<Result<Vec<Release>, ReleasesError>>>,
    asked: AtomicUsize,
    gate: Option<Arc<tokio::sync::Semaphore>>,
}

impl FakeReleases {
    fn answer(&self, answer: Result<Vec<Release>, ReleasesError>) {
        let mut answers = self.answers.lock().expect("unpoisoned");
        answers.clear();
        answers.push_back(answer);
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Releases for FakeReleases {
    async fn list(&self) -> Result<Vec<Release>, ReleasesError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.acquire().await.expect("open").forget();
        }
        let mut answers = self.answers.lock().expect("unpoisoned");
        if answers.len() > 1 {
            answers.pop_front().expect("an answer")
        } else {
            answers.front().cloned().unwrap_or(Ok(Vec::new()))
        }
    }
}

/// The updater: its status, unset to never answer, what it answers to an update, and
/// every request it was sent.
struct FakeUpdater {
    status: Mutex<Option<Result<UpdaterStatus, UpdaterError>>>,
    update: Mutex<Result<UpdaterStatus, UpdaterError>>,
    requests: Mutex<Vec<(Url, String, Option<Version>)>>,
}

impl Default for FakeUpdater {
    fn default() -> Self {
        Self {
            status: Mutex::new(Some(Ok(updater_status(UpdaterState::Idle, None)))),
            update: Mutex::new(Ok(updater_status(UpdaterState::Updating, Some("0.2.0")))),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl FakeUpdater {
    /// The versions it was asked to update to.
    fn updates(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("unpoisoned")
            .iter()
            .filter_map(|(_, _, version)| version.as_ref().map(ToString::to_string))
            .collect()
    }
}

#[async_trait]
impl Updater for FakeUpdater {
    async fn status(&self, at: &UpdaterEndpoint) -> Result<UpdaterStatus, UpdaterError> {
        self.requests.lock().expect("unpoisoned").push((
            at.url.clone(),
            at.token.expose().to_owned(),
            None,
        ));
        let answer = self.status.lock().expect("unpoisoned").clone();
        match answer {
            Some(answer) => answer,
            None => std::future::pending().await,
        }
    }

    async fn update(
        &self,
        at: &UpdaterEndpoint,
        version: &Version,
    ) -> Result<UpdaterStatus, UpdaterError> {
        self.requests.lock().expect("unpoisoned").push((
            at.url.clone(),
            at.token.expose().to_owned(),
            Some(version.clone()),
        ));
        self.update.lock().expect("unpoisoned").clone()
    }
}

fn updater_status(state: UpdaterState, version: Option<&str>) -> UpdaterStatus {
    UpdaterStatus {
        state,
        version: version.map(str::to_owned),
        error: None,
        log: None,
        started_at: None,
        finished_at: None,
        updater_version: "0.2.0".to_owned(),
    }
}

fn release(version: &str) -> Release {
    Release {
        version: Version::parse(version).expect("a version"),
        prerelease: false,
        draft: false,
        url: Url::parse(&format!(
            "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v{version}"
        ))
        .expect("a URL"),
        published_at: Some(START),
    }
}

/// The settings with `values` saved, the rest at their defaults.
fn snapshot(values: &[(&str, Json)]) -> Arc<Snapshot> {
    let registry = registry();
    let (stored, problems) = registry.parse_stored(
        values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone())),
    );
    assert!(problems.is_empty(), "{problems:?}");
    Arc::new(registry.resolve(&Pinned::default(), &stored))
}

fn person(role: Role) -> User {
    User {
        id: Uuid::now_v7(),
        kind: UserKind::Human,
        email: Some(Email::parse("grace@example.com").expect("an address")),
        display_name: "Grace".into(),
        role,
        hue_override: None,
        email_verified_at: Some(START),
        disabled_at: None,
        created_at: START,
        updated_at: START,
        workspace_id: None,
        owner_id: None,
    }
}

fn session(role: Role) -> Principal {
    Principal {
        user: person(role),
        credential: Credential::Session { id: Uuid::now_v7() },
    }
}

fn admin_token() -> Principal {
    Principal {
        user: person(Role::Admin),
        credential: Credential::Token {
            id: Uuid::now_v7(),
            name: "ops".into(),
            scopes: vec![Scope::Read, Scope::Write, Scope::Admin],
            workspace_id: None,
        },
    }
}

struct Fixture {
    updates: UpdateService,
    releases: Arc<FakeReleases>,
    updater: Arc<FakeUpdater>,
    settings: Arc<Mutex<Arc<Snapshot>>>,
    time: ManualTime,
    admin: Principal,
}

impl Fixture {
    fn new(running: &str) -> Self {
        Self::build(running, false)
    }

    fn build(running: &str, development: bool) -> Self {
        Self::with_releases(running, development, FakeReleases::default())
    }

    fn with_releases(running: &str, development: bool, releases: FakeReleases) -> Self {
        let releases = Arc::new(releases);
        let updater = Arc::new(FakeUpdater::default());
        let settings = Arc::new(Mutex::new(snapshot(&[])));
        let time = ManualTime::new(START);
        let read = Arc::clone(&settings);
        let updates = UpdateService::reading(
            Arc::new(move || Arc::clone(&read.lock().expect("unpoisoned"))),
            Arc::clone(&releases) as Arc<dyn Releases>,
            Arc::clone(&updater) as Arc<dyn Updater>,
            Running {
                version: Version::parse(running).expect("a version"),
                development,
            },
        )
        .clock(time.clock())
        .build();
        Self {
            updates,
            releases,
            updater,
            settings,
            time,
            admin: session(Role::Admin),
        }
    }

    /// The settings with `values` saved from now, the rest at their defaults.
    fn set(&self, values: &[(&str, Json)]) {
        *self.settings.lock().expect("unpoisoned") = snapshot(values);
    }

    /// With an updater set up at its default URL.
    fn with_updater(self) -> Self {
        self.set(&[("updates.updater_token", json!(TOKEN))]);
        self
    }

    async fn status(&self) -> UpdateStatus {
        self.updates.status(&self.admin).await.expect("the status")
    }

    async fn apply(&self, version: &str) -> Result<UpdateStatus, UpdatesError> {
        self.updates.apply(&self.admin, version).await
    }

    /// The fixture after a check that found 0.1.0, 0.2.0 and 0.3.0-rc.1.
    async fn checked(self) -> Self {
        self.releases.answer(Ok(vec![
            release("0.3.0-rc.1"),
            release("0.2.0"),
            release("0.1.0"),
        ]));
        self.updates.look().await;
        self
    }
}

fn latest(status: &UpdateStatus) -> Option<String> {
    status
        .latest
        .as_ref()
        .map(|release| release.version.to_string())
}

#[tokio::test]
async fn checks_at_once_then_once_a_day() {
    let fixture = Fixture::new("0.1.0");
    let before = fixture.status().await;
    assert!(before.check);
    assert_eq!((before.latest, before.checked_at), (None, None));
    assert_eq!(before.running.version.to_string(), "0.1.0");

    fixture
        .releases
        .answer(Ok(vec![release("0.1.0"), release("0.2.0")]));
    fixture.updates.look().await;

    let status = fixture.status().await;
    assert_eq!(latest(&status).as_deref(), Some("0.2.0"));
    assert!(status.newer);
    assert_eq!(status.checked_at, Some(START));
    assert_eq!(status.error, None);
    assert_eq!(
        status.latest.map(|release| release.url.to_string()),
        Some("https://github.com/lein3000zzz/neon-teamspace/releases/tag/v0.2.0".to_owned())
    );

    fixture.time.advance(Duration::from_secs(23 * 3600));
    fixture.updates.look().await;
    assert_eq!(fixture.releases.asked(), 1, "not due for a day");

    fixture.time.advance(Duration::from_secs(3600));
    fixture.updates.look().await;
    assert_eq!(fixture.releases.asked(), 2);
}

#[tokio::test]
async fn a_failed_check_keeps_what_the_last_found_and_tries_again_in_an_hour() {
    let fixture = Fixture::new("0.1.0").checked().await;
    fixture.time.advance(CHECK_INTERVAL);
    fixture.releases.answer(Err(ReleasesError(
        "GitHub answered 403: rate limited".to_owned(),
    )));
    fixture.updates.look().await;

    let status = fixture.status().await;
    assert_eq!(
        latest(&status).as_deref(),
        Some("0.2.0"),
        "the last answer stays"
    );
    assert_eq!(
        status.error.as_deref(),
        Some("GitHub answered 403: rate limited")
    );
    assert_eq!(status.checked_at, Some(START + CHECK_INTERVAL));

    fixture.time.advance(Duration::from_secs(59 * 60));
    fixture.updates.look().await;
    assert_eq!(fixture.releases.asked(), 2, "not due yet");

    fixture.releases.answer(Ok(vec![release("0.2.1")]));
    fixture.time.advance(Duration::from_secs(60));
    fixture.updates.look().await;
    let status = fixture.status().await;
    assert_eq!(fixture.releases.asked(), 3);
    assert_eq!(latest(&status).as_deref(), Some("0.2.1"));
    assert_eq!(status.error, None);
}

#[tokio::test]
async fn pre_releases_count_while_they_are_on_without_asking_again() {
    let fixture = Fixture::new("0.1.0").checked().await;
    assert_eq!(latest(&fixture.status().await).as_deref(), Some("0.2.0"));

    fixture.set(&[("updates.prereleases", json!(true))]);
    let status = fixture.status().await;
    assert_eq!(latest(&status).as_deref(), Some("0.3.0-rc.1"));
    assert!(status.prereleases && status.newer);
    assert_eq!(fixture.releases.asked(), 1);
}

#[tokio::test]
async fn drafts_never_count() {
    let fixture = Fixture::new("0.1.0");
    let draft = Release {
        draft: true,
        published_at: None,
        ..release("0.5.0")
    };
    fixture.releases.answer(Ok(vec![draft, release("0.1.0")]));
    fixture.updates.look().await;

    let status = fixture.status().await;
    assert_eq!(latest(&status).as_deref(), Some("0.1.0"));
    assert!(!status.newer, "the server runs it");
    assert_eq!(
        fixture.with_updater().apply("0.5.0").await,
        Err(UpdatesError::NotAllowed(
            "0.5.0 is not a release the last check saw on GitHub: check again first".to_owned()
        ))
    );
}

#[tokio::test]
async fn off_the_server_asks_nothing_and_keeps_nothing() {
    let fixture = Fixture::new("0.1.0").checked().await;
    fixture.set(&[("updates.check", json!(false))]);

    fixture.time.advance(Duration::from_secs(48 * 3600));
    fixture.updates.look().await;
    let status = fixture
        .updates
        .check_now(&fixture.admin)
        .await
        .expect("the status");
    assert_eq!(fixture.releases.asked(), 1, "GitHub was asked nothing");
    assert!(!status.check);
    assert_eq!(
        (status.latest, status.checked_at, status.error),
        (None, None, None)
    );

    fixture.set(&[]);
    assert_eq!(
        fixture.status().await.latest,
        None,
        "what it found was forgotten"
    );
    fixture.updates.look().await;
    assert_eq!(fixture.releases.asked(), 2, "on again, it asks at once");
}

#[tokio::test]
async fn an_admin_checks_whenever_they_ask() {
    let fixture = Fixture::new("0.1.0").checked().await;
    fixture.releases.answer(Ok(vec![release("0.2.1")]));

    let status = fixture
        .updates
        .check_now(&fixture.admin)
        .await
        .expect("the status");

    assert_eq!(fixture.releases.asked(), 2);
    assert_eq!(latest(&status).as_deref(), Some("0.2.1"));
    fixture.updates.look().await;
    assert_eq!(fixture.releases.asked(), 2, "the next is due a day later");
}

#[tokio::test]
async fn a_development_build_is_never_behind() {
    let fixture = Fixture::build("0.1.0", true).checked().await.with_updater();

    let status = fixture.status().await;
    assert_eq!(latest(&status).as_deref(), Some("0.2.0"));
    assert!(!status.newer);
    assert!(status.running.development);
    let refused = fixture.apply("0.2.0").await;
    assert!(
        matches!(&refused, Err(UpdatesError::NotAllowed(why)) if why.contains("development build")),
        "{refused:?}"
    );
    assert!(fixture.updater.updates().is_empty());
}

#[tokio::test]
async fn only_instance_admins_see_and_check_and_only_at_a_browser_update() {
    let fixture = Fixture::new("0.1.0").checked().await.with_updater();
    let member = session(Role::User);
    let forbidden = Err(UpdatesError::Forbidden(
        "only an instance admin may do this",
    ));

    assert_eq!(fixture.updates.status(&member).await, forbidden);
    assert_eq!(fixture.updates.check_now(&member).await, forbidden);
    assert_eq!(fixture.updates.apply(&member, "0.2.0").await, forbidden);

    let token = admin_token();
    assert!(fixture.updates.status(&token).await.is_ok());
    assert_eq!(
        fixture.updates.apply(&token, "0.2.0").await,
        Err(UpdatesError::Forbidden(
            "updating the server needs a browser session: an API token cannot"
        ))
    );
    assert!(fixture.updater.updates().is_empty());
}

#[tokio::test]
async fn an_update_goes_to_the_updater_with_its_token() {
    let fixture = Fixture::new("0.1.0").checked().await.with_updater();

    let status = fixture.apply("0.2.0").await.expect("asked");

    assert_eq!(fixture.updater.updates(), ["0.2.0"]);
    let requests = fixture.updater.requests.lock().expect("unpoisoned").clone();
    assert_eq!(requests[0].0.as_str(), "http://updater:8080/");
    assert_eq!(requests[0].1, TOKEN);
    assert!(status.updater.set_up);
    assert_eq!(
        status.updater.status,
        Some(updater_status(UpdaterState::Updating, Some("0.2.0")))
    );
    assert_eq!(latest(&status).as_deref(), Some("0.2.0"));
}

#[tokio::test]
async fn only_a_newer_release_the_last_check_saw_is_an_update() {
    let not_allowed = |why: &str| Err(UpdatesError::NotAllowed(why.to_owned()));
    let cases = [
        (
            "0.1.0",
            "0.2",
            not_allowed("want a release's version, such as 0.2.1"),
        ),
        (
            "0.1.0",
            "v0.2.0",
            not_allowed("want a release's version, such as 0.2.1"),
        ),
        (
            "0.1.0",
            "0.2.5",
            not_allowed("0.2.5 is not a release the last check saw on GitHub: check again first"),
        ),
        (
            "0.1.0",
            "0.3.0-rc.1",
            not_allowed(
                "0.3.0-rc.1 is not a release the last check saw on GitHub: check again first",
            ),
        ),
        (
            "0.2.0",
            "0.2.0",
            not_allowed("0.2.0 is not newer than 0.2.0, which the server runs"),
        ),
        (
            "0.2.1",
            "0.2.0",
            not_allowed("0.2.0 is not newer than 0.2.1, which the server runs"),
        ),
    ];
    for (running, asked, refused) in cases {
        let fixture = Fixture::new(running).checked().await.with_updater();

        assert_eq!(
            fixture.apply(asked).await,
            refused,
            "{asked} over {running}"
        );
        assert!(
            fixture.updater.updates().is_empty(),
            "{asked} over {running}"
        );
    }

    let fixture = Fixture::new("0.1.0").checked().await;
    assert_eq!(fixture.apply("0.2.0").await, Err(UpdatesError::NoUpdater));

    let fixture = Fixture::new("0.1.0").with_updater();
    assert_eq!(
        fixture.apply("0.2.0").await,
        not_allowed("0.2.0 is not a release the last check saw on GitHub: check again first"),
        "nothing checked, nothing to update to"
    );

    let fixture = Fixture::new("0.1.0").checked().await;
    fixture.set(&[
        ("updates.updater_token", json!(TOKEN)),
        ("updates.check", json!(false)),
    ]);
    assert_eq!(
        fixture.apply("0.2.0").await,
        not_allowed("the release check is off, so no release is known: turn updates.check on")
    );
}

#[tokio::test]
async fn an_older_release_the_check_saw_is_an_update_too_and_a_pre_release_once_on() {
    let fixture = Fixture::new("0.0.9").checked().await.with_updater();
    assert!(fixture.apply("0.1.0").await.is_ok());

    fixture.set(&[
        ("updates.updater_token", json!(TOKEN)),
        ("updates.prereleases", json!(true)),
    ]);
    assert!(fixture.apply("0.3.0-rc.1").await.is_ok());
    assert_eq!(fixture.updater.updates(), ["0.1.0", "0.3.0-rc.1"]);
}

#[tokio::test]
async fn the_updater_s_refusals_say_what_went_wrong() {
    let cases = [
        (UpdaterError::Busy, UpdatesError::UpdateInProgress),
        (
            UpdaterError::Rejected("no such release".to_owned()),
            UpdatesError::NotAllowed("the updater refused 0.2.0: no such release".to_owned()),
        ),
        (
            UpdaterError::Refused,
            UpdatesError::UpdaterUnreachable(UpdaterError::Refused.to_string()),
        ),
        (
            UpdaterError::Unreachable("the updater did not answer at http://updater:8080".into()),
            UpdatesError::UpdaterUnreachable(
                "the updater did not answer at http://updater:8080".into(),
            ),
        ),
    ];
    for (answer, refused) in cases {
        let fixture = Fixture::new("0.1.0").checked().await.with_updater();
        *fixture.updater.update.lock().expect("unpoisoned") = Err(answer);

        assert_eq!(fixture.apply("0.2.0").await, Err(refused));
    }
}

#[tokio::test]
async fn the_status_asks_the_updater_only_where_one_is_set_up() {
    let fixture = Fixture::new("0.1.0");
    let status = fixture.status().await;
    assert_eq!(status.updater, UpdaterReport::default());
    assert!(
        fixture
            .updater
            .requests
            .lock()
            .expect("unpoisoned")
            .is_empty()
    );

    let fixture = fixture.with_updater();
    let status = fixture.status().await;
    assert_eq!(
        status.updater,
        UpdaterReport {
            set_up: true,
            status: Some(updater_status(UpdaterState::Idle, None)),
            error: None,
        }
    );

    *fixture.updater.status.lock().expect("unpoisoned") = Some(Err(UpdaterError::Unreachable(
        "the updater did not answer at http://updater:8080".into(),
    )));
    let status = fixture.status().await;
    assert_eq!(
        status.updater.error.as_deref(),
        Some("the updater did not answer at http://updater:8080")
    );
    assert_eq!(status.updater.status, None);
}

#[tokio::test(start_paused = true)]
async fn the_status_waits_three_seconds_for_the_updater() {
    let fixture = Fixture::new("0.1.0").with_updater();
    *fixture.updater.status.lock().expect("unpoisoned") = None;

    let started = tokio::time::Instant::now();
    let status = fixture.status().await;

    assert_eq!(started.elapsed(), UPDATER_WAIT);
    assert_eq!(
        status.updater.error.as_deref(),
        Some("the updater did not answer within 3s")
    );
}

#[tokio::test]
async fn a_newer_release_is_logged_once() {
    let fixture = Fixture::new("0.1.0").checked().await;
    assert_eq!(
        fixture
            .updates
            .inner
            .checked
            .lock()
            .expect("unpoisoned")
            .announced
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("0.2.0")
    );
}

#[tokio::test(start_paused = true)]
async fn the_periodic_task_looks_at_once_then_every_interval() {
    let fixture = Fixture::new("0.1.0");
    fixture.releases.answer(Ok(vec![release("0.2.0")]));
    let checks = ReleaseChecks::spawn(fixture.updates.clone(), LOOK_INTERVAL);

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(fixture.releases.asked(), 1);

    fixture.time.advance(CHECK_INTERVAL);
    tokio::time::sleep(LOOK_INTERVAL).await;
    assert_eq!(fixture.releases.asked(), 2);
    tokio::time::sleep(LOOK_INTERVAL).await;
    assert_eq!(fixture.releases.asked(), 2, "not due again");

    checks.stop().await;
}

#[test]
fn the_updater_s_token_never_shows_in_its_endpoint() {
    let endpoint = UpdaterEndpoint {
        url: Url::parse("http://updater:8080").expect("a URL"),
        token: Secret::new(TOKEN),
    };

    assert!(!format!("{endpoint:?}").contains(TOKEN));
}

#[tokio::test]
async fn a_check_asked_during_another_waits_for_it_instead_of_asking_again() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let releases = FakeReleases {
        gate: Some(Arc::clone(&gate)),
        ..FakeReleases::default()
    };
    releases.answer(Ok(vec![release("0.2.0")]));
    let fixture = Fixture::with_releases("0.1.0", false, releases);

    let periodic = tokio::spawn({
        let updates = fixture.updates.clone();
        async move { updates.look().await }
    });
    while fixture.releases.asked() == 0 {
        tokio::task::yield_now().await;
    }
    let asked = tokio::spawn({
        let updates = fixture.updates.clone();
        let admin = fixture.admin.clone();
        async move { updates.check_now(&admin).await }
    });
    tokio::task::yield_now().await;
    // Enough for two answers, so asking twice would show in the count, not hang.
    gate.add_permits(2);

    let status = asked.await.expect("joined").expect("the status");
    periodic.await.expect("joined");
    assert_eq!(fixture.releases.asked(), 1);
    assert_eq!(latest(&status).as_deref(), Some("0.2.0"));
}
