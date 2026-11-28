//! Agents and integrations: making agents, listing them and the teamspaces each is in,
//! renaming and recolouring them, handing them to another person, deleting them, and their
//! API tokens; and making the integrations of incoming webhooks.
//!
//! An agent is made by a person of the workspace, its owner, as the workspace's switches
//! allow, and within its agent limit, which agents and integrations count against together.
//! It is a member of the workspace from then on, in its default teamspace like everyone but
//! guests, and joins others as people do. Handing it to another person revokes its tokens
//! and the OAuth grants that act as it: they were made by, and acted for, its former owner;
//! turns off its webhooks whose URL its former owner set, unless they may still register
//! them; and gives an integration's incoming webhook a URL and secret nobody was shown.
//! Deleting it ends its membership and revokes its tokens and grants, and an integration's
//! incoming webhook goes with it; its identity stays, for what it made. Once a change that ends tokens or access commits, the
//! [`AccessChanges`](crate::pages::AccessChanges) port hears of it, and its open connections
//! close.
//!
//! An integration is made with its incoming webhook, by the webhook's use case, on its
//! transaction, within the same agent limit. It joins no teamspace, not even the default
//! one: what it may edit is what its webhook shares with it, its target, so its access is
//! the least that target needs, and bounded by its owner's as every agent's is.
//!
//! Every public use case runs in a span named `agents <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`]: a refusal is `ok`, only failures of the server are `error`. No
//! token reaches a log line or a span.

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;

use neon_domain::agents::{Agent, AgentStatus, check_agent_name};
use neon_domain::auth::token::token_scopes;
use neon_domain::auth::{ApiToken, Principal, TokenHolder, UserKind};
use neon_domain::hue::check_hue;
use neon_domain::settings::Secret;
use neon_domain::workspaces::{Action, Resource, Teamspace, TeamspaceRole};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::agents::{self as repo, New, Stored};
use neon_store::{api_tokens, incoming_webhooks, oauth, teamspaces, users};
use sqlx::PgConnection;
use time::OffsetDateTime;
use tracing::{Instrument as _, info, info_span};
use uuid::Uuid;

use super::members::target;
use super::teamspaces::placed_teamspaces;
use super::{
    AccessChange, Membership, NO_WORKSPACE, WorkspaceError, WorkspaceService, locked_membership,
    membership,
};
use crate::Read;
use crate::auth::{TokenSpec, mint};
use crate::webhooks;

/// RED subsystem of the agent use cases.
pub const SUBSYSTEM: &str = "agents";

const NO_AGENT: &str = "no such agent";
const NO_TOKEN: &str = "no such token";

/// What a new agent starts with.
#[derive(Debug, Clone, Default)]
pub struct NewAgent {
    pub name: String,
    /// The hue to pin, 0 to 359; unset, it is drawn in the hue its id hashes to.
    pub hue: Option<i64>,
}

/// What a change to an agent sets. Each part left out stays as it is; for the hue,
/// `Some(None)` unpins it.
#[derive(Debug, Clone, Default)]
pub struct AgentUpdate {
    pub name: Option<String>,
    pub hue: Option<Option<i64>>,
}

/// A teamspace an agent is in, and whether the caller may take the agent out of it.
#[derive(Debug, Clone)]
pub struct AgentTeamspace {
    pub teamspace: Teamspace,
    pub removable: bool,
}

/// The agent use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct AgentService {
    inner: Arc<Inner>,
}

struct Inner {
    workspaces: WorkspaceService,
    red: Red,
}

impl AgentService {
    /// The agents of the workspaces `workspaces` serves, with its settings, limits, clock
    /// and listener of access changes. Call it after telemetry is installed, as it creates
    /// the RED metrics.
    pub fn new(workspaces: WorkspaceService) -> Self {
        Self {
            inner: Arc::new(Inner {
                workspaces,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }

    fn workspaces(&self) -> &WorkspaceService {
        &self.inner.workspaces
    }

    fn now(&self) -> OffsetDateTime {
        self.workspaces().now()
    }

    /// The workspace's agents and integrations that the principal may know of, deleted
    /// ones too, those that are not first: everyone's for its members, none for a guest.
    pub async fn list_agents(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Vec<Agent>, WorkspaceError> {
        self.observe("list_agents", async {
            let mut conn = self.workspaces().pool().acquire().await?;
            let membership = membership(&mut conn, principal, workspace_id).await?;
            membership.check(Action::ViewWorkspace)?;
            Ok(repo::list(&mut *conn, workspace_id)
                .await?
                .into_iter()
                .filter(|stored| check(principal, &membership, stored, Action::ViewAgent).is_ok())
                .map(|stored| self.agent_of(stored))
                .collect())
        })
        .await
    }

    /// One of the workspace's agents or integrations, deleted or not.
    pub async fn agent(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Agent, WorkspaceError> {
        self.observe("get_agent", async {
            let mut conn = self.workspaces().pool().acquire().await?;
            let (_, stored) = self
                .open(
                    &mut conn,
                    principal,
                    workspace_id,
                    id,
                    Action::ViewAgent,
                    Read::Plain,
                )
                .await?;
            Ok(self.agent_of(stored))
        })
        .await
    }

    /// The teamspaces an agent is in, of those whose members the principal may list, with
    /// whether they may take it out of each. Whoever may know of the agent asks; a teamspace
    /// they may not see, such as a private one they are not in, stays out.
    pub async fn agent_teamspaces(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Vec<AgentTeamspace>, WorkspaceError> {
        self.observe("agent_teamspaces", async {
            let mut conn = self.workspaces().pool().acquire().await?;
            let (membership, stored) = self
                .open(
                    &mut conn,
                    principal,
                    workspace_id,
                    id,
                    Action::ViewAgent,
                    Read::Plain,
                )
                .await?;
            let theirs: HashSet<Uuid> =
                teamspaces::list_with_membership(&mut *conn, workspace_id, stored.id)
                    .await?
                    .into_iter()
                    .filter_map(|(teamspace, role, _)| role.map(|_| teamspace.id))
                    .collect();
            Ok(
                placed_teamspaces(&mut conn, principal, &membership, workspace_id)
                    .await?
                    .into_iter()
                    .filter(|placed| {
                        theirs.contains(&placed.teamspace.id)
                            && placed.allows(&membership, Action::ListTeamspaceMembers)
                    })
                    .map(|placed| AgentTeamspace {
                        removable: placed.allows(&membership, Action::RemoveTeamspaceMember),
                        teamspace: placed.teamspace,
                    })
                    .collect(),
            )
        })
        .await
    }

    /// Makes an agent owned by the principal: while agents are on, on the instance and in
    /// the workspace, by whom the workspace lets make them, and within its agent limit. It
    /// joins the workspace as a member, and its default teamspace.
    pub async fn create_agent(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        new: NewAgent,
    ) -> Result<Agent, WorkspaceError> {
        self.observe("create_agent", async {
            let mut tx = self.workspaces().pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, workspace_id).await?;
            membership.check(Action::ViewWorkspace)?;
            let workspace = &membership.workspace;
            let agents = Resource::Agents {
                instance: self.workspaces().agents_on(),
                workspace: workspace.agents_enabled,
                creation: workspace.agent_creation,
            };
            membership
                .standing
                .authorize_same(Action::CreateAgent, agents)
                .map_err(|denied| WorkspaceError::denied(denied, NO_WORKSPACE))?;
            let name = check_agent_name(&new.name).map_err(WorkspaceError::invalid)?;
            let hue_override = new
                .hue
                .map(check_hue)
                .transpose()
                .map_err(WorkspaceError::invalid)?;
            // The workspace's lock keeps the count as it is until this one is in.
            let limits = self.workspaces().limits_of(&mut tx, workspace_id).await?;
            let count = repo::count_live(&mut *tx, workspace_id).await?;
            if let Some(limit) = limits.agents
                && !limits.admits_agent(count)
            {
                return Err(WorkspaceError::AgentLimit { limit });
            }
            let id = Uuid::now_v7();
            let now = self.now();
            repo::insert(
                &mut tx,
                &New {
                    id,
                    kind: UserKind::Agent,
                    workspace_id,
                    owner_id: principal.user.id,
                    name: &name,
                    hue_override,
                    at: now,
                },
            )
            .await?;
            teamspaces::join_default(&mut *tx, workspace_id, id, TeamspaceRole::Member, now)
                .await?;
            let stored = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_AGENT))?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                agent_id = %id,
                "agent created"
            );
            Ok(self.agent_of(stored))
        })
        .await
    }

    /// Renames an agent or integration, pins its hue, or both, for its owner.
    pub async fn update_agent(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        update: AgentUpdate,
    ) -> Result<Agent, WorkspaceError> {
        self.observe("update_agent", async {
            let name = update
                .name
                .map(|name| check_agent_name(&name))
                .transpose()
                .map_err(WorkspaceError::invalid)?;
            let hue = update
                .hue
                .map(|hue| hue.map(check_hue).transpose())
                .transpose()
                .map_err(WorkspaceError::invalid)?;
            let mut tx = self.workspaces().pool().begin().await?;
            let (_, stored) = self
                .open(
                    &mut tx,
                    principal,
                    workspace_id,
                    id,
                    Action::UpdateAgent,
                    Read::Locked,
                )
                .await?;
            live(&stored)?;
            if name.is_some() || hue.is_some() {
                repo::update(&mut *tx, id, name.as_deref(), hue, self.now()).await?;
            }
            let stored = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_AGENT))?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                agent_id = %id,
                "agent updated"
            );
            Ok(self.agent_of(stored))
        })
        .await
    }

    /// Hands an agent or integration to another person of the workspace, neither a guest
    /// nor disabled, for the workspace's owners and admins. It acts for them from then on,
    /// which resumes it when it was suspended; its tokens, made by its former owner, are
    /// revoked, and its webhooks whose URL they set turned off unless they may still
    /// register them. An integration's incoming webhook gets a new URL and signing secret
    /// that nobody is shown, as the old ones were shown to whoever made them.
    pub async fn transfer_agent(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        to: Uuid,
    ) -> Result<Agent, WorkspaceError> {
        self.observe("transfer_agent", async {
            let mut tx = self.workspaces().pool().begin().await?;
            let (_, stored) = self
                .open(
                    &mut tx,
                    principal,
                    workspace_id,
                    id,
                    Action::TransferAgent,
                    Read::Locked,
                )
                .await?;
            live(&stored)?;
            let newcomer = target(&mut tx, workspace_id, to).await?;
            if !newcomer.kind.is_human() {
                return Err(WorkspaceError::InvalidInput(
                    "an agent or integration goes to a person".to_owned(),
                ));
            }
            if !newcomer.role.joins_teamspaces() {
                return Err(WorkspaceError::InvalidInput(
                    "guests own no agents: make them a member first".to_owned(),
                ));
            }
            if users::by_id(&mut *tx, to)
                .await?
                .is_none_or(|person| person.disabled_at.is_some())
            {
                return Err(WorkspaceError::InvalidInput(
                    "their account is disabled: it would stay suspended with them".to_owned(),
                ));
            }
            if stored.owner.as_ref().is_some_and(|owner| owner.id == to) {
                return Ok(self.agent_of(stored));
            }
            repo::set_owner(&mut *tx, id, to, self.now()).await?;
            let revoked = api_tokens::delete_for_user(&mut *tx, id).await?
                + oauth::delete_grants_of_user(&mut *tx, id).await?;
            webhooks::lapse(&mut tx, workspace_id, id, self.now()).await?;
            if stored.kind == UserKind::Integration {
                webhooks::hand_over_incoming(&mut tx, self.workspaces().envelope(), id, self.now())
                    .await?;
            }
            let stored = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_AGENT))?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                agent_id = %id,
                owner_id = %to,
                tokens_revoked = revoked,
                "agent transferred"
            );
            self.ended(id);
            Ok(self.agent_of(stored))
        })
        .await
    }

    /// Deletes an agent or integration, for its owner and the workspace's owners and
    /// admins: it leaves the workspace and its tokens are revoked, an integration's incoming
    /// webhook is deleted, and what it made still names it. One deleted already stays so.
    pub async fn delete_agent(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("delete_agent", async {
            let mut tx = self.workspaces().pool().begin().await?;
            let (_, stored) = self
                .open(
                    &mut tx,
                    principal,
                    workspace_id,
                    id,
                    Action::DeleteAgent,
                    Read::Locked,
                )
                .await?;
            if stored.deleted_at.is_some() {
                return Ok(());
            }
            repo::mark_deleted(&mut tx, workspace_id, id, self.now()).await?;
            let revoked = api_tokens::delete_for_user(&mut *tx, id).await?
                + oauth::delete_grants_of_user(&mut *tx, id).await?;
            // An integration is its incoming webhook's: the webhook goes with it.
            let hook_deleted = stored.kind == UserKind::Integration
                && incoming_webhooks::delete_of_integration(&mut *tx, id).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                agent_id = %id,
                tokens_revoked = revoked,
                hook_deleted,
                "agent deleted"
            );
            self.ended(id);
            Ok(())
        })
        .await
    }

    /// An agent's tokens, without their secrets, oldest first: for its owner and the
    /// workspace's owners and admins.
    pub async fn agent_tokens(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Vec<ApiToken>, WorkspaceError> {
        self.observe("list_agent_tokens", async {
            let mut conn = self.workspaces().pool().acquire().await?;
            self.open(
                &mut conn,
                principal,
                workspace_id,
                id,
                Action::ListAgentTokens,
                Read::Plain,
            )
            .await?;
            Ok(api_tokens::list_for_user(&mut *conn, id).await?)
        })
        .await
    }

    /// Makes a token that acts as the agent, bound to its workspace, with read and write
    /// but never admin, for its owner, from a browser session, while the agent works. Its
    /// secret is returned once; only its hash is kept.
    pub async fn create_agent_token<S: AsRef<str> + Sync>(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        name: &str,
        expires_at: Option<OffsetDateTime>,
        scopes: &[S],
    ) -> Result<(Secret, ApiToken), WorkspaceError> {
        self.observe("create_agent_token", async {
            let scopes = token_scopes(scopes, TokenHolder::Agent).map_err(|error| {
                match error.refusal() {
                    Some(refusal) => WorkspaceError::Forbidden(refusal),
                    None => WorkspaceError::invalid(error),
                }
            })?;
            let mut tx = self.workspaces().pool().begin().await?;
            let (_, stored) = self
                .open(
                    &mut tx,
                    principal,
                    workspace_id,
                    id,
                    Action::CreateAgentToken,
                    Read::Locked,
                )
                .await?;
            match self.agent_of(stored).status {
                AgentStatus::Active => {}
                AgentStatus::Deleted => return Err(DELETED),
                AgentStatus::Suspended => {
                    return Err(WorkspaceError::Forbidden(
                        "this agent is suspended while its owner is out of the workspace, a \
                         guest there or disabled",
                    ));
                }
                AgentStatus::Disabled => {
                    return Err(WorkspaceError::Forbidden(
                        "agents are turned off: its tokens would not work",
                    ));
                }
            }
            let spec = TokenSpec {
                user_id: id,
                workspace_id: Some(workspace_id),
                name,
                scopes,
                expires_at,
            };
            let (secret, token) = mint(&mut *tx, spec, self.now())
                .await?
                .map_err(WorkspaceError::invalid)?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                agent_id = %id,
                token_id = %token.id,
                scopes = ?token.scopes,
                "agent token created"
            );
            Ok((secret, token))
        })
        .await
    }

    /// Revokes one of an agent's tokens, which stops working at once: for its owner and
    /// the workspace's owners and admins, from a browser session.
    pub async fn revoke_agent_token(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        token_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("revoke_agent_token", async {
            let mut conn = self.workspaces().pool().acquire().await?;
            self.open(
                &mut conn,
                principal,
                workspace_id,
                id,
                Action::RevokeAgentToken,
                Read::Plain,
            )
            .await?;
            if !api_tokens::delete(&mut *conn, id, token_id).await? {
                return Err(WorkspaceError::NotFound(NO_TOKEN));
            }
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                agent_id = %id,
                token_id = %token_id,
                "agent token revoked"
            );
            self.ended(id);
            Ok(())
        })
        .await
    }

    /// Makes an integration owned by the principal, for an incoming webhook they make, on the
    /// webhook's transaction: for those who manage the workspace's incoming webhooks, within
    /// its agent limit. It is a member of the workspace and of none of its teamspaces. The
    /// workspace stays locked until the transaction ends.
    pub(crate) async fn add_integration(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
        name: &str,
    ) -> Result<Uuid, WorkspaceError> {
        let membership = locked_membership(&mut *conn, principal, workspace_id).await?;
        membership.check(Action::ViewWorkspace)?;
        membership.check(Action::ManageIncomingWebhooks)?;
        let name = check_agent_name(name).map_err(WorkspaceError::invalid)?;
        let limits = self
            .workspaces()
            .limits_of(&mut *conn, workspace_id)
            .await?;
        let count = repo::count_live(&mut *conn, workspace_id).await?;
        if let Some(limit) = limits.agents
            && !limits.admits_agent(count)
        {
            return Err(WorkspaceError::AgentLimit { limit });
        }
        let id = Uuid::now_v7();
        repo::insert(
            &mut *conn,
            &New {
                id,
                kind: UserKind::Integration,
                workspace_id,
                owner_id: principal.user.id,
                name: &name,
                hue_override: None,
                at: self.now(),
            },
        )
        .await?;
        info!(
            user_id = %principal.user.id,
            workspace_id = %workspace_id,
            agent_id = %id,
            "integration created"
        );
        Ok(id)
    }

    /// Deletes an integration with its incoming webhook, on the webhook's transaction: it
    /// leaves the workspace, and so its shares. Its open connections, which it has none of,
    /// are told once the transaction commits by [`integration_ended`](Self::integration_ended).
    pub(crate) async fn remove_integration(
        &self,
        conn: &mut PgConnection,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), WorkspaceError> {
        if repo::lock(&mut *conn, workspace_id, id).await? {
            repo::mark_deleted(conn, workspace_id, id, self.now()).await?;
        }
        Ok(())
    }

    /// Tells the port that an integration's access ended, once its deletion committed.
    pub(crate) fn integration_ended(&self, id: Uuid) {
        self.ended(id);
    }

    /// Reads an agent or integration of a workspace, once the principal may know of the
    /// workspace, and asks the rules about `action` on it. With [`Read::Locked`], the
    /// workspace's row is locked, then the agent's, in the order every change takes them.
    async fn open(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        action: Action,
        read: Read,
    ) -> Result<(Membership, Stored), WorkspaceError> {
        let membership = match read {
            Read::Locked => locked_membership(&mut *conn, principal, workspace_id).await?,
            Read::Plain => membership(&mut *conn, principal, workspace_id).await?,
        };
        membership.check(Action::ViewWorkspace)?;
        if read == Read::Locked && !repo::lock(&mut *conn, workspace_id, id).await? {
            return Err(WorkspaceError::NotFound(NO_AGENT));
        }
        let stored = repo::get(&mut *conn, workspace_id, id)
            .await?
            .ok_or(WorkspaceError::NotFound(NO_AGENT))?;
        check(principal, &membership, &stored, action)?;
        Ok((membership, stored))
    }

    /// An agent as stored, with its status as the instance's switch says now.
    fn agent_of(&self, stored: Stored) -> Agent {
        Agent {
            status: stored
                .standing
                .status(stored.kind, self.workspaces().agents_on()),
            id: stored.id,
            kind: stored.kind,
            workspace_id: stored.workspace_id,
            name: stored.name,
            hue_override: stored.hue_override,
            owner: stored.owner,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
            deleted_at: stored.deleted_at,
        }
    }

    /// Tells the port that an agent's tokens ended, or whom it acts for changed, once the
    /// change committed: its open connections look again, and close.
    fn ended(&self, id: Uuid) {
        self.workspaces()
            .changed(AccessChange::Account { user_id: id });
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, WorkspaceError>>,
    ) -> Result<T, WorkspaceError> {
        let span = info_span!(
            "agents",
            otel.name = %format!("agents {operation}"),
            agents.operation = operation,
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

const DELETED: WorkspaceError = WorkspaceError::Forbidden("this agent is deleted");

/// Refuses a change to one deleted already.
fn live(stored: &Stored) -> Result<(), WorkspaceError> {
    match stored.deleted_at {
        Some(_) => Err(DELETED),
        None => Ok(()),
    }
}

/// Asks the rules about an action on an agent or integration, which the principal owns or
/// not, as an agent's owner owns it or not.
fn check(
    principal: &Principal,
    membership: &Membership,
    stored: &Stored,
    action: Action,
) -> Result<(), WorkspaceError> {
    let owner = stored.owner.as_ref().map(|owner| owner.id);
    let owned_by = |id: Option<Uuid>| Resource::Agent {
        kind: stored.kind,
        own: id.is_some() && id == owner,
    };
    membership
        .standing
        .authorize(
            action,
            owned_by(Some(principal.user.id)),
            owned_by(membership.standing.owner_id()),
        )
        .map_err(|denied| WorkspaceError::denied(denied, NO_AGENT))
}
