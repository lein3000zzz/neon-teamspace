//! The MCP sessions of this process.
//!
//! [`Sessions`] is the transport's session manager: each session is one of the SDK's local
//! session workers, which its requests and streams go through, held here with the identity
//! bound to it and its subscriptions.
//!
//! - **Binding.** A session is bound to the identity whose `initialize` opened it, in the
//!   workspace its token is bound to. A request on it from another identity, or for another
//!   workspace, finds no session, as if it were not there. Each request's principal, read
//!   afresh from its token, replaces the one held, so a token renewed keeps the session.
//! - **Bounds.** `mcp.max_sessions` sessions at once, `mcp.max_sessions_per_identity` of one
//!   identity, and `mcp.max_sessions_per_person` of one person with their agents together,
//!   so nobody takes every session through agents of theirs; those being opened included.
//!   An identity at its bound that opens one more ends its session used least recently
//!   that has no request in flight and no stream open, as a client that crashed and lost
//!   its session ids would otherwise be shut out until they idle out. Past the other
//!   bounds, or when each of the identity's sessions is in use, one more is refused, saying
//!   when the next idle session ends.
//! - **Idle sessions** end: once a session has no request in flight and no stream open for
//!   `mcp.session_idle_timeout`, it ends with its subscriptions. An open stream counts as
//!   use, as its client listens; one whose connection broke closes as the transport's pings
//!   fail. A session the transport made for an `initialize` whose client went away before
//!   the answer is bound to nobody, and ends after [`UNBOUND`].
//! - **Access.** Every access change that may concern a session's identity, a token revoked,
//!   an account disabled, a member removed, an agent's owner leaving, agents switched off,
//!   makes the watcher look again at once: the session ends when its token no longer works,
//!   its identity is no longer in the workspace, or MCP clients may no longer act as it, and
//!   each subscription whose resource it no longer sees drops. A request on a session that
//!   the gate refuses makes it look again at that session, so one whose token expired ends
//!   at its next request. Every [`REVALIDATE`] it looks so at every session, for what no
//!   change tells of.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use futures_util::{Stream, StreamExt as _};
use metrics::Unit;
use neon_app::auth::AuthError;
use neon_app::pages::AccessChange;
use neon_app::settings::mcp as keys;
use neon_domain::agents::IdentityMode;
use neon_domain::auth::Principal;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::WorkerTransport;
use rmcp::transport::common::server_side_http::{ServerSseMessage, session_id};
use rmcp::transport::streamable_http_server::SessionId;
use rmcp::transport::streamable_http_server::session::SessionManager;
use rmcp::transport::streamable_http_server::session::local::{
    EventIdParseError, LocalSessionHandle, LocalSessionWorker, SessionConfig, SessionError,
    create_local_session,
};
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior, interval_at};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};
use uuid::Uuid;

use super::Services;
use super::resources;
use super::tools::ToolError;
use super::watch::Watch;

/// How often every session is looked at again, as an access change would make it.
pub(crate) const REVALIDATE: Duration = Duration::from_secs(300);

/// How long a session the transport made may stay bound to nobody: its `initialize` is
/// answered at once, so one older than this lost its client before the answer.
pub(crate) const UNBOUND: Duration = Duration::from_secs(60);

/// How many sessions the watcher looks at again at once.
const RECHECKS_AT_ONCE: usize = 8;

/// The most resources one session subscribes to.
pub(crate) const MAX_WATCHES: usize = 100;

/// The gauge of the sessions open.
const OPEN_SESSIONS: &str = "neon_mcp_sessions";
/// The gauge of the subscriptions held.
const SUBSCRIPTIONS: &str = "neon_mcp_subscriptions";

/// How many sessions there may be, and how long one idles before it ends.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) overall: usize,
    pub(crate) per_identity: usize,
    pub(crate) per_person: usize,
    pub(crate) idle: Duration,
}

/// The person a principal is, or acts for: an agent's owner. An agent whose owner's account
/// is gone stands for the nil id, as it stands nowhere.
pub(crate) fn person_of(principal: &Principal) -> Uuid {
    if principal.user.kind.is_human() {
        principal.user.id
    } else {
        principal.user.owner_id.unwrap_or_default()
    }
}

/// Why a session cannot open, with when the first of the sessions that count ends if it
/// stays idle.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum TooMany {
    #[error("this server holds {0} MCP sessions, the most it does: try again once one ends")]
    Overall(usize, Option<Duration>),
    #[error(
        "this identity holds {0} MCP sessions, the most it may, each with a request or a \
         stream open: end one with DELETE /mcp, or wait for one to idle out"
    )]
    Identity(usize, Option<Duration>),
    #[error(
        "the person this identity is or acts for holds {0} MCP sessions with their agents, the \
         most they may: end one with DELETE /mcp, or wait for one to idle out"
    )]
    Person(usize, Option<Duration>),
}

impl TooMany {
    /// How long until a session that counts ends, if one is idle now.
    pub(crate) fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Overall(_, wait) | Self::Identity(_, wait) | Self::Person(_, wait) => *wait,
        }
    }
}

/// What the session manager's operations fail with.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionFailure {
    #[error("no such session")]
    NotFound,
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    EventId(#[from] EventIdParseError),
}

/// The sessions, and what each is bound to.
pub(crate) struct Sessions {
    services: Services,
    config: SessionConfig,
    workers: Mutex<HashMap<SessionId, Worker>>,
    state: Mutex<State>,
    /// The watcher, and the sessions it or a request ends or looks again at, which
    /// [`close_all`](Self::close_all) waits for.
    tasks: TaskTracker,
    /// Stops the watcher: a child of the transport's token, which stops it too.
    watching: CancellationToken,
}

/// One of the transport's session workers, and when it was made.
struct Worker {
    handle: LocalSessionHandle,
    made: Instant,
}

#[derive(Default)]
struct State {
    bound: HashMap<SessionId, Binding>,
    /// Sessions being opened, by identity and by person: their `initialize` is on its way.
    opening: HashMap<Uuid, usize>,
    opening_people: HashMap<Uuid, usize>,
}

/// Takes one from a count of sessions being opened, forgetting it at none.
fn release(counts: &mut HashMap<Uuid, usize>, key: Uuid) {
    if let Some(count) = counts.get_mut(&key) {
        *count -= 1;
        if *count == 0 {
            counts.remove(&key);
        }
    }
}

impl State {
    fn opening(&self) -> usize {
        self.opening.values().sum()
    }

    fn of(&self, user: Uuid) -> usize {
        self.bound
            .values()
            .filter(|binding| binding.principal.user.id == user)
            .count()
            + self.opening.get(&user).copied().unwrap_or(0)
    }

    fn of_person(&self, person: Uuid) -> usize {
        self.bound
            .values()
            .filter(|binding| person_of(&binding.principal) == person)
            .count()
            + self.opening_people.get(&person).copied().unwrap_or(0)
    }

    fn subscriptions(&self) -> usize {
        self.bound
            .values()
            .map(|binding| binding.watches.len())
            .sum()
    }

    /// Of the sessions `counts` says count, the one used least recently with no request in
    /// flight and no stream open.
    fn idlest(&self, counts: impl Fn(&Binding) -> bool) -> Option<(&SessionId, &Binding)> {
        self.bound
            .iter()
            .filter(|(_, binding)| binding.active == 0 && counts(binding))
            .min_by_key(|(_, binding)| binding.seen)
    }

    /// How long until the idlest of the sessions `counts` says count ends, at `now`.
    fn next_idle_end(
        &self,
        now: Instant,
        idle: Duration,
        counts: impl Fn(&Binding) -> bool,
    ) -> Option<Duration> {
        self.idlest(counts)
            .map(|(_, binding)| (binding.seen + idle).saturating_duration_since(now))
    }

    fn gauges(&self) {
        metrics::gauge!(OPEN_SESSIONS).set(self.bound.len() as f64);
        metrics::gauge!(SUBSCRIPTIONS).set(self.subscriptions() as f64);
    }

    /// The sessions to end at `now`: those idle for `idle`, those whose worker stopped, and
    /// workers bound to nobody for [`UNBOUND`].
    fn gone(
        &self,
        workers: &HashMap<SessionId, Worker>,
        now: Instant,
        idle: Duration,
    ) -> Vec<SessionId> {
        let ended = self.bound.iter().filter(|(id, binding)| {
            !workers.contains_key(*id)
                || (binding.active == 0 && now.duration_since(binding.seen) >= idle)
        });
        let unbound = workers.iter().filter(|(id, worker)| {
            !self.bound.contains_key(*id) && now.duration_since(worker.made) >= UNBOUND
        });
        ended
            .map(|(id, _)| id.clone())
            .chain(unbound.map(|(id, _)| id.clone()))
            .collect()
    }
}

/// A session's identity, use and subscriptions.
struct Binding {
    principal: Principal,
    /// When a request last came or ended.
    seen: Instant,
    /// Requests in flight and streams open.
    active: usize,
    /// A refused request made the watcher look again at it, and it has not finished.
    checking: bool,
    watches: HashMap<String, Watch>,
}

/// A session being opened, counted against the bounds. Dropped without opening, it no
/// longer counts.
pub(crate) struct Opening {
    sessions: Arc<Sessions>,
    user: Uuid,
    person: Uuid,
}

impl Opening {
    /// The session `id`, which the transport answered `initialize` with, bound to
    /// `principal`.
    pub(crate) fn open(self, id: &str, principal: Principal) {
        let live = self.sessions.workers().get(id).map(|_| SessionId::from(id));
        let mut state = self.sessions.state();
        if let Some(id) = live {
            info!(user_id = %principal.user.id, "MCP session opened");
            state.bound.insert(
                id,
                Binding {
                    principal,
                    seen: Instant::now(),
                    active: 0,
                    checking: false,
                    watches: HashMap::new(),
                },
            );
            state.gauges();
        }
    }
}

impl Drop for Opening {
    fn drop(&mut self) {
        let mut state = self.sessions.state();
        release(&mut state.opening, self.user);
        release(&mut state.opening_people, self.person);
    }
}

/// A request on a session, or a stream of it, in use until dropped.
pub(crate) struct Activity {
    sessions: Arc<Sessions>,
    id: SessionId,
}

impl Drop for Activity {
    fn drop(&mut self) {
        if let Some(binding) = self.sessions.state().bound.get_mut(&self.id) {
            binding.active = binding.active.saturating_sub(1);
            binding.seen = Instant::now();
        }
    }
}

impl Sessions {
    /// The sessions on `services`, with the watcher that ends them, which stops with `stop`.
    pub(crate) fn start(
        services: Services,
        heard: mpsc::UnboundedReceiver<AccessChange>,
        stop: CancellationToken,
    ) -> Arc<Self> {
        describe();
        let mut config = SessionConfig::default();
        // Sessions end as idle here, where a stream open counts as use.
        config.keep_alive = None;
        let sessions = Arc::new(Self {
            services,
            config,
            workers: Mutex::new(HashMap::new()),
            state: Mutex::new(State::default()),
            tasks: TaskTracker::new(),
            watching: stop.child_token(),
        });
        sessions.tasks.spawn(watch(
            Arc::downgrade(&sessions),
            heard,
            sessions.watching.clone(),
        ));
        sessions
    }

    fn workers(&self) -> MutexGuard<'_, HashMap<SessionId, Worker>> {
        self.workers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn handle(&self, id: &SessionId) -> Result<LocalSessionHandle, SessionFailure> {
        self.workers()
            .get(id)
            .map(|worker| worker.handle.clone())
            .ok_or(SessionFailure::NotFound)
    }

    pub(crate) fn count(&self) -> usize {
        self.state().bound.len()
    }

    pub(crate) fn subscriptions(&self) -> usize {
        self.state().subscriptions()
    }

    /// How many sessions have a request in flight or a stream open.
    #[cfg(test)]
    pub(crate) fn in_use(&self) -> usize {
        self.state()
            .bound
            .values()
            .filter(|binding| binding.active > 0)
            .count()
    }

    /// How many of the sessions' tasks run.
    #[cfg(test)]
    pub(crate) fn tasks_running(&self) -> usize {
        self.tasks.len()
    }

    /// Counts a session `principal` is opening, if the bounds leave room for it. At the
    /// identity's own bound, its idlest session ends to make room; see the module docs.
    pub(crate) fn reserve(
        self: &Arc<Self>,
        principal: &Principal,
        limits: Limits,
    ) -> Result<Opening, TooMany> {
        let (user, person) = (principal.user.id, person_of(principal));
        let now = Instant::now();
        let mut state = self.state();
        while state.of(user) >= limits.per_identity {
            let own = |binding: &Binding| binding.principal.user.id == user;
            let Some(id) = state.idlest(own).map(|(id, _)| id.clone()) else {
                return Err(TooMany::Identity(limits.per_identity, None));
            };
            state.bound.remove(&id);
            state.gauges();
            info!(user_id = %user, "MCP session ended to make room for a new one of its identity");
            let sessions = Arc::clone(self);
            self.tasks.spawn(async move { sessions.end(&id).await });
        }
        if state.bound.len() + state.opening() >= limits.overall {
            let wait = state.next_idle_end(now, limits.idle, |_| true);
            return Err(TooMany::Overall(limits.overall, wait));
        }
        if state.of_person(person) >= limits.per_person {
            let wait = state.next_idle_end(now, limits.idle, |binding| {
                person_of(&binding.principal) == person
            });
            return Err(TooMany::Person(limits.per_person, wait));
        }
        *state.opening.entry(user).or_default() += 1;
        *state.opening_people.entry(person).or_default() += 1;
        Ok(Opening {
            sessions: Arc::clone(self),
            user,
            person,
        })
    }

    /// A request on session `id`, if it is `principal`'s, whose principal it holds from now
    /// on.
    pub(crate) fn enter(self: &Arc<Self>, id: &str, principal: &Principal) -> Option<Activity> {
        let mut state = self.state();
        let (id, binding) = state.bound.get_key_value(id)?;
        let same = binding.principal.user.id == principal.user.id
            && binding.principal.workspace() == principal.workspace();
        if !same {
            return None;
        }
        let id = id.clone();
        let binding = state.bound.get_mut(&id)?;
        binding.principal = principal.clone();
        binding.active += 1;
        binding.seen = Instant::now();
        Some(Activity {
            sessions: Arc::clone(self),
            id,
        })
    }

    /// Looks again at session `id`, if there is one, as the gate refused a request on it:
    /// its token may have expired, or its identity lost the workspace. One look at a time.
    pub(crate) fn look_again_soon(self: &Arc<Self>, id: &str) {
        let principal = {
            let mut state = self.state();
            let Some(binding) = state.bound.get_mut(id).filter(|binding| !binding.checking) else {
                return;
            };
            binding.checking = true;
            binding.principal.clone()
        };
        let sessions = Arc::clone(self);
        let id = SessionId::from(id);
        self.tasks.spawn(async move {
            sessions.look_again(id.clone(), principal).await;
            if let Some(binding) = sessions.state().bound.get_mut(&id) {
                binding.checking = false;
            }
        });
    }

    /// Adds a subscription to session `id`, in place of one to the same resource.
    pub(crate) fn subscribe(&self, id: &str, watch: Watch) -> Result<(), ToolError> {
        let mut state = self.state();
        let binding = state
            .bound
            .get_mut(id)
            .ok_or_else(|| ToolError::invalid("not_found", "the session ended: initialize anew"))?;
        let uri = watch.resource.uri();
        if binding.watches.len() >= MAX_WATCHES && !binding.watches.contains_key(&uri) {
            return Err(ToolError::invalid(
                "too_many_subscriptions",
                format!("a session subscribes to at most {MAX_WATCHES} resources"),
            ));
        }
        binding.watches.insert(uri, watch);
        state.gauges();
        Ok(())
    }

    pub(crate) fn unsubscribe(&self, id: &str, resource: &resources::Resource) {
        let mut state = self.state();
        if let Some(binding) = state.bound.get_mut(id) {
            binding.watches.remove(&resource.uri());
        }
        state.gauges();
    }

    /// Ends a session: its worker, its streams and its subscriptions.
    async fn end(&self, id: &SessionId) {
        let worker = self.workers().remove(id);
        let binding = self.state().bound.remove(id);
        if let Some(worker) = worker {
            // A worker that stopped already has nothing more to close.
            let _ = worker.handle.close().await;
        }
        if binding.is_some() {
            info!("MCP session ended");
        }
        self.state().gauges();
    }

    /// Ends every session, as the server stops: stops the watcher, ends each session, and
    /// waits for the sessions' tasks to end.
    pub(crate) async fn close_all(&self) {
        self.watching.cancel();
        let ids: Vec<SessionId> = self.workers().keys().cloned().collect();
        for id in ids {
            self.end(&id).await;
        }
        self.tasks.close();
        self.tasks.wait().await;
    }

    /// Ends the sessions idle for `idle` and the workers bound to nobody, and forgets those
    /// whose worker stopped.
    async fn sweep(&self, idle: Duration) {
        let gone = {
            let workers = self.workers();
            self.state().gone(&workers, Instant::now(), idle)
        };
        for id in gone {
            self.end(&id).await;
        }
    }

    /// Looks again at the sessions whose identity in their workspace `concerned` says to.
    async fn recheck(&self, concerned: impl Fn(Uuid, &Principal) -> bool) {
        let sessions: Vec<(SessionId, Principal)> = self
            .state()
            .bound
            .iter()
            .filter(|(_, binding)| {
                binding
                    .principal
                    .workspace()
                    .is_some_and(|workspace| concerned(workspace, &binding.principal))
            })
            .map(|(id, binding)| (id.clone(), binding.principal.clone()))
            .collect();
        futures_util::stream::iter(sessions)
            .for_each_concurrent(RECHECKS_AT_ONCE, |(id, principal)| async move {
                self.look_again(id, principal).await;
            })
            .await;
    }

    /// Ends a session whose token stopped working or whose identity left its workspace,
    /// and drops the subscriptions to what it no longer sees.
    async fn look_again(&self, id: SessionId, principal: Principal) {
        let s = &self.services;
        let fresh = match s.auth.refresh(&principal).await {
            Ok(fresh) => fresh,
            Err(AuthError::Unauthenticated) => {
                info!(user_id = %principal.user.id, "MCP session's token stopped working");
                self.end(&id).await;
                return;
            }
            Err(error) => {
                warn!(%error, "cannot look again at an MCP session; it stays for now");
                return;
            }
        };
        let Some(workspace) = fresh.workspace() else {
            self.end(&id).await;
            return;
        };
        let settings = s.settings.current();
        if fresh.user.kind.is_human() && !IdentityMode::User.among(settings.get(keys::IDENTITIES)) {
            info!(user_id = %fresh.user.id, "MCP clients may no longer act as people");
            self.end(&id).await;
            return;
        }
        if let Err(error) = s.workspaces.admits(&fresh, workspace).await {
            if crate::http::ApiError::from(error)
                .status()
                .is_server_error()
            {
                warn!("cannot look again at an MCP session's workspace; it stays for now");
            } else {
                info!(user_id = %fresh.user.id, "MCP session's identity left its workspace");
                self.end(&id).await;
            }
            return;
        }
        let watched: Vec<(String, resources::Resource)> = {
            let mut state = self.state();
            let Some(binding) = state.bound.get_mut(&id) else {
                return;
            };
            binding.principal = fresh.clone();
            binding
                .watches
                .iter()
                .map(|(uri, watch)| (uri.clone(), watch.resource))
                .collect()
        };
        for (uri, resource) in watched {
            match resources::check(s, &fresh, &resource).await {
                Ok(()) => {}
                Err(refused) if refused.expected => {
                    let mut state = self.state();
                    if let Some(binding) = state.bound.get_mut(&id) {
                        binding.watches.remove(&uri);
                    }
                    state.gauges();
                }
                Err(_) => warn!("cannot look again at a subscription; it stays for now"),
            }
        }
    }
}

fn describe() {
    metrics::describe_gauge!(OPEN_SESSIONS, Unit::Count, "MCP sessions open.");
    metrics::describe_gauge!(
        SUBSCRIPTIONS,
        Unit::Count,
        "Resources MCP sessions subscribe to."
    );
}

/// A timer that ticks every `period`, the first time a `period` from now, and after a tick
/// it missed a `period` from when it ticked.
fn every(period: Duration) -> Interval {
    let mut timer = interval_at(Instant::now() + period, period);
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    timer
}

/// Ends idle sessions, and those whose access an access change ended, until `stop`.
async fn watch(
    sessions: Weak<Sessions>,
    mut heard: mpsc::UnboundedReceiver<AccessChange>,
    stop: CancellationToken,
) {
    let mut revalidate = every(REVALIDATE);
    // Its own timer, which access changes coming between its ticks do not put off.
    let mut sweep: Option<(Duration, Interval)> = None;
    loop {
        let Some(idle) = sessions.upgrade().map(|sessions| {
            *sessions
                .services
                .settings
                .current()
                .get(keys::SESSION_IDLE_TIMEOUT)
        }) else {
            return;
        };
        // Looked at often enough that a session ends within a quarter of its idle time of
        // when it should, and at least every second, so a change of the setting holds soon.
        let tick = (idle / 4).clamp(Duration::from_millis(100), Duration::from_secs(1));
        if sweep.as_ref().is_none_or(|(at, _)| *at != tick) {
            sweep = Some((tick, every(tick)));
        }
        let Some((_, timer)) = sweep.as_mut() else {
            return;
        };
        tokio::select! {
            () = stop.cancelled() => return,
            change = heard.recv() => {
                let Some(change) = change else { return };
                let mut changes = vec![change];
                while let Ok(change) = heard.try_recv() {
                    changes.push(change);
                }
                let Some(sessions) = sessions.upgrade() else { return };
                // Agents switched on or off goes with the setting of whom MCP clients act
                // as, which concerns people too.
                let everyone = changes.contains(&AccessChange::Agents);
                sessions
                    .recheck(|workspace, principal| {
                        everyone
                            || changes.iter().any(|change| change.concerns(workspace, principal))
                    })
                    .await;
            }
            _ = revalidate.tick() => {
                let Some(sessions) = sessions.upgrade() else { return };
                sessions.recheck(|_, _| true).await;
            }
            _ = timer.tick() => {
                let Some(sessions) = sessions.upgrade() else { return };
                sessions.sweep(idle).await;
            }
        }
    }
}

impl SessionManager for Sessions {
    type Error = SessionFailure;
    type Transport = WorkerTransport<LocalSessionWorker>;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        let id = session_id();
        let (handle, worker) = create_local_session(id.clone(), self.config.clone());
        let made = Instant::now();
        self.workers().insert(id.clone(), Worker { handle, made });
        Ok((id, WorkerTransport::spawn(worker)))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        Ok(self.handle(id)?.initialize(message).await?)
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        Ok(self.workers().contains_key(id))
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        self.end(id).await;
        Ok(())
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let handle = self.handle(id)?;
        let receiver = handle.establish_request_wise_channel().await?;
        handle
            .push_message(message, receiver.http_request_id)
            .await?;
        Ok(ReceiverStream::new(receiver.inner))
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        Ok(self.handle(id)?.push_message(message, None).await?)
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let receiver = self.handle(id)?.establish_common_channel().await?;
        Ok(ReceiverStream::new(receiver.inner))
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let receiver = self.handle(id)?.resume(last_event_id.parse()?).await?;
        Ok(ReceiverStream::new(receiver.inner))
    }
}

#[cfg(test)]
mod tests {
    use neon_domain::auth::{Credential, Role, User, UserKind};
    use time::OffsetDateTime;

    use super::*;

    fn principal() -> Principal {
        let at = OffsetDateTime::UNIX_EPOCH;
        Principal {
            user: User {
                id: Uuid::from_u128(1),
                kind: UserKind::Human,
                email: None,
                display_name: "Ada".into(),
                role: Role::User,
                hue_override: None,
                email_verified_at: None,
                disabled_at: None,
                created_at: at,
                updated_at: at,
                workspace_id: None,
                owner_id: None,
            },
            credential: Credential::Token {
                id: Uuid::from_u128(2),
                name: "mcp".into(),
                scopes: Vec::new(),
                workspace_id: Some(Uuid::from_u128(3)),
            },
        }
    }

    fn worker(id: &str, made: Instant) -> (SessionId, Worker) {
        let (handle, _) = create_local_session(id, SessionConfig::default());
        (SessionId::from(id), Worker { handle, made })
    }

    fn binding(seen: Instant, active: usize) -> Binding {
        Binding {
            principal: principal(),
            seen,
            active,
            checking: false,
            watches: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn the_sweep_ends_idle_sessions_stopped_workers_and_workers_nobody_bound() {
        let idle = Duration::from_secs(30);
        let start = Instant::now();
        let now = start + UNBOUND + idle;
        let workers: HashMap<SessionId, Worker> = [
            worker("busy", start),
            worker("idle", start),
            worker("listening", start),
            worker("lost", start),
            worker("opening", now - UNBOUND / 2),
        ]
        .into_iter()
        .collect();
        let mut state = State::default();
        for (id, seen, active) in [
            ("busy", now - idle / 2, 0),
            ("idle", now - idle, 0),
            ("listening", start, 1),
            ("stopped", now, 0),
        ] {
            state
                .bound
                .insert(SessionId::from(id), binding(seen, active));
        }

        let mut gone: Vec<String> = state
            .gone(&workers, now, idle)
            .iter()
            .map(ToString::to_string)
            .collect();
        gone.sort();
        // Busy was seen within the idle time, listening holds a stream open, and opening
        // waits for its answer still.
        assert_eq!(gone, ["idle", "lost", "stopped"]);
    }
}
