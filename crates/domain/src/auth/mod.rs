//! Accounts and how they sign in: users and roles, registration modes, password rules,
//! invites, requests to join, sessions, API tokens and their scopes, mailed links, and
//! each user's preferences.
//!
//! Everything here is a rule over plain values. Hashing, randomness and storage belong to
//! the use cases and the repositories.

pub mod email;
pub mod email_token;
pub mod invite;
pub mod join_request;
pub mod password;
pub mod preferences;
mod principal;
mod registration;
pub mod session;
pub mod token;
mod user;

pub use email::{Email, InvalidEmail};
pub use email_token::EmailTokenPurpose;
pub use invite::{Invite, InviteStatus, InviteUse};
pub use join_request::{JoinRequest, JoinRequestStatus};
pub use password::{PasswordPolicy, WeakPassword};
pub use preferences::Preferences;
pub use principal::{Credential, Principal};
pub use registration::RegistrationMode;
pub use session::{Session, SessionPolicy};
pub use token::{ApiToken, Scope, TokenHolder};
pub use user::{InvalidDisplayName, MAX_DISPLAY_NAME, Role, User, UserKind, display_name};
