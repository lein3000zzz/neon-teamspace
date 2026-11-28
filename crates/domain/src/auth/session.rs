//! Browser sessions: an opaque token in a cookie, kept only as its hash.
//!
//! A session expires when it goes unused for the idle time, which slides forward with
//! use, or at its absolute expiry, which never moves, whichever comes first.

use std::time::Duration;

use time::OffsetDateTime;
use uuid::Uuid;

/// The least time between two writes of a session's last use, or a token's, to save
/// writes on every request.
pub const TOUCH_INTERVAL: Duration = Duration::from_secs(60);

/// The longest user agent kept with a session, in bytes.
pub const MAX_USER_AGENT: usize = 512;

/// `auth.session_idle_ttl` and `auth.session_absolute_ttl` unless set.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(7 * 24 * 3600);
pub const DEFAULT_ABSOLUTE: Duration = Duration::from_secs(30 * 24 * 3600);

/// The shortest either may be.
pub const MIN_TTL: Duration = Duration::from_secs(5 * 60);

/// How long sessions last, as the settings say now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionPolicy {
    pub idle: Duration,
    pub absolute: Duration,
}

/// When a new session expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expiry {
    /// The idle expiry, never past the absolute one.
    pub expires_at: OffsetDateTime,
    pub absolute_expires_at: OffsetDateTime,
}

impl SessionPolicy {
    /// The expiries of a session opened at `now`.
    pub fn open(&self, now: OffsetDateTime) -> Expiry {
        let absolute_expires_at = now + self.absolute;
        Expiry {
            expires_at: (now + self.idle).min(absolute_expires_at),
            absolute_expires_at,
        }
    }

    /// The new idle expiry of a session used at `now`, when its last use was written at
    /// least [`TOUCH_INTERVAL`] ago; `None` when it is not due a write yet.
    pub fn touch(&self, session: &Session, now: OffsetDateTime) -> Option<OffsetDateTime> {
        (now - session.last_seen_at >= TOUCH_INTERVAL)
            .then(|| (now + self.idle).min(session.absolute_expires_at))
    }
}

/// A browser session. Its token exists only in the cookie; the database keeps its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub user_agent: String,
    pub ip: String,
    pub created_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    /// Slides forward with use.
    pub expires_at: OffsetDateTime,
    /// Never moves.
    pub absolute_expires_at: OffsetDateTime,
}

impl Session {
    /// Past either expiry at `now`.
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        now >= self.expires_at || now >= self.absolute_expires_at
    }
}

/// A user agent as kept: at most [`MAX_USER_AGENT`] bytes, cut at a character boundary.
pub fn truncate_user_agent(user_agent: &str) -> String {
    user_agent[..user_agent.floor_char_boundary(MAX_USER_AGENT)].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);
    const POLICY: SessionPolicy = SessionPolicy {
        idle: Duration::from_secs(7 * 24 * 3600),
        absolute: Duration::from_secs(30 * 24 * 3600),
    };

    fn now() -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn session(policy: SessionPolicy) -> Session {
        let expiry = policy.open(now());
        Session {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            user_agent: String::new(),
            ip: String::new(),
            created_at: now(),
            last_seen_at: now(),
            expires_at: expiry.expires_at,
            absolute_expires_at: expiry.absolute_expires_at,
        }
    }

    #[test]
    fn a_new_session_gets_both_expiries() {
        let expiry = POLICY.open(now());

        assert_eq!(expiry.expires_at, now() + POLICY.idle);
        assert_eq!(expiry.absolute_expires_at, now() + POLICY.absolute);
    }

    #[test]
    fn the_idle_expiry_never_passes_the_absolute_one() {
        let short = SessionPolicy {
            idle: 48 * HOUR,
            absolute: 24 * HOUR,
        };

        let expiry = short.open(now());

        assert_eq!(expiry.expires_at, expiry.absolute_expires_at);
    }

    #[test]
    fn use_slides_the_idle_expiry_at_most_once_a_minute() {
        let session = session(POLICY);

        assert_eq!(
            POLICY.touch(&session, now() + Duration::from_secs(59)),
            None
        );
        assert_eq!(
            POLICY.touch(&session, now() + HOUR),
            Some(now() + HOUR + POLICY.idle)
        );
        // Near the end, the absolute expiry caps the slide.
        let late = now() + POLICY.absolute - HOUR;
        assert_eq!(
            POLICY.touch(&session, late),
            Some(session.absolute_expires_at)
        );
    }

    #[test]
    fn a_session_expires_at_either_deadline() {
        let mut session = session(POLICY);

        assert!(!session.is_expired(now()));
        assert!(session.is_expired(now() + POLICY.idle));
        session.expires_at = now() + POLICY.absolute + HOUR;
        assert!(!session.is_expired(now() + POLICY.absolute - HOUR));
        assert!(session.is_expired(now() + POLICY.absolute));
    }

    #[test]
    fn user_agents_are_cut_at_a_character_boundary() {
        let long = format!("{}é", "a".repeat(MAX_USER_AGENT - 1));

        assert_eq!(truncate_user_agent("curl/8"), "curl/8");
        assert_eq!(truncate_user_agent(&long), "a".repeat(MAX_USER_AGENT - 1));
    }
}
