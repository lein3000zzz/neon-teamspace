//! Requests to join: someone without an invite asks for one while registration is by
//! invite. They confirm their address by a mailed link first; only then do admins see the
//! request, and approve it, which mails a single-use invite bound to that address, or deny
//! it.

use std::fmt;
use std::time::Duration;

use time::OffsetDateTime;
use uuid::Uuid;

/// How long a confirmation link works.
pub const LINK_TTL: Duration = Duration::from_secs(24 * 3600);

/// The most confirmation links mailed for one request.
pub const MAX_MAILS: i32 = 5;

/// The least time between two confirmation links for one request.
pub const MAIL_GAP: Duration = Duration::from_secs(60);

/// How long a denied address waits before it may ask again.
pub const DENIED_WAIT: Duration = Duration::from_secs(30 * 24 * 3600);

/// The most requests confirmed and waiting for the admins at once on the instance. Past it,
/// confirming one more is refused until the admins decide some, so the list they go through
/// stays bounded. Requests nobody confirmed do not count: whoever can only send the form
/// cannot fill it.
pub const MAX_OPEN: i64 = 200;

/// The most requests nobody confirmed yet from one source, an IPv4 address or an IPv6 /64,
/// at once. Past it, new requests from the source are dropped, so the form cannot fill the
/// table, and a source sending junk takes no room from anyone else.
pub const MAX_UNCONFIRMED_PER_SOURCE: i64 = 10;

/// The longest note, in characters.
pub const MAX_NOTE: usize = 500;

/// How long decided requests stay listed, and kept: a denied one blocks its address that
/// long, then the hourly cleanup removes it.
pub const RETENTION: Duration = DENIED_WAIT;

/// Where a request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRequestStatus {
    /// Its link was mailed and not opened yet. Admins do not see it.
    Unconfirmed,
    /// Confirmed, waiting for an admin.
    Pending,
    Approved,
    Denied,
}

impl JoinRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unconfirmed => "unconfirmed",
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }
}

impl fmt::Display for JoinRequestStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A request to join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequest {
    pub id: Uuid,
    /// The address as typed.
    pub email: String,
    /// Theirs, plain text, at most [`MAX_NOTE`] characters.
    pub note: String,
    pub created_at: OffsetDateTime,
    pub confirmed_at: Option<OffsetDateTime>,
    pub decided_at: Option<OffsetDateTime>,
    pub decided_by: Option<Uuid>,
    /// Set once decided.
    pub approved: Option<bool>,
    /// The invite an approval made.
    pub invite_id: Option<Uuid>,
    /// When the last confirmation link stops working.
    pub link_expires_at: OffsetDateTime,
    /// How many confirmation links were mailed, and when the last one was.
    pub mails: i32,
    pub mailed_at: OffsetDateTime,
}

impl JoinRequest {
    pub fn status(&self) -> JoinRequestStatus {
        match (self.decided_at, self.confirmed_at, self.approved) {
            (None, None, _) => JoinRequestStatus::Unconfirmed,
            (None, Some(_), _) => JoinRequestStatus::Pending,
            (Some(_), _, Some(true)) => JoinRequestStatus::Approved,
            (Some(_), _, _) => JoinRequestStatus::Denied,
        }
    }

    /// Whether asking again mails a new link: the request is unconfirmed, fewer than
    /// [`MAX_MAILS`] went, and the last was at least [`MAIL_GAP`] ago.
    pub fn may_mail_again(&self, now: OffsetDateTime) -> bool {
        self.status() == JoinRequestStatus::Unconfirmed
            && self.mails < MAX_MAILS
            && now - self.mailed_at >= MAIL_GAP
    }
}

/// A note that cannot be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a note has at most {MAX_NOTE} characters, and no NUL characters")]
pub struct InvalidNote;

/// The note to keep, trimmed.
pub fn check_note(raw: &str) -> Result<String, InvalidNote> {
    let note = raw.trim();
    if note.chars().count() > MAX_NOTE || note.contains('\0') {
        Err(InvalidNote)
    } else {
        Ok(note.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn request() -> JoinRequest {
        JoinRequest {
            id: Uuid::now_v7(),
            email: "ada@example.com".into(),
            note: String::new(),
            created_at: now(),
            confirmed_at: None,
            decided_at: None,
            decided_by: None,
            approved: None,
            invite_id: None,
            link_expires_at: now() + LINK_TTL,
            mails: 1,
            mailed_at: now(),
        }
    }

    #[test]
    fn status_follows_confirmation_then_decision() {
        let confirmed = JoinRequest {
            confirmed_at: Some(now()),
            ..request()
        };
        let approved = JoinRequest {
            decided_at: Some(now()),
            approved: Some(true),
            ..confirmed.clone()
        };
        let denied = JoinRequest {
            approved: Some(false),
            ..approved.clone()
        };

        assert_eq!(request().status(), JoinRequestStatus::Unconfirmed);
        assert_eq!(confirmed.status(), JoinRequestStatus::Pending);
        assert_eq!(approved.status(), JoinRequestStatus::Approved);
        assert_eq!(denied.status(), JoinRequestStatus::Denied);
    }

    #[test]
    fn links_are_mailed_again_at_most_once_a_minute_and_five_times() {
        let minute = Duration::from_secs(60);

        assert!(!request().may_mail_again(now() + Duration::from_secs(59)));
        assert!(request().may_mail_again(now() + minute));
        let fifth = JoinRequest {
            mails: MAX_MAILS,
            ..request()
        };
        assert!(!fifth.may_mail_again(now() + LINK_TTL));
        let pending = JoinRequest {
            confirmed_at: Some(now()),
            ..request()
        };
        assert!(!pending.may_mail_again(now() + minute));
    }

    #[test]
    fn notes_are_trimmed_and_bounded() {
        assert_eq!(check_note("  hello  "), Ok("hello".into()));
        assert_eq!(check_note(&"é".repeat(MAX_NOTE)), Ok("é".repeat(MAX_NOTE)));
        assert_eq!(check_note(&"a".repeat(MAX_NOTE + 1)), Err(InvalidNote));
        assert_eq!(check_note("nul\0byte"), Err(InvalidNote));
    }
}
