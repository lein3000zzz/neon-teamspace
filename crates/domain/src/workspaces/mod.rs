//! Workspaces, their members and teamspaces, invitations and invite links, limits, and the
//! permission rules over all of them.
//!
//! An instance holds workspaces. Accounts belong to the instance, and a person can be in
//! many workspaces, as an owner, admin, member or guest. Each workspace has exactly one
//! default teamspace, which everyone but guests is in, and as many open, closed and private
//! teamspaces as its people make.

mod invitations;
mod limits;
mod members;
pub mod permissions;
mod roles;
mod workspace;

pub use invitations::{
    DEFAULT_LINK_TTL, DEFAULT_LINK_USES, INVITATION_TTL, InvalidLinkSpec, Invitation,
    InvitationState, InviteLink, LINK_TOKEN_PREFIX, MAX_LINK_LIFETIME, MAX_LINK_TOKEN_INPUT,
    MAX_LINK_USES, RETENTION, check_link_expiry, check_link_uses,
};
pub use limits::{InvalidLimit, LimitOverrides, LimitValue, LimitValues, Limits, check_limit};
pub use members::{Member, Person, TeamspaceMember};
pub use permissions::{
    Action, Actor, Denied, Newcomer, OwnerStanding, PageSpace, Resource, authorize,
};
pub use roles::{TeamspaceAccess, TeamspaceRole, WorkspaceCreation, WorkspaceRole};
pub use workspace::{
    InvalidIcon, InvalidName, InvalidSlug, MAX_ICON, MAX_NAME, MAX_SLUG, RESERVED_SLUGS, Teamspace,
    Workspace, check_icon, check_name, check_slug, slug_base, suffixed_slug, usable_as_is,
};
