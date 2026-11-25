//! Agents and integrations: members of one workspace each, made by a person there, their
//! owner, and acting for them.
//!
//! - **What they are.** An agent works through API tokens its owner makes, later over MCP
//!   too; an integration delivers what an incoming webhook's sender posts. Each is a member
//!   of its workspace, always with the role member, never admin, owner or guest. It has a
//!   name and a hue, no address and no password, and never signs in.
//! - **Access.** What one may do is what its own place in the workspace allows, its
//!   teamspaces and the pages shared with it, and no more than its owner may do: the
//!   permission rules ask both, see [`authorize`](crate::workspaces::authorize). It joins
//!   teamspaces as a person does, and is a teamspace member, never a teamspace owner. It
//!   has no private pages of its own: its owner could not see them.
//! - **Suspension** is worked out each time, never stored: one is suspended while its owner
//!   is out of its workspace or disabled, or its owner's account is gone. Its tokens are
//!   refused meanwhile, and it resumes as soon as that ends, or once a workspace's owner or
//!   admin hands it to another person there.
//! - **Switches.** The setting `mcp.identities` says whether agents exist on the instance,
//!   and each workspace may turn its agents off and say who makes them: its owners and
//!   admins, or every member. While agents are off, none is made and every agent's tokens
//!   are refused. Integrations come with incoming webhooks, which those switches leave
//!   alone.
//! - **Deletion** keeps the identity, so what it made still names it, and ends its
//!   membership and its tokens.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::{MAX_DISPLAY_NAME, UserKind};
use crate::hue::hue_or_hashed;
use crate::workspaces::TeamspaceRole;

text_enum! {
    /// Who may make agents in a workspace.
    AgentCreation {
        /// The workspace's owners and admins.
        Admins => "admins",
        /// Everyone but guests.
        Members => "members",
    }
}

impl AgentCreation {
    pub const DEFAULT: Self = Self::Members;
}

text_enum! {
    /// Whom an MCP client may act as, the options of the setting `mcp.identities`.
    IdentityMode {
        /// The person who connects it.
        User => "user",
        /// One of that person's agents. Without it, no agent is made and every agent's
        /// tokens are refused.
        Agent => "agent",
    }
}

impl IdentityMode {
    /// The setting's options, in the order the docs list them.
    pub const OPTIONS: &'static [&'static str] = &["user", "agent"];
    /// Both modes.
    pub const DEFAULT: &'static [&'static str] = Self::OPTIONS;

    /// Whether the modes a setting holds, as their texts, include this one.
    pub fn among<S: AsRef<str>>(self, modes: &[S]) -> bool {
        modes.iter().any(|mode| mode.as_ref() == self.as_str())
    }
}

text_enum! {
    /// Whether an agent or integration works now, and why not.
    AgentStatus {
        /// Its tokens work.
        Active => "active",
        /// Its owner is out of its workspace or disabled, or their account is gone.
        Suspended => "suspended",
        /// Agents are off on the instance or in its workspace.
        Disabled => "disabled",
        /// Deleted: what it made still names it.
        Deleted => "deleted",
    }
}

impl AgentStatus {
    /// The status of an identity of `kind`: whether it was deleted, whether its owner is a
    /// member of its workspace who can sign in, and whether agents are on, on the
    /// instance and in the workspace both. Deletion shows first, then suspension.
    pub fn of(kind: UserKind, deleted: bool, owner_active: bool, agents_on: bool) -> Self {
        if deleted {
            Self::Deleted
        } else if !owner_active {
            Self::Suspended
        } else if kind == UserKind::Agent && !agents_on {
            Self::Disabled
        } else {
            Self::Active
        }
    }

    /// Whether its tokens work.
    pub fn works(self) -> bool {
        self == Self::Active
    }
}

/// What decides whether an agent's or integration's tokens work, read with them each time:
/// see [`AgentStatus::of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub deleted: bool,
    /// Its owner is a member of its workspace, and can sign in.
    pub owner_active: bool,
    /// Its workspace has its agents on.
    pub workspace_agents_on: bool,
}

impl Standing {
    /// The status of an identity of `kind`, with agents on or off on the instance.
    pub fn status(self, kind: UserKind, instance_agents_on: bool) -> AgentStatus {
        AgentStatus::of(
            kind,
            self.deleted,
            self.owner_active,
            instance_agents_on && self.workspace_agents_on,
        )
    }
}

/// An agent's or integration's owner, as its lists show them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOwner {
    pub id: Uuid,
    pub display_name: String,
    pub hue_override: Option<u16>,
}

impl AgentOwner {
    pub fn hue(&self) -> u16 {
        hue_or_hashed(self.hue_override, self.id)
    }
}

/// An agent or an integration of a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub id: Uuid,
    /// Agent or integration.
    pub kind: UserKind,
    pub workspace_id: Uuid,
    pub name: String,
    pub hue_override: Option<u16>,
    /// Unset once the owner's account is gone.
    pub owner: Option<AgentOwner>,
    pub status: AgentStatus,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

impl Agent {
    pub fn hue(&self) -> u16 {
        hue_or_hashed(self.hue_override, self.id)
    }
}

/// How an agent or integration is named wherever people are: its name and, through whom it
/// acts, its owner's, `Scout · via Ada Lovelace`, with `(deleted)` once it is.
pub fn attribution(name: &str, owner: Option<&str>, deleted: bool) -> String {
    let mut text = name.to_owned();
    if let Some(owner) = owner {
        text.push_str(" · via ");
        text.push_str(owner);
    }
    if deleted {
        text.push_str(" (deleted)");
    }
    text
}

/// The longest agent name, in characters, as long as a person's.
pub const MAX_AGENT_NAME: usize = MAX_DISPLAY_NAME;

/// An agent name that cannot be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("an agent's name has 1 to {MAX_AGENT_NAME} characters, and no control characters")]
pub struct InvalidAgentName;

/// The name to keep, trimmed.
pub fn check_agent_name(raw: &str) -> Result<String, InvalidAgentName> {
    let name = raw.trim();
    if name.is_empty()
        || name.chars().count() > MAX_AGENT_NAME
        || name.chars().any(char::is_control)
    {
        Err(InvalidAgentName)
    } else {
        Ok(name.to_owned())
    }
}

/// A teamspace role an agent or integration cannot have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("an agent or integration is a teamspace's member, never its owner")]
pub struct DelegateOwnsNothing;

/// Whether an identity of `kind` may have `role` in a teamspace: people any, agents and
/// integrations member only.
pub fn check_teamspace_role(
    kind: UserKind,
    role: TeamspaceRole,
) -> Result<(), DelegateOwnsNothing> {
    if kind.is_human() || role == TeamspaceRole::Member {
        Ok(())
    } else {
        Err(DelegateOwnsNothing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_round_trips_through_its_text() {
        for creation in AgentCreation::ALL {
            assert_eq!(AgentCreation::parse(creation.as_str()), Some(*creation));
        }
        for status in AgentStatus::ALL {
            assert_eq!(AgentStatus::parse(status.as_str()), Some(*status));
        }
        let modes: Vec<&str> = IdentityMode::ALL.iter().map(|m| m.as_str()).collect();
        assert_eq!(modes, IdentityMode::OPTIONS);
        assert_eq!(AgentCreation::DEFAULT, AgentCreation::Members);
    }

    #[test]
    fn modes_are_found_among_the_settings_texts() {
        let both = ["user".to_owned(), "agent".to_owned()];
        let people = ["user"];
        let none: [&str; 0] = [];

        assert!(IdentityMode::Agent.among(&both));
        assert!(!IdentityMode::Agent.among(&people));
        assert!(IdentityMode::User.among(&people));
        assert!(!IdentityMode::User.among(&none));
        assert!(IdentityMode::Agent.among(IdentityMode::DEFAULT));
    }

    #[test]
    fn deletion_shows_first_then_suspension_then_the_switches() {
        use AgentStatus::{Active, Deleted, Disabled, Suspended};
        use UserKind::{Agent, Integration};

        let cases = [
            // kind, deleted, owner active, agents on
            ((Agent, true, false, false), Deleted),
            ((Agent, true, true, true), Deleted),
            ((Agent, false, false, true), Suspended),
            ((Agent, false, false, false), Suspended),
            ((Agent, false, true, false), Disabled),
            ((Agent, false, true, true), Active),
            ((Integration, false, true, false), Active),
            ((Integration, false, false, true), Suspended),
            ((Integration, true, true, true), Deleted),
        ];
        for ((kind, deleted, owner, on), want) in cases {
            let status = AgentStatus::of(kind, deleted, owner, on);
            assert_eq!(
                status, want,
                "{kind} deleted {deleted} owner {owner} on {on}"
            );
            assert_eq!(status.works(), want == Active);
        }
    }

    #[test]
    fn agents_work_only_while_both_switches_are_on() {
        let standing = Standing {
            deleted: false,
            owner_active: true,
            workspace_agents_on: true,
        };
        let off_here = Standing {
            workspace_agents_on: false,
            ..standing
        };

        assert_eq!(standing.status(UserKind::Agent, true), AgentStatus::Active);
        assert_eq!(
            standing.status(UserKind::Agent, false),
            AgentStatus::Disabled
        );
        assert_eq!(
            off_here.status(UserKind::Agent, true),
            AgentStatus::Disabled
        );
        assert_eq!(
            off_here.status(UserKind::Integration, false),
            AgentStatus::Active
        );
    }

    #[test]
    fn an_agent_is_named_with_its_owner() {
        assert_eq!(attribution("Scout", Some("Ada"), false), "Scout · via Ada");
        assert_eq!(
            attribution("Scout", Some("Ada"), true),
            "Scout · via Ada (deleted)"
        );
        assert_eq!(attribution("Scout", None, false), "Scout");
        assert_eq!(attribution("Scout", None, true), "Scout (deleted)");
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(check_agent_name("  Scout "), Ok("Scout".into()));
        assert_eq!(check_agent_name("  "), Err(InvalidAgentName));
        assert_eq!(check_agent_name("a\nb"), Err(InvalidAgentName));
        assert_eq!(
            check_agent_name(&"x".repeat(MAX_AGENT_NAME + 1)),
            Err(InvalidAgentName)
        );
        assert!(check_agent_name(&"é".repeat(MAX_AGENT_NAME)).is_ok());
    }

    #[test]
    fn agents_and_integrations_are_teamspace_members_only() {
        for kind in UserKind::ALL {
            for role in TeamspaceRole::ALL {
                let allowed = kind.is_human() || *role == TeamspaceRole::Member;
                assert_eq!(
                    check_teamspace_role(*kind, *role).is_ok(),
                    allowed,
                    "{kind} as {role}"
                );
            }
        }
    }
}
