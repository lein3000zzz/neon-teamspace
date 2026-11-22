//! Rate limits on the attempts that guess or flood: sign-ins, sign-ups, mailed links,
//! requests to join, invite lookups, and invitations into workspaces.
//!
//! [`RateLimiter`] is the port; [`MemoryLimiter`] keeps the buckets in this process's
//! memory, which suits one server process. Several would need a shared store behind the
//! same port.

mod memory;

use std::time::Duration;

use async_trait::async_trait;

pub use memory::MemoryLimiter;

/// How many attempts a key gets at once, and how often one more comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    pub burst: u32,
    pub every: Duration,
}

/// Ten attempts, then one every 30 seconds: what one client gets at each guarded door.
pub const STANDARD: Quota = Quota {
    burst: 10,
    every: Duration::from_secs(30),
};

/// Fifty attempts, then one every 6 seconds: sign-ins to one address from every IP
/// together. Looser than [`STANDARD`] per IP and address, so a stranger cannot lock the
/// owner out, yet guessing from many places at once stays slow.
pub const ACCOUNT: Quota = Quota {
    burst: 50,
    every: Duration::from_secs(6),
};

/// What is being limited. Each has its own buckets, keyed by an IP, an address or a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bucket {
    /// Sign-ins from one IP, checked first so a client it stops costs nothing more.
    LoginIp,
    /// Sign-ins to one address from one IP.
    LoginIpEmail,
    /// Sign-ins to one address from every IP together.
    LoginEmail,
    Register,
    /// Verifying an address, and asking for the link again.
    Verify,
    /// Asking for a password reset link, and using one.
    Reset,
    /// Asking to join.
    Join,
    /// Confirming a request to join.
    JoinConfirm,
    /// Looking an invite up for the sign-up form.
    InviteLookup,
    /// Changing one's password, by user.
    PasswordChange,
    /// Inviting into a workspace by address, by inviter: each answer says whether the
    /// address has an account, so probing for addresses is slow.
    WorkspaceInvite,
    /// Joining a workspace by invite link, by user.
    WorkspaceLinkJoin,
}

impl Bucket {
    pub const ALL: [Self; 12] = [
        Self::LoginIp,
        Self::LoginIpEmail,
        Self::LoginEmail,
        Self::Register,
        Self::Verify,
        Self::Reset,
        Self::Join,
        Self::JoinConfirm,
        Self::InviteLookup,
        Self::PasswordChange,
        Self::WorkspaceInvite,
        Self::WorkspaceLinkJoin,
    ];

    /// Starts the keys of this bucket, so buckets never share one.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::LoginIp => "login:ip:",
            Self::LoginIpEmail => "login:ip-email:",
            Self::LoginEmail => "login:email:",
            Self::Register => "register:ip:",
            Self::Verify => "verify:ip:",
            Self::Reset => "reset:ip:",
            Self::Join => "join:ip:",
            Self::JoinConfirm => "join-confirm:ip:",
            Self::InviteLookup => "invite:ip:",
            Self::PasswordChange => "password:user:",
            Self::WorkspaceInvite => "workspace-invite:user:",
            Self::WorkspaceLinkJoin => "workspace-link:user:",
        }
    }

    pub fn quota(self) -> Quota {
        match self {
            Self::LoginEmail => ACCOUNT,
            _ => STANDARD,
        }
    }
}

/// An attempt over the limit, and how long until the next one is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limited {
    pub retry_after: Duration,
}

/// Counts attempts and says when there were too many.
#[async_trait]
pub trait RateLimiter: Send + Sync + 'static {
    /// Takes an attempt for `key` from the bucket, or says when one is next allowed.
    async fn check(&self, bucket: Bucket, key: &str) -> Result<(), Limited>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn buckets_have_distinct_prefixes() {
        let prefixes: HashSet<&str> = Bucket::ALL.iter().map(|b| b.prefix()).collect();

        assert_eq!(prefixes.len(), Bucket::ALL.len());
        assert!(prefixes.iter().all(|prefix| prefix.ends_with(':')));
    }

    #[test]
    fn only_the_cross_ip_bucket_is_looser() {
        for bucket in Bucket::ALL {
            let want = if bucket == Bucket::LoginEmail {
                ACCOUNT
            } else {
                STANDARD
            };
            assert_eq!(bucket.quota(), want, "{bucket:?}");
        }
    }
}
