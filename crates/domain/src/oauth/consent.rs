//! Consent: whom a client may act as, in which workspace, and with which scopes, and how
//! its tokens' scopes follow the grant afterwards.
//!
//! A person connects a client as themselves, or as one of their agents in the workspace,
//! as the setting `mcp.identities` and the workspace's agent switch allow. Acting as
//! themselves it may read and write, and administer the workspace when they own or
//! administer it; acting as an agent it reads and writes, never more. The scopes a token
//! has are the ones it was given that the grant still has: consenting to fewer narrows
//! every token at once, and refreshing never widens them.

use uuid::Uuid;

use super::{ErrorCode, Grant, Problem};
use crate::agents::{IdentityMode, Standing};
use crate::auth::{Scope, UserKind};
use crate::workspaces::WorkspaceRole;

/// Whom a client acts as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identity {
    /// The person who connects it.
    Person,
    /// One of their agents in the workspace.
    Agent(Uuid),
}

/// Which identities a workspace offers a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    /// Acting as the person.
    pub person: bool,
    /// Acting as one of their agents.
    pub agents: bool,
}

impl Modes {
    /// What `mcp.identities`, as its texts, allows, in a workspace whose agents are on or
    /// off.
    pub fn of<S: AsRef<str>>(identities: &[S], workspace_agents_on: bool) -> Self {
        Self {
            person: IdentityMode::User.among(identities),
            agents: IdentityMode::Agent.among(identities) && workspace_agents_on,
        }
    }

    /// Whether `identity` is one of them.
    pub fn allow(self, identity: Identity) -> bool {
        match identity {
            Identity::Person => self.person,
            Identity::Agent(_) => self.agents,
        }
    }
}

/// The scopes a client may get, acting as an agent or as a person whose role in the
/// workspace is `role`, in [`Scope::ALL`] order.
pub fn offered_scopes(as_agent: bool, role: WorkspaceRole) -> Vec<Scope> {
    Scope::ALL
        .into_iter()
        .filter(|scope| *scope != Scope::Admin || !as_agent && role.manages())
        .collect()
}

/// Scopes a person cannot choose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidChoice {
    #[error("choose at least one scope, or deny")]
    None,
    #[error("there is no scope {0:?}: the scopes are read, write and admin")]
    Unknown(String),
    #[error("the {0} scope is not offered here")]
    NotOffered(Scope),
}

/// The scopes a person chose of those `offered`: each once, in [`Scope::ALL`] order.
pub fn chosen_scopes<S: AsRef<str>>(
    chosen: &[S],
    offered: &[Scope],
) -> Result<Vec<Scope>, InvalidChoice> {
    let mut scopes = Vec::with_capacity(chosen.len());
    for text in chosen {
        let text = text.as_ref();
        let scope = Scope::parse(text).ok_or_else(|| InvalidChoice::Unknown(text.to_owned()))?;
        if !offered.contains(&scope) {
            return Err(InvalidChoice::NotOffered(scope));
        }
        scopes.push(scope);
    }
    if scopes.is_empty() {
        return Err(InvalidChoice::None);
    }
    scopes.sort();
    scopes.dedup();
    Ok(scopes)
}

/// The scopes a token has now: those it was given that the grant still has.
pub fn narrowed(given: &[Scope], grant: &[Scope]) -> Vec<Scope> {
    Scope::ALL
        .into_iter()
        .filter(|scope| given.contains(scope) && grant.contains(scope))
        .collect()
}

/// The scopes a refresh gives: those of the refresh token that the grant still has, or the
/// fewer a `scope` parameter asks for. Never one the refresh token did not have.
pub fn refreshed_scopes(
    asked: Option<&str>,
    had: &[Scope],
    grant: &[Scope],
) -> Result<Vec<Scope>, Problem> {
    let wanted = match asked {
        None => had.to_vec(),
        Some(text) => {
            let wanted = super::request::parse_scope(text).ok_or_else(|| {
                Problem::new(
                    ErrorCode::InvalidScope,
                    "the scopes are read, write and admin",
                )
            })?;
            if let Some(extra) = wanted.iter().find(|scope| !had.contains(scope)) {
                return Err(Problem::new(
                    ErrorCode::InvalidScope,
                    format!("the {extra} scope was not granted"),
                ));
            }
            wanted
        }
    };
    let scopes = narrowed(&wanted, grant);
    if scopes.is_empty() {
        Err(Problem::new(
            ErrorCode::InvalidScope,
            "none of those scopes is granted any more",
        ))
    } else {
        Ok(scopes)
    }
}

/// What decides whether a grant works now, read with it on every use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantStanding {
    /// The person who consented cannot sign in.
    pub person_disabled: bool,
    /// Their role in the grant's workspace; `None` once they are out of it.
    pub person_role: Option<WorkspaceRole>,
    /// What the client acts as.
    pub kind: UserKind,
    /// The owner of the identity, for an agent.
    pub owner_id: Option<Uuid>,
    /// The workspace of the identity, for an agent.
    pub identity_workspace_id: Option<Uuid>,
    /// For an agent, what decides whether it works.
    pub agent: Standing,
}

impl GrantStanding {
    /// Whether `grant` works now, with `mcp.identities` as its texts: its person can sign
    /// in, and the identity it acts as is still one the setting allows. As the person, they
    /// are in its workspace. As an agent, the agent is still theirs, of that workspace, and
    /// works: not deleted, nor suspended, nor switched off.
    pub fn works<S: AsRef<str>>(&self, grant: &Grant, identities: &[S]) -> bool {
        if self.person_disabled {
            return false;
        }
        if !grant.acts_as_agent() {
            return self.kind == UserKind::Human
                && IdentityMode::User.among(identities)
                && self.person_role.is_some();
        }
        let agents_on = IdentityMode::Agent.among(identities);
        self.kind == UserKind::Agent
            && self.owner_id == Some(grant.person_id)
            && self.identity_workspace_id == Some(grant.workspace_id)
            && self.agent.status(self.kind, agents_on).works()
    }
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::super::Audience;
    use super::*;

    const BOTH: &[&str] = &["user", "agent"];

    #[test]
    fn the_setting_and_the_workspace_switch_decide_the_identities() {
        let both = Modes {
            person: true,
            agents: true,
        };
        assert_eq!(Modes::of(BOTH, true), both);
        assert_eq!(
            Modes::of(BOTH, false),
            Modes {
                person: true,
                agents: false
            }
        );
        assert_eq!(
            Modes::of(&["agent"], true),
            Modes {
                person: false,
                agents: true
            }
        );
        assert_eq!(
            Modes::of(&["user"], true),
            Modes {
                person: true,
                agents: false
            }
        );
        let none: [&str; 0] = [];
        assert_eq!(
            Modes::of(&none, true),
            Modes {
                person: false,
                agents: false
            }
        );
        assert!(both.allow(Identity::Person) && both.allow(Identity::Agent(Uuid::nil())));
        assert!(!Modes::of(&["user"], true).allow(Identity::Agent(Uuid::nil())));
        assert!(!Modes::of(&["agent"], true).allow(Identity::Person));
    }

    #[test]
    fn admin_is_offered_to_owners_and_admins_acting_as_themselves() {
        for role in WorkspaceRole::ALL {
            let admin = role.manages();
            assert_eq!(
                offered_scopes(false, *role).contains(&Scope::Admin),
                admin,
                "{role}"
            );
            assert_eq!(
                offered_scopes(true, *role),
                [Scope::Read, Scope::Write],
                "an agent of an {role} reads and writes alone"
            );
        }
        assert_eq!(
            offered_scopes(false, WorkspaceRole::Owner),
            Scope::ALL.to_vec()
        );
        assert_eq!(
            offered_scopes(false, WorkspaceRole::Guest),
            [Scope::Read, Scope::Write]
        );
    }

    #[test]
    fn a_choice_is_some_of_the_scopes_offered() {
        let offered = [Scope::Read, Scope::Write];
        let none: [&str; 0] = [];

        assert_eq!(
            chosen_scopes(&["write", "read", "write"], &offered),
            Ok(vec![Scope::Read, Scope::Write])
        );
        assert_eq!(chosen_scopes(&none, &offered), Err(InvalidChoice::None));
        assert_eq!(
            chosen_scopes(&["read", "admin"], &offered),
            Err(InvalidChoice::NotOffered(Scope::Admin))
        );
        assert_eq!(
            chosen_scopes(&["delete"], &offered),
            Err(InvalidChoice::Unknown("delete".into()))
        );
    }

    #[test]
    fn a_token_keeps_only_what_the_grant_still_has() {
        let all = Scope::ALL;

        assert_eq!(narrowed(&all, &[Scope::Read]), [Scope::Read]);
        assert_eq!(
            narrowed(&[Scope::Write, Scope::Read], &all),
            [Scope::Read, Scope::Write]
        );
        assert_eq!(narrowed(&[Scope::Admin], &[Scope::Read]), []);
    }

    #[test]
    fn refreshing_never_widens() {
        let had = [Scope::Read, Scope::Write];
        let grant = Scope::ALL;

        assert_eq!(refreshed_scopes(None, &had, &grant), Ok(had.to_vec()));
        assert_eq!(
            refreshed_scopes(Some("read"), &had, &grant),
            Ok(vec![Scope::Read])
        );
        let wider = refreshed_scopes(Some("read admin"), &had, &grant).expect_err("wider");
        assert_eq!(wider.code, ErrorCode::InvalidScope);
        assert!(wider.description.contains("admin"), "{wider}");
        assert_eq!(
            refreshed_scopes(None, &had, &[Scope::Read]),
            Ok(vec![Scope::Read]),
            "a grant narrowed since narrows the refresh"
        );
        let gone = refreshed_scopes(Some("write"), &had, &[Scope::Read]).expect_err("gone");
        assert_eq!(gone.code, ErrorCode::InvalidScope);
        let unknown = refreshed_scopes(Some("read sudo"), &had, &grant).expect_err("unknown");
        assert_eq!(unknown.code, ErrorCode::InvalidScope);
    }

    fn grant(as_agent: bool) -> Grant {
        let at = OffsetDateTime::UNIX_EPOCH;
        Grant {
            id: Uuid::from_u128(1),
            client_id: Uuid::from_u128(2),
            person_id: Uuid::from_u128(3),
            user_id: Uuid::from_u128(if as_agent { 4 } else { 3 }),
            workspace_id: Uuid::from_u128(5),
            scopes: vec![Scope::Read],
            audience: Audience::Mcp,
            created_at: at,
            updated_at: at,
            last_used_at: None,
        }
    }

    const WORKING_AGENT: Standing = Standing {
        deleted: false,
        owner_active: true,
        workspace_agents_on: true,
    };

    fn person() -> GrantStanding {
        GrantStanding {
            person_disabled: false,
            person_role: Some(WorkspaceRole::Member),
            kind: UserKind::Human,
            owner_id: None,
            identity_workspace_id: None,
            agent: WORKING_AGENT,
        }
    }

    fn agent() -> GrantStanding {
        GrantStanding {
            kind: UserKind::Agent,
            owner_id: Some(Uuid::from_u128(3)),
            identity_workspace_id: Some(Uuid::from_u128(5)),
            ..person()
        }
    }

    #[test]
    fn a_persons_grant_works_while_they_are_in_the_workspace() {
        let mine = grant(false);

        assert!(person().works(&mine, BOTH));
        for role in WorkspaceRole::ALL {
            let standing = GrantStanding {
                person_role: Some(*role),
                ..person()
            };
            assert!(standing.works(&mine, BOTH), "{role}");
        }
        for (standing, why) in [
            (
                GrantStanding {
                    person_role: None,
                    ..person()
                },
                "out of the workspace",
            ),
            (
                GrantStanding {
                    person_disabled: true,
                    ..person()
                },
                "disabled",
            ),
            (
                GrantStanding {
                    kind: UserKind::Agent,
                    ..person()
                },
                "not a person",
            ),
        ] {
            assert!(!standing.works(&mine, BOTH), "{why}");
        }
        assert!(
            !person().works(&mine, &["agent"]),
            "acting as the person is switched off"
        );
    }

    #[test]
    fn an_agents_grant_works_while_the_agent_does_and_is_theirs() {
        let theirs = grant(true);

        assert!(agent().works(&theirs, BOTH));
        assert!(
            agent().works(&theirs, &["agent"]),
            "acting as a person may be off"
        );
        for (standing, why) in [
            (
                GrantStanding {
                    owner_id: Some(Uuid::from_u128(9)),
                    ..agent()
                },
                "handed to someone else",
            ),
            (
                GrantStanding {
                    owner_id: None,
                    ..agent()
                },
                "its owner's account is gone",
            ),
            (
                GrantStanding {
                    identity_workspace_id: Some(Uuid::from_u128(9)),
                    ..agent()
                },
                "of another workspace",
            ),
            (
                GrantStanding {
                    person_disabled: true,
                    ..agent()
                },
                "its person disabled",
            ),
            (
                GrantStanding {
                    agent: Standing {
                        owner_active: false,
                        ..WORKING_AGENT
                    },
                    ..agent()
                },
                "suspended",
            ),
            (
                GrantStanding {
                    agent: Standing {
                        deleted: true,
                        ..WORKING_AGENT
                    },
                    ..agent()
                },
                "deleted",
            ),
            (
                GrantStanding {
                    agent: Standing {
                        workspace_agents_on: false,
                        ..WORKING_AGENT
                    },
                    ..agent()
                },
                "agents off in the workspace",
            ),
            (
                GrantStanding {
                    kind: UserKind::Integration,
                    ..agent()
                },
                "an integration",
            ),
            (
                GrantStanding {
                    kind: UserKind::Human,
                    ..agent()
                },
                "a person",
            ),
        ] {
            assert!(!standing.works(&theirs, BOTH), "{why}");
        }
        assert!(
            !agent().works(&theirs, &["user"]),
            "agents are switched off on the instance"
        );
    }
}
