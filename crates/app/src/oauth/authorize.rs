//! Registering clients, and the authorization request: checking it, the consent page's
//! choices, and the person's answer.

use neon_domain::agents::{Agent, AgentStatus, IdentityMode};
use neon_domain::auth::{Principal, Scope, UserKind};
use neon_domain::network::client_source;
use neon_domain::oauth::{
    AuthorizationRequest, CODE_TTL, CheckedRequest, Client, ClientMetadata, Endpoints, ErrorCode,
    Identity, Modes, RequestProblem, check_registration, check_request, chosen_scopes,
    offered_scopes, parse_client_id, redirect_host,
};
use neon_domain::workspaces::{Action, Resource, Workspace, WorkspaceRole};
use neon_store::agents::{self as agent_store, Stored};
use neon_store::oauth::{self as repo, Code, NewGrant};
use neon_store::workspaces;
use sqlx::PgConnection;
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;

use super::{OAuthError, OAuthService};
use crate::crypto::{hash_secret, new_secret};
use crate::ratelimit::Bucket;
use crate::settings::{mcp, oauth as keys};
use crate::standing::Standing;

const NO_WORKSPACE: &str = "no such workspace";
const NO_AGENT: &str = "no such agent";

/// What the consent page shows of a request that holds: the client, where the browser
/// goes back to, and, for each of the person's workspaces, whom the client may act as
/// there and with which scopes.
#[derive(Debug, Clone)]
pub struct Consent {
    pub client: Client,
    pub request: CheckedRequest,
    /// Where the browser goes back to, as [`redirect_host`] names it.
    pub redirect_host: String,
    pub workspaces: Vec<ConsentWorkspace>,
}

/// One of the person's workspaces, as the consent page offers it.
#[derive(Debug, Clone)]
pub struct ConsentWorkspace {
    pub workspace: Workspace,
    pub role: WorkspaceRole,
    /// The scopes the client may have acting as the person; `None` while `mcp.identities`
    /// leaves that out.
    pub as_person: Option<Vec<Scope>>,
    /// Whether the client may act as an agent here: `mcp.identities` and the workspace's
    /// switch both allow it.
    pub agents_on: bool,
    /// The person's agents here that work, which the client may act as.
    pub agents: Vec<Agent>,
    /// The scopes the client may have acting as one of them.
    pub agent_scopes: Vec<Scope>,
}

/// What a person chose on the consent page.
#[derive(Debug, Clone)]
pub struct Approval {
    pub workspace_id: Uuid,
    pub identity: Identity,
    pub scopes: Vec<String>,
}

impl OAuthService {
    /// Registers a client as it describes itself, public, with no secret: PKCE stands in
    /// for one. While `oauth.registration` is on, at most a few at a time from one source,
    /// an IPv4 address or an IPv6 /64 as [`client_source`] names it, while fewer than
    /// `oauth.max_unused_clients_per_source` clients of that source wait unconnected, and
    /// fewer than `oauth.max_unused_clients` of all.
    pub async fn register(
        &self,
        metadata: &ClientMetadata,
        ip: &str,
    ) -> Result<Client, OAuthError> {
        self.observe("register", async {
            if !self.registration_open() {
                return Err(OAuthError::RegistrationOff);
            }
            let source = client_source(ip);
            self.limit(Bucket::OAuthRegister, &source).await?;
            let registered = check_registration(metadata).map_err(OAuthError::Protocol)?;
            let settings = self.settings();
            let theirs = repo::count_unused_clients_from(self.pool(), &source).await?;
            if theirs >= *settings.get(keys::MAX_UNUSED_CLIENTS_PER_SOURCE) {
                warn!(
                    unused = theirs,
                    source, "OAuth registration refused: too many of its clients wait unconnected"
                );
                return Err(OAuthError::SourceLimit);
            }
            let unused = repo::count_unused_clients(self.pool()).await?;
            if unused >= *settings.get(keys::MAX_UNUSED_CLIENTS) {
                warn!(
                    unused,
                    "OAuth registration refused: too many clients wait unconnected"
                );
                return Err(OAuthError::ClientLimit);
            }
            let client = Client {
                id: Uuid::now_v7(),
                name: registered.name,
                redirect_uris: registered.redirect_uris,
                created_at: self.now(),
                last_used_at: None,
            };
            repo::insert_client(self.pool(), &client, &source).await?;
            info!(client_id = %client.id, ip, "OAuth client registered");
            Ok(client)
        })
        .await
    }

    /// Checks an authorization request as the authorization endpoint receives it, before
    /// the browser goes on to the consent page.
    pub async fn check_authorization(
        &self,
        request: &AuthorizationRequest,
        endpoints: &Endpoints,
    ) -> Result<(Client, CheckedRequest), OAuthError> {
        self.observe("authorize", async {
            let mut conn = self.pool().acquire().await?;
            self.checked(&mut conn, request, endpoints).await
        })
        .await
    }

    /// What the consent page offers the person signed in at a browser: the request,
    /// checked again, and each of their workspaces with whom the client may act as there.
    pub async fn consent(
        &self,
        principal: &Principal,
        request: &AuthorizationRequest,
        endpoints: &Endpoints,
    ) -> Result<Consent, OAuthError> {
        self.observe("consent", async {
            require_session(principal)?;
            let mut conn = self.pool().acquire().await?;
            let (client, request) = self.checked(&mut conn, request, endpoints).await?;
            let identities = self.settings();
            let identities = identities.get(mcp::IDENTITIES);
            let agents_on_here = IdentityMode::Agent.among(identities);
            let person = principal.user.id;
            let mut offered = Vec::new();
            for (workspace, role) in workspaces::list_for_user(&mut *conn, person).await? {
                let modes = Modes::of(identities, workspace.agents_enabled);
                let agents = if modes.agents {
                    agent_store::list(&mut *conn, workspace.id)
                        .await?
                        .into_iter()
                        .map(|stored| agent_of(stored, agents_on_here))
                        .filter(|agent| {
                            agent.kind == UserKind::Agent
                                && agent.status == AgentStatus::Active
                                && agent.owner.as_ref().is_some_and(|owner| owner.id == person)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                offered.push(ConsentWorkspace {
                    as_person: modes
                        .person
                        .then(|| offered_scopes(IdentityMode::User, role)),
                    agents_on: modes.agents && role.joins_teamspaces(),
                    agents,
                    agent_scopes: offered_scopes(IdentityMode::Agent, role),
                    workspace,
                    role,
                });
            }
            Ok(Consent {
                redirect_host: redirect_host(&request.redirect_uri),
                client,
                request,
                workspaces: offered,
            })
        })
        .await
    }

    /// Grants the client what the person chose, from a browser session alone, and gives
    /// where to send the browser: the client's redirect URI with a code that works once,
    /// for a minute, with the request's PKCE verifier. Consenting again to the same client,
    /// workspace and identity replaces the grant's scopes, and the connections its tokens
    /// hold open look again at once, keeping to fewer if they are.
    pub async fn approve(
        &self,
        principal: &Principal,
        request: &AuthorizationRequest,
        approval: &Approval,
        endpoints: &Endpoints,
    ) -> Result<String, OAuthError> {
        self.observe("approve", async {
            require_session(principal)?;
            let mut tx = self.pool().begin().await?;
            let (client, request) = self.checked(&mut tx, request, endpoints).await?;
            let workspace_id = approval.workspace_id;
            let workspace = workspaces::get(&mut *tx, workspace_id)
                .await?
                .ok_or(OAuthError::NotFound(NO_WORKSPACE))?;
            let standing = Standing::read(&mut tx, principal, workspace_id).await?;
            let role = standing.role().ok_or(OAuthError::NotFound(NO_WORKSPACE))?;
            let settings = self.settings();
            let identities = settings.get(mcp::IDENTITIES);
            let modes = Modes::of(identities, workspace.agents_enabled);
            if !modes.allow(approval.identity) {
                return Err(OAuthError::IdentityUnavailable(match approval.identity {
                    Identity::Person => "clients do not act as the people who connect them here",
                    Identity::Agent(_) => "clients do not act as agents in this workspace",
                }));
            }
            let (user_id, offered) = match approval.identity {
                Identity::Person => (principal.user.id, offered_scopes(IdentityMode::User, role)),
                Identity::Agent(agent_id) => {
                    let stored = agent_store::get(&mut *tx, workspace_id, agent_id)
                        .await?
                        .ok_or(OAuthError::NotFound(NO_AGENT))?;
                    usable_agent(
                        &standing,
                        principal,
                        stored,
                        IdentityMode::Agent.among(identities),
                    )?;
                    (agent_id, offered_scopes(IdentityMode::Agent, role))
                }
            };
            let scopes = chosen_scopes(&approval.scopes, &offered)?;
            let now = self.now();
            let new_id = Uuid::now_v7();
            let grant = repo::save_grant(
                &mut *tx,
                &NewGrant {
                    id: new_id,
                    client_id: client.id,
                    person_id: principal.user.id,
                    user_id,
                    workspace_id,
                    scopes: scopes.clone(),
                    audience: request.audience,
                    at: now,
                },
            )
            .await?;
            let code = new_secret("");
            repo::insert_code(
                &mut *tx,
                &hash_secret(&code),
                &Code {
                    grant_id: grant.id,
                    redirect_uri: request.redirect_uri.clone(),
                    challenge: request.code_challenge.clone(),
                    scopes,
                    audience: request.audience,
                    created_at: now,
                    expires_at: now + CODE_TTL,
                    used_at: None,
                },
            )
            .await?;
            repo::touch_client(&mut *tx, client.id, now).await?;
            tx.commit().await?;
            if grant.id != new_id {
                self.grant_changed(user_id);
            }
            info!(
                user_id = %principal.user.id,
                client_id = %client.id,
                grant_id = %grant.id,
                workspace_id = %workspace_id,
                acts_as = %user_id,
                scopes = ?grant.scopes,
                "OAuth client approved"
            );
            back_to(&request, &[("code", &code)], endpoints)
        })
        .await
    }

    /// Where to send the browser when the person denies the client: its redirect URI with
    /// `error=access_denied`.
    pub async fn deny(
        &self,
        principal: &Principal,
        request: &AuthorizationRequest,
        endpoints: &Endpoints,
    ) -> Result<String, OAuthError> {
        self.observe("deny", async {
            require_session(principal)?;
            let mut conn = self.pool().acquire().await?;
            let (client, request) = self.checked(&mut conn, request, endpoints).await?;
            info!(user_id = %principal.user.id, client_id = %client.id, "OAuth client denied");
            back_to(
                &request,
                &[
                    ("error", ErrorCode::AccessDenied.as_str()),
                    ("error_description", "the person did not allow it"),
                ],
                endpoints,
            )
        })
        .await
    }

    /// The request's client, and the request checked against it.
    async fn checked(
        &self,
        conn: &mut PgConnection,
        request: &AuthorizationRequest,
        endpoints: &Endpoints,
    ) -> Result<(Client, CheckedRequest), OAuthError> {
        let client_id =
            parse_client_id(request.client_id.as_deref()).map_err(OAuthError::Request)?;
        let client = repo::client(&mut *conn, client_id)
            .await?
            .ok_or(OAuthError::Request(RequestProblem::UnknownClient))?;
        let checked = check_request(request, &client, endpoints).map_err(OAuthError::Request)?;
        Ok((client, checked))
    }
}

/// An agent as stored, with its status as the instance's switch says now.
fn agent_of(stored: Stored, agents_on: bool) -> Agent {
    Agent {
        status: stored.standing.status(stored.kind, agents_on),
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

/// Refuses an agent the person may not connect a client as: one of another, an
/// integration, or one that does not work now. Connecting one is making a credential for
/// it, which the rules allow its owner alone, from a browser.
fn usable_agent(
    standing: &Standing,
    principal: &Principal,
    stored: Stored,
    agents_on: bool,
) -> Result<(), OAuthError> {
    let owner = stored.owner.as_ref().map(|owner| owner.id);
    let resource = Resource::Agent {
        kind: stored.kind,
        own: owner == Some(principal.user.id),
    };
    standing
        .authorize_same(Action::CreateAgentToken, resource)
        .map_err(|denied| OAuthError::denied(denied, NO_AGENT))?;
    match agent_of(stored, agents_on).status {
        AgentStatus::Active => Ok(()),
        AgentStatus::Deleted => Err(OAuthError::IdentityUnavailable("this agent is deleted")),
        AgentStatus::Suspended => Err(OAuthError::IdentityUnavailable(
            "this agent is suspended while its owner is out of the workspace, a guest there or \
             disabled",
        )),
        AgentStatus::Disabled => Err(OAuthError::IdentityUnavailable(
            "agents are turned off: the client could not act as one",
        )),
    }
}

fn require_session(principal: &Principal) -> Result<(), OAuthError> {
    if principal.via_session() {
        Ok(())
    } else {
        Err(OAuthError::Forbidden(
            "connecting a client needs a browser session: tokens cannot",
        ))
    }
}

/// The client's redirect URI with `params` added, then its state and the issuer, which
/// clients check (RFC 9207).
fn back_to(
    request: &CheckedRequest,
    params: &[(&str, &str)],
    endpoints: &Endpoints,
) -> Result<String, OAuthError> {
    let mut url = Url::parse(&request.redirect_uri)
        .map_err(|_| OAuthError::Internal("a checked redirect URI does not parse".to_owned()))?;
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in params {
            query.append_pair(key, value);
        }
        if let Some(state) = &request.state {
            query.append_pair("state", state);
        }
        query.append_pair("iss", endpoints.issuer());
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use neon_domain::oauth::Audience;

    use super::*;

    fn request(redirect_uri: &str, state: Option<&str>) -> CheckedRequest {
        CheckedRequest {
            client_id: Uuid::nil(),
            redirect_uri: redirect_uri.to_owned(),
            code_challenge: String::new(),
            scopes: Vec::new(),
            state: state.map(str::to_owned),
            audience: Audience::Mcp,
        }
    }

    #[test]
    fn the_answer_keeps_the_redirect_query_and_adds_state_and_issuer() {
        let endpoints = Endpoints::new("https://neon.example.com");

        assert_eq!(
            back_to(
                &request("https://app.example.com/cb?client=cli", Some("a b&c")),
                &[("code", "xyz")],
                &endpoints
            )
            .expect("an address"),
            "https://app.example.com/cb?client=cli&code=xyz&state=a+b%26c&iss=https%3A%2F%2Fneon.example.com"
        );
        assert_eq!(
            back_to(
                &request("com.example.app:/oauth2redirect", None),
                &[("error", "access_denied")],
                &endpoints
            )
            .expect("an address"),
            "com.example.app:/oauth2redirect?error=access_denied&iss=https%3A%2F%2Fneon.example.com"
        );
    }
}
