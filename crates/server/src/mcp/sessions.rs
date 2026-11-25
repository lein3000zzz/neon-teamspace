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
//! - **Bounds.** `mcp.max_sessions` sessions at once, and `mcp.max_sessions_per_identity`
//!   of one identity, those being opened included. One more is refused.
//! - **Idle sessions** end: once a session has no request in flight and no stream open for
//!   `mcp.session_idle_timeout`, it ends with its subscriptions. An open stream counts as
//!   use, as its client listens; one whose connection broke closes as the transport's pings
//!   fail.
//! - **Access.** Every access change that may concern a session's identity, a token revoked,
//!   an account disabled, a member removed, an agent's owner leaving, agents switched off,
//!   makes the watcher look again at once: the session ends when its token no longer works,
//!   its identity is no longer in the workspace, or MCP clients may no longer act as it, and
//!   each subscription whose resource it no longer sees drops.
//!   Every [`REVALIDATE`] it looks so at every session, for what no change tells of, such
//!   as a token expiring.

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
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use super::Services;
use super::resources;
use super::tools::ToolError;
use super::watch::Watch;

/// How often every session is looked at again, as an access change would make it.
pub(crate) const REVALIDATE: Duration = Duration::from_secs(300);

/// How many sessions the watcher looks at again at once.
const RECHECKS_AT_ONCE: usize = 8;

/// The most resources one session subscribes to.
pub(crate) const MAX_WATCHES: usize = 100;

/// The gauge of the sessions open.
const OPEN_SESSIONS: &str = "neon_mcp_sessions";
/// The gauge of the subscriptions held.
const SUBSCRIPTIONS: &str = "neon_mcp_subscriptions";

/// How many sessions there may be.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) overall: usize,
    pub(crate) per_identity: usize,
}

/// Why a session cannot open.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum TooMany {
    #[error("this server holds {0} MCP sessions, the most it does: try again once one ends")]
    Overall(usize),
    #[error(
        "this identity holds {0} MCP sessions, the most it may: end one with DELETE /mcp, or \
         wait for one to idle out"
    )]
    Identity(usize),
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
    workers: Mutex<HashMap<SessionId, LocalSessionHandle>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    bound: HashMap<SessionId, Binding>,
    /// Sessions being opened, by identity: their `initialize` is on its way.
    opening: HashMap<Uuid, usize>,
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

    fn subscriptions(&self) -> usize {
        self.bound
            .values()
            .map(|binding| binding.watches.len())
            .sum()
    }

    fn gauges(&self) {
        metrics::gauge!(OPEN_SESSIONS).set(self.bound.len() as f64);
        metrics::gauge!(SUBSCRIPTIONS).set(self.subscriptions() as f64);
    }
}

/// A session's identity, use and subscriptions.
struct Binding {
    principal: Principal,
    /// When a request last came or ended.
    seen: Instant,
    /// Requests in flight and streams open.
    active: usize,
    watches: HashMap<String, Watch>,
}

/// A session being opened, counted against the bounds. Dropped without opening, it no
/// longer counts.
pub(crate) struct Opening {
    sessions: Arc<Sessions>,
    user: Uuid,
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
        if let Some(count) = state.opening.get_mut(&self.user) {
            *count -= 1;
            if *count == 0 {
                state.opening.remove(&self.user);
            }
        }
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
        });
        tokio::spawn(watch(Arc::downgrade(&sessions), heard, stop));
        sessions
    }

    fn workers(&self) -> MutexGuard<'_, HashMap<SessionId, LocalSessionHandle>> {
        self.workers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn handle(&self, id: &SessionId) -> Result<LocalSessionHandle, SessionFailure> {
        self.workers()
            .get(id)
            .cloned()
            .ok_or(SessionFailure::NotFound)
    }

    pub(crate) fn count(&self) -> usize {
        self.state().bound.len()
    }

    pub(crate) fn subscriptions(&self) -> usize {
        self.state().subscriptions()
    }

    /// Counts a session `principal` is opening, if the bounds leave room for it.
    pub(crate) fn reserve(
        self: &Arc<Self>,
        principal: &Principal,
        limits: Limits,
    ) -> Result<Opening, TooMany> {
        let user = principal.user.id;
        let mut state = self.state();
        if state.bound.len() + state.opening() >= limits.overall {
            return Err(TooMany::Overall(limits.overall));
        }
        if state.of(user) >= limits.per_identity {
            return Err(TooMany::Identity(limits.per_identity));
        }
        *state.opening.entry(user).or_default() += 1;
        Ok(Opening {
            sessions: Arc::clone(self),
            user,
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
        let handle = self.workers().remove(id);
        let binding = self.state().bound.remove(id);
        if let Some(handle) = handle {
            // A worker that stopped already has nothing more to close.
            let _ = handle.close().await;
        }
        if binding.is_some() {
            info!("MCP session ended");
        }
        self.state().gauges();
    }

    /// Ends every session, as the server stops.
    pub(crate) async fn close_all(&self) {
        let ids: Vec<SessionId> = self.workers().keys().cloned().collect();
        for id in ids {
            self.end(&id).await;
        }
    }

    /// Ends the sessions idle for `idle`, and forgets those whose worker stopped.
    async fn sweep(&self, idle: Duration) {
        let now = Instant::now();
        let gone: Vec<SessionId> = {
            let workers = self.workers();
            let state = self.state();
            state
                .bound
                .iter()
                .filter(|(id, binding)| {
                    !workers.contains_key(*id)
                        || (binding.active == 0 && now.duration_since(binding.seen) >= idle)
                })
                .map(|(id, _)| id.clone())
                .collect()
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

/// Ends idle sessions, and those whose access an access change ended, until `stop`.
async fn watch(
    sessions: Weak<Sessions>,
    mut heard: mpsc::UnboundedReceiver<AccessChange>,
    stop: CancellationToken,
) {
    let mut revalidate = interval_at(Instant::now() + REVALIDATE, REVALIDATE);
    revalidate.set_missed_tick_behavior(MissedTickBehavior::Delay);
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
            () = sleep(tick) => {
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
        self.workers().insert(id.clone(), handle);
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
