//! Workspaces and what is in them: members and their roles, invitations and invite links,
//! teamspaces and their members, the limits instance admins set, and the hourly cleanup.
//!
//! Every use case asks [`authorize`] before it acts, so the rules live in one place, the
//! domain. A use case that changes who is in a workspace locks the workspace's row first,
//! in the transaction that makes the change: changes then run one at a time, and the last
//! owner and the member limit hold however many arrive at once. Changes to a teamspace's
//! members lock the teamspace the same way.
//!
//! Every public use case runs in a span named `workspaces <operation>` and is measured as
//! RED metrics of [`SUBSYSTEM`]. An expected error, such as a refusal of the rules, is
//! recorded on the span without failing it and counts as `ok`; only failures of the
//! server count as `error`.

mod admin;
mod cleanup;
mod error;
mod invitations;
mod lifecycle;
mod links;
mod members;
mod teamspaces;

use std::future::Future;
use std::sync::Arc;

use neon_domain::auth::Principal;
use neon_domain::settings::Snapshot;
use neon_domain::workspaces::{
    Action, Actor, LimitOverrides, LimitValues, Limits, Resource, Workspace, WorkspaceCreation,
    WorkspaceRole, authorize,
};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::workspaces as repo;
use sqlx::{PgConnection, PgPool};
use tracing::{Instrument as _, info_span, warn};
use uuid::Uuid;

use crate::clock::Clock;
use crate::crypto::Envelope;
use crate::ratelimit::{Bucket, MemoryLimiter, RateLimiter};
use crate::settings::{SettingsService, workspaces as keys};

pub use admin::{AdminWorkspace, AdminWorkspacePage, LimitsUpdate, Usage};
pub use cleanup::{Cleanup, CleanupReport};
pub use error::WorkspaceError;
pub use invitations::{ReceivedInvitation, SentInvitation};
pub use lifecycle::{
    DEFAULT_TEAMSPACE_NAME, Joined, NewWorkspace, WorkspaceDetails, WorkspaceUpdate,
};
pub use links::{CreatedLink, LinkSpec};
pub use teamspaces::{NewTeamspace, TeamspaceEntry, TeamspaceList, TeamspaceUpdate};

/// RED subsystem of the workspace use cases.
pub const SUBSYSTEM: &str = "workspaces";

/// The workspace use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct WorkspaceService {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    settings: Arc<SettingsService>,
    limiter: Arc<dyn RateLimiter>,
    envelope: Envelope,
    clock: Clock,
    red: Red,
}

/// Puts a [`WorkspaceService`] together. Without being told otherwise it limits rates in
/// memory, seals no invite link and reads the system clock.
pub struct WorkspaceServiceBuilder {
    pool: PgPool,
    settings: Arc<SettingsService>,
    limiter: Option<Arc<dyn RateLimiter>>,
    envelope: Envelope,
    clock: Clock,
}

impl WorkspaceServiceBuilder {
    pub fn limiter(mut self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    /// Seals invite links' tokens, so they can be copied again.
    pub fn envelope(mut self, envelope: Envelope) -> Self {
        self.envelope = envelope;
        self
    }

    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Call it after telemetry is installed, as it creates the RED metrics.
    pub fn build(self) -> WorkspaceService {
        WorkspaceService {
            inner: Arc::new(Inner {
                pool: self.pool,
                settings: self.settings,
                limiter: self
                    .limiter
                    .unwrap_or_else(|| Arc::new(MemoryLimiter::new())),
                envelope: self.envelope,
                clock: self.clock,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

impl WorkspaceService {
    pub fn builder(pool: PgPool, settings: Arc<SettingsService>) -> WorkspaceServiceBuilder {
        WorkspaceServiceBuilder {
            pool,
            settings,
            limiter: None,
            envelope: Envelope::default(),
            clock: Clock::system(),
        }
    }

    fn settings(&self) -> Arc<Snapshot> {
        self.inner.settings.current()
    }

    fn now(&self) -> time::OffsetDateTime {
        self.inner.clock.now()
    }

    fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    fn creation(&self) -> WorkspaceCreation {
        WorkspaceCreation::parse(self.settings().get(keys::CREATION))
            .unwrap_or(WorkspaceCreation::DEFAULT)
    }

    /// The instance's limits, for workspaces that set none of their own.
    fn default_limits(&self) -> LimitValues {
        let settings = self.settings();
        LimitValues {
            members: *settings.get(keys::LIMIT_MEMBERS),
            storage_bytes: *settings.get(keys::LIMIT_STORAGE_BYTES),
            agents: *settings.get(keys::LIMIT_AGENTS),
        }
    }

    /// The limits that hold for a workspace now.
    async fn limits_of(
        &self,
        conn: &mut PgConnection,
        workspace_id: Uuid,
    ) -> Result<Limits, WorkspaceError> {
        let overrides = repo::limits(&mut *conn, workspace_id)
            .await?
            .unwrap_or_default();
        Ok(self.limits(overrides))
    }

    fn limits(&self, overrides: LimitOverrides) -> Limits {
        Limits::resolve(overrides, self.default_limits())
    }

    /// Refuses one more member of a workspace at its limit. Run it after locking the
    /// workspace, so the count is the one the change sees.
    async fn check_room(
        &self,
        conn: &mut PgConnection,
        workspace_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        let limits = self.limits_of(&mut *conn, workspace_id).await?;
        let members = repo::count_members(&mut *conn, workspace_id).await?;
        match limits.members {
            Some(limit) if !limits.admits_member(members) => {
                Err(WorkspaceError::MemberLimit { limit })
            }
            _ => Ok(()),
        }
    }

    async fn limit(&self, bucket: Bucket, key: &str) -> Result<(), WorkspaceError> {
        self.inner
            .limiter
            .check(bucket, key)
            .await
            .map_err(|limited| {
                warn!(bucket = bucket.prefix(), "rate limited");
                WorkspaceError::RateLimited {
                    retry_after: limited.retry_after,
                }
            })
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, WorkspaceError>>,
    ) -> Result<T, WorkspaceError> {
        let span = info_span!(
            "workspaces",
            otel.name = %format!("workspaces {operation}"),
            workspaces.operation = operation,
        );
        let guard = self.inner.red.start(operation);
        let result = work.instrument(span.clone()).await;
        match &result {
            Ok(_) => guard.ok(),
            Err(error) if error.is_expected() => {
                mark_expected(&span, error.code());
                guard.ok();
            }
            Err(error) => {
                record_unexpected(&span, error);
                guard.err();
            }
        }
        result
    }
}

/// The principal as the rules see them in a workspace where their role is `role`.
fn actor(principal: &Principal, role: Option<WorkspaceRole>) -> Actor {
    Actor {
        instance_admin: principal.is_admin(),
        role,
    }
}

/// A workspace and the principal's role in it, read on `conn`, which may hold its lock.
struct Membership {
    workspace: Workspace,
    role: Option<WorkspaceRole>,
}

impl Membership {
    fn resource(&self) -> Resource {
        Resource::Workspace {
            members_can_create_teamspaces: self.workspace.members_can_create_teamspaces,
        }
    }

    /// Asks the rules about an action on the workspace as a whole.
    fn check(&self, principal: &Principal, action: Action) -> Result<(), WorkspaceError> {
        authorize(actor(principal, self.role), action, self.resource())
            .map_err(|denied| WorkspaceError::denied(denied, NO_WORKSPACE))
    }

    /// The principal's role, once the rules let them act in the workspace.
    fn role(&self) -> Result<WorkspaceRole, WorkspaceError> {
        self.role.ok_or(WorkspaceError::NotFound(NO_WORKSPACE))
    }
}

const NO_WORKSPACE: &str = "no such workspace";

/// Reads a workspace and the principal's role in it, without a lock.
async fn membership(
    conn: &mut PgConnection,
    principal: &Principal,
    id: Uuid,
) -> Result<Membership, WorkspaceError> {
    let workspace = repo::get(&mut *conn, id)
        .await?
        .ok_or(WorkspaceError::NotFound(NO_WORKSPACE))?;
    let role = repo::member_role(&mut *conn, id, principal.user.id).await?;
    Ok(Membership { workspace, role })
}

/// Locks a workspace, then reads the principal's role in it as the lock leaves it.
async fn locked_membership(
    conn: &mut PgConnection,
    principal: &Principal,
    id: Uuid,
) -> Result<Membership, WorkspaceError> {
    let workspace = repo::lock(&mut *conn, id)
        .await?
        .ok_or(WorkspaceError::NotFound(NO_WORKSPACE))?;
    let role = repo::member_role(&mut *conn, id, principal.user.id).await?;
    Ok(Membership { workspace, role })
}

fn require_session(principal: &Principal, what: &'static str) -> Result<(), WorkspaceError> {
    if principal.via_session() {
        Ok(())
    } else {
        Err(WorkspaceError::Forbidden(what))
    }
}
