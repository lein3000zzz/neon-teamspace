//! The inbox: what someone is told of, who is told of a comment, and how they want mail of
//! it.
//!
//! A notification is made in the transaction of what it tells of, once per cause: a comment
//! tells each person once, whichever of the rules below reaches them first, and a page's
//! text tells someone it mentions once their mention is stored. Whether they may still see
//! what it is about is asked again whenever it is shown.

use uuid::Uuid;

text_enum! {
    /// What a notification tells of.
    NotificationKind {
        /// A page's text, or a comment on it, mentions the person.
        Mention => "mention",
        /// Someone replied in a thread the person wrote in or was mentioned in.
        Reply => "reply",
        /// Someone commented on a page the person follows: one they made, commented on or
        /// were mentioned on, unless they stopped following it, or one they chose to follow.
        Comment => "comment",
        /// The person was added to a person property of a row.
        Assigned => "assigned",
        /// A workspace invites the person to join it.
        Invitation => "invitation",
    }
}

text_enum! {
    /// How someone wants mail of their inbox, while the instance sends it.
    MailPreference {
        /// No mail.
        None => "none",
        /// A mail for each notification still unread a little while after it came.
        Each => "each",
        /// One mail every few hours, of what came and is still unread.
        Digest => "digest",
    }
}

/// The preference of someone who has not chosen one.
pub const DEFAULT_MAIL: MailPreference = MailPreference::Each;

/// The most characters of a comment a notification shows.
pub const MAX_EXCERPT: usize = 200;

/// The most notifications a digest lists; it says how many more there are.
pub const MAX_DIGEST: usize = 20;

/// What a new comment's notifications need to know: who wrote it, whom it mentions, who
/// is in its thread already, by writing in it or being mentioned in it, and who follows
/// the page.
#[derive(Debug, Clone, Copy)]
pub struct CommentAudience<'a> {
    pub author: Uuid,
    pub mentioned: &'a [Uuid],
    pub in_thread: &'a [Uuid],
    pub followers: &'a [Uuid],
}

/// Whom a comment tells, and why, each person once, in that order: those it mentions; then
/// those in its thread; then the page's followers. Never its author.
pub fn comment_audience(audience: CommentAudience<'_>) -> Vec<(Uuid, NotificationKind)> {
    let mut told: Vec<(Uuid, NotificationKind)> = Vec::new();
    let groups = [
        (audience.mentioned, NotificationKind::Mention),
        (audience.in_thread, NotificationKind::Reply),
        (audience.followers, NotificationKind::Comment),
    ];
    for (people, kind) in groups {
        for person in people {
            if *person != audience.author && !told.iter().any(|(id, _)| id == person) {
                told.push((*person, kind));
            }
        }
    }
    told
}

/// An excerpt of a comment's text for a notification: its first [`MAX_EXCERPT`]
/// characters on one line, with an ellipsis when it was cut.
pub fn excerpt(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_EXCERPT {
        return flat;
    }
    let mut cut: String = flat.chars().take(MAX_EXCERPT - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: Uuid = Uuid::from_u128(1);
    const BEA: Uuid = Uuid::from_u128(2);
    const CY: Uuid = Uuid::from_u128(3);
    const DEE: Uuid = Uuid::from_u128(4);

    #[test]
    fn a_comment_tells_each_person_once_by_the_first_rule_that_reaches_them() {
        let told = comment_audience(CommentAudience {
            author: ADA,
            mentioned: &[BEA, ADA],
            in_thread: &[ADA, BEA, CY],
            followers: &[CY, DEE, ADA],
        });
        assert_eq!(
            told,
            [
                (BEA, NotificationKind::Mention),
                (CY, NotificationKind::Reply),
                (DEE, NotificationKind::Comment),
            ]
        );
    }

    #[test]
    fn nobody_is_told_of_their_own_comment() {
        let told = comment_audience(CommentAudience {
            author: ADA,
            mentioned: &[ADA],
            in_thread: &[ADA],
            followers: &[ADA],
        });
        assert!(told.is_empty());
    }

    #[test]
    fn excerpts_are_one_short_line() {
        assert_eq!(excerpt("  two\n\nlines  "), "two lines");
        let long = excerpt(&"word ".repeat(100));
        assert_eq!(long.chars().count(), MAX_EXCERPT);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn kinds_and_preferences_read_back() {
        for kind in NotificationKind::ALL {
            assert_eq!(NotificationKind::parse(kind.as_str()), Some(*kind));
        }
        assert_eq!(
            MailPreference::parse("digest"),
            Some(MailPreference::Digest)
        );
        assert_eq!(DEFAULT_MAIL, MailPreference::Each);
    }
}
