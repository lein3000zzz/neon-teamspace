//! Updates: whether a newer release of Neon Teamspace is out, and having the stack updated
//! to one.
//!
//! [`UpdateService`] asks a [`Releases`] port, GitHub's list of releases, what is out: once
//! a day while `updates.check` is on, an hour after a check that failed, and whenever an
//! instance admin asks. It keeps what the last check that worked found, through the checks
//! that fail after it. Which of those releases is the newest is decided each time the
//! status is read, so `updates.prereleases` applies at once: pre-releases count only while
//! it is on, and drafts never. A development build is never behind a release. While the
//! check is off, the server asks GitHub nothing and keeps nothing.
//!
//! Where an updater is set up, which `updates.updater_token` says, the status asks it how
//! its latest update went, waiting [`UPDATER_WAIT`] at most, and an instance admin at a
//! browser has it update the stack, through the [`Updater`] port, to a release newer than
//! the server's that the last check saw: never back, and never to a tag GitHub did not list.
//!
//! Every public use case runs in a span named `updates <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`], the periodic checks as `check`. A refusal, and a GitHub or an
//! updater that does not answer, count as `ok`: none is a failure of the server.

mod error;
mod ports;
#[cfg(test)]
mod tests;

use std::convert::Infallible;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use neon_domain::auth::Principal;
use neon_domain::duration;
use neon_domain::settings::Snapshot;
use neon_domain::updates::{Release, Running, UpdaterStatus, newest};
use neon_observability::errors::mark_expected;
use neon_observability::red::Red;
use semver::Version;
use time::OffsetDateTime;
use tracing::{Instrument as _, info, info_span, warn};
use url::Url;

use crate::clock::Clock;
use crate::periodic::Periodic;
use crate::settings::{SettingsService, updates as keys};

pub use error::UpdatesError;
pub use ports::{Releases, ReleasesError, Updater, UpdaterEndpoint, UpdaterError};

/// RED subsystem of the updates use cases.
pub const SUBSYSTEM: &str = "updates";

/// How long after a check that worked the next one is due: releases are days apart.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 3600);

/// How long after a check that failed the next one is due.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(3600);

/// How often the server looks whether a check is due, and so how soon a check follows
/// `updates.check` turned on.
pub const LOOK_INTERVAL: Duration = Duration::from_secs(60);

/// How long the status waits for the updater to say how it is.
pub const UPDATER_WAIT: Duration = Duration::from_secs(3);

/// How long asking the updater for an update may take.
pub const APPLY_WAIT: Duration = Duration::from_secs(10);

/// What is known of updates, for instance admins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    /// The build the server runs.
    pub running: Running,
    /// `updates.check`: off, the server asks GitHub nothing.
    pub check: bool,
    /// `updates.prereleases`.
    pub prereleases: bool,
    /// The newest release the last check that worked saw, as the settings count releases
    /// now. Unset while the check is off.
    pub latest: Option<Release>,
    /// The latest release is newer than the server's, never so for a development build.
    pub newer: bool,
    /// When the last check ran, and why it failed, when it did.
    pub checked_at: Option<OffsetDateTime>,
    pub error: Option<String>,
    pub updater: UpdaterReport,
}

/// The updater, as the status found it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdaterReport {
    /// One is set up: `updates.updater_token` is set.
    pub set_up: bool,
    /// Its latest update, when it answered.
    pub status: Option<UpdaterStatus>,
    /// Why it could not be asked, when it did not answer.
    pub error: Option<String>,
}

/// Reads the settings as they are now.
type ReadSettings = Arc<dyn Fn() -> Arc<Snapshot> + Send + Sync>;

/// The updates use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct UpdateService {
    inner: Arc<Inner>,
}

struct Inner {
    settings: ReadSettings,
    releases: Arc<dyn Releases>,
    updater: Arc<dyn Updater>,
    running: Running,
    clock: Clock,
    /// Held through a check, so that an admin's and the periodic one never ask together.
    checking: tokio::sync::Mutex<()>,
    checked: Mutex<Checked>,
    red: Red,
}

/// What the checks found.
#[derive(Debug, Default)]
struct Checked {
    /// What the last check that worked found, as the source listed it.
    releases: Option<Vec<Release>>,
    /// When the last check ran, and why it failed, when it did.
    at: Option<OffsetDateTime>,
    error: Option<String>,
    /// When the next check is due; unset, at the next look.
    due: Option<OffsetDateTime>,
    /// Checks have failed since the last that worked, which the log said once.
    failing: bool,
    /// The newer release the log last told of, so that it tells of each once.
    announced: Option<Version>,
}

/// Puts an [`UpdateService`] together. Without being told otherwise it reads the system
/// clock.
pub struct UpdateServiceBuilder {
    settings: ReadSettings,
    releases: Arc<dyn Releases>,
    updater: Arc<dyn Updater>,
    running: Running,
    clock: Clock,
}

impl UpdateServiceBuilder {
    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn build(self) -> UpdateService {
        UpdateService {
            inner: Arc::new(Inner {
                settings: self.settings,
                releases: self.releases,
                updater: self.updater,
                running: self.running,
                clock: self.clock,
                checking: tokio::sync::Mutex::new(()),
                checked: Mutex::new(Checked::default()),
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

impl UpdateService {
    /// The updates of the server that runs `running`, asking `releases` what is out and
    /// `updater` to update, as `settings` say.
    pub fn builder(
        settings: Arc<SettingsService>,
        releases: Arc<dyn Releases>,
        updater: Arc<dyn Updater>,
        running: Running,
    ) -> UpdateServiceBuilder {
        Self::reading(
            Arc::new(move || settings.current()),
            releases,
            updater,
            running,
        )
    }

    /// As [`builder`](Self::builder), reading the settings with `settings`.
    fn reading(
        settings: ReadSettings,
        releases: Arc<dyn Releases>,
        updater: Arc<dyn Updater>,
        running: Running,
    ) -> UpdateServiceBuilder {
        UpdateServiceBuilder {
            settings,
            releases,
            updater,
            running,
            clock: Clock::system(),
        }
    }

    /// What is known of updates, asking the updater how it is where one is set up.
    pub async fn status(&self, actor: &Principal) -> Result<UpdateStatus, UpdatesError> {
        self.observe("status", async {
            require_admin(actor)?;
            Ok(self.report().await)
        })
        .await
    }

    /// Asks GitHub again now, while the check is on, then says what is known. Off, it asks
    /// nothing.
    pub async fn check_now(&self, actor: &Principal) -> Result<UpdateStatus, UpdatesError> {
        self.observe("check_now", async {
            require_admin(actor)?;
            if *self.settings().get(keys::CHECK) {
                self.check().await;
            }
            Ok(self.report().await)
        })
        .await
    }

    /// Has the updater update the stack to `version`, for an instance admin at a browser:
    /// a release newer than the server's that the last check saw. Answers what is known,
    /// with where the updater is.
    pub async fn apply(
        &self,
        actor: &Principal,
        version: &str,
    ) -> Result<UpdateStatus, UpdatesError> {
        self.observe("apply", async {
            require_admin(actor)?;
            if !actor.via_session() {
                return Err(UpdatesError::Forbidden(
                    "updating the server needs a browser session: an API token cannot",
                ));
            }
            let settings = self.settings();
            let endpoint = match endpoint(&settings) {
                None => return Err(UpdatesError::NoUpdater),
                Some(Err(problem)) => return Err(UpdatesError::UpdaterUnreachable(problem)),
                Some(Ok(endpoint)) => endpoint,
            };
            let version = self.allowed(&settings, version)?;
            let answer =
                tokio::time::timeout(APPLY_WAIT, self.inner.updater.update(&endpoint, &version))
                    .await
                    .map_err(|_| UpdatesError::UpdaterUnreachable(no_answer(APPLY_WAIT)))?
                    .map_err(|error| match error {
                        UpdaterError::Busy => UpdatesError::UpdateInProgress,
                        UpdaterError::Rejected(why) => UpdatesError::NotAllowed(format!(
                            "the updater refused {version}: {why}"
                        )),
                        other => UpdatesError::UpdaterUnreachable(other.to_string()),
                    })?;
            info!(
                actor_id = %actor.user.id,
                version = %version,
                running = %self.inner.running,
                "the updater was asked to update the stack to a release"
            );
            let mut status = self.known(&settings);
            status.updater = UpdaterReport {
                set_up: true,
                status: Some(answer),
                error: None,
            };
            Ok(status)
        })
        .await
    }

    /// Checks when a check is due: [`CHECK_INTERVAL`] after the last that worked,
    /// [`RETRY_INTERVAL`] after one that failed, and at once when none ran since the check
    /// was turned on. While it is off, what the checks found is forgotten.
    pub async fn look(&self) {
        if !*self.settings().get(keys::CHECK) {
            *self.checked() = Checked::default();
            return;
        }
        let now = self.inner.clock.now();
        if self.checked().due.is_none_or(|due| now >= due) {
            // A check never fails as a use case: what went wrong is kept for the status.
            let span = info_span!(
                "updates",
                otel.name = "updates check",
                updates.operation = "check",
            );
            let guard = self.inner.red.start("check");
            self.check().instrument(span).await;
            guard.ok();
        }
    }

    /// Asks GitHub, and keeps what it answers. While a check is under way already, it waits
    /// for that one instead, whose answer is as fresh, so a check never waits for two.
    async fn check(&self) {
        let Ok(_turn) = self.inner.checking.try_lock() else {
            drop(self.inner.checking.lock().await);
            return;
        };
        let found = self.inner.releases.list().await;
        let now = self.inner.clock.now();
        let prereleases = *self.settings().get(keys::PRERELEASES);
        let mut checked = self.checked();
        match found {
            Ok(releases) => {
                if checked.failing {
                    info!(releases = releases.len(), "GitHub lists the releases again");
                }
                if let Some(release) = newest(&releases, prereleases)
                    && self.inner.running.is_behind(&release.version)
                    && checked.announced.as_ref() != Some(&release.version)
                {
                    info!(
                        version = %release.version,
                        running = %self.inner.running,
                        "a newer release is out"
                    );
                    checked.announced = Some(release.version.clone());
                }
                checked.releases = Some(releases);
                checked.error = None;
                checked.due = Some(now + CHECK_INTERVAL);
                checked.failing = false;
            }
            Err(error) => {
                if !checked.failing {
                    warn!(%error, "the release check failed: it tries again in an hour");
                }
                checked.error = Some(error.to_string());
                checked.due = Some(now + RETRY_INTERVAL);
                checked.failing = true;
            }
        }
        checked.at = Some(now);
    }

    /// What is known, with the updater asked how it is.
    async fn report(&self) -> UpdateStatus {
        let settings = self.settings();
        let mut status = self.known(&settings);
        status.updater = self.ask_updater(&settings).await;
        status
    }

    /// What the checks found, as `settings` count releases; the updater left unasked.
    fn known(&self, settings: &Snapshot) -> UpdateStatus {
        let check = *settings.get(keys::CHECK);
        let prereleases = *settings.get(keys::PRERELEASES);
        let mut status = UpdateStatus {
            running: self.inner.running.clone(),
            check,
            prereleases,
            latest: None,
            newer: false,
            checked_at: None,
            error: None,
            updater: UpdaterReport::default(),
        };
        if check {
            let checked = self.checked();
            status.checked_at = checked.at;
            status.error.clone_from(&checked.error);
            status.latest = checked
                .releases
                .as_deref()
                .and_then(|releases| newest(releases, prereleases))
                .cloned();
        }
        status.newer = status
            .latest
            .as_ref()
            .is_some_and(|release| self.inner.running.is_behind(&release.version));
        status
    }

    /// How the updater is, where one is set up.
    async fn ask_updater(&self, settings: &Snapshot) -> UpdaterReport {
        let endpoint = match endpoint(settings) {
            None => return UpdaterReport::default(),
            Some(Err(problem)) => {
                return UpdaterReport {
                    set_up: true,
                    status: None,
                    error: Some(problem),
                };
            }
            Some(Ok(endpoint)) => endpoint,
        };
        let asked = tokio::time::timeout(UPDATER_WAIT, self.inner.updater.status(&endpoint));
        let (status, error) = match asked.await {
            Ok(Ok(status)) => (Some(status), None),
            Ok(Err(error)) => (None, Some(error.to_string())),
            Err(_) => (None, Some(no_answer(UPDATER_WAIT))),
        };
        UpdaterReport {
            set_up: true,
            status,
            error,
        }
    }

    /// `asked` as the version to update to, when it is an update the server may take.
    fn allowed(&self, settings: &Snapshot, asked: &str) -> Result<Version, UpdatesError> {
        let refuse = |why: String| Err(UpdatesError::NotAllowed(why));
        let Ok(version) = Version::parse(asked) else {
            return refuse("want a release's version, such as 0.2.1".to_owned());
        };
        let running = &self.inner.running;
        if running.development {
            return refuse(
                "the server is a development build, which is updated from its code, not to a \
                 release"
                    .to_owned(),
            );
        }
        if !*settings.get(keys::CHECK) {
            return refuse(
                "the release check is off, so no release is known: turn updates.check on"
                    .to_owned(),
            );
        }
        let prereleases = *settings.get(keys::PRERELEASES);
        let seen = self.checked().releases.as_deref().is_some_and(|releases| {
            releases
                .iter()
                .any(|release| release.counts(prereleases) && release.version == version)
        });
        if !seen {
            return refuse(format!(
                "{version} is not a release the last check saw on GitHub: check again first"
            ));
        }
        if !running.is_behind(&version) {
            return refuse(format!(
                "{version} is not newer than {running}, which the server runs"
            ));
        }
        Ok(version)
    }

    fn settings(&self) -> Arc<Snapshot> {
        (self.inner.settings)()
    }

    fn checked(&self) -> MutexGuard<'_, Checked> {
        self.inner
            .checked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, UpdatesError>>,
    ) -> Result<T, UpdatesError> {
        let span = info_span!(
            "updates",
            otel.name = %format!("updates {operation}"),
            updates.operation = operation,
        );
        let guard = self.inner.red.start(operation);
        let result = work.instrument(span.clone()).await;
        if let Err(error) = &result {
            mark_expected(&span, error.code());
        }
        guard.ok();
        result
    }
}

/// The running release check: it looks whether a check is due at once, then every
/// interval until stopped.
#[must_use = "dropping the handle leaves the task running with no way to stop it"]
pub struct ReleaseChecks(Periodic);

impl ReleaseChecks {
    pub fn spawn(updates: UpdateService, interval: Duration) -> Self {
        Self(Periodic::spawn("release check", interval, move || {
            let updates = updates.clone();
            async move {
                updates.look().await;
                Ok::<(), Infallible>(())
            }
        }))
    }

    /// Stops, waiting for a check in progress.
    pub async fn stop(self) {
        self.0.stop().await;
    }
}

fn require_admin(actor: &Principal) -> Result<(), UpdatesError> {
    if actor.is_admin() {
        Ok(())
    } else {
        Err(UpdatesError::Forbidden(
            "only an instance admin may do this",
        ))
    }
}

/// Where the updater is, with its token: unset while no updater is set up, and why it
/// cannot be reached when `updates.updater_url` is no URL.
fn endpoint(settings: &Snapshot) -> Option<Result<UpdaterEndpoint, String>> {
    let token = settings.get(keys::UPDATER_TOKEN);
    if token.expose().is_empty() {
        return None;
    }
    let url = Url::parse(settings.get(keys::UPDATER_URL))
        .map_err(|_| "updates.updater_url is not a URL".to_owned());
    Some(url.map(|url| UpdaterEndpoint {
        url,
        token: token.clone(),
    }))
}

fn no_answer(wait: Duration) -> String {
    format!(
        "the updater did not answer within {}",
        duration::format(wait)
    )
}
