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
        /// A mail for each notification still unread a little while after it came; those
        /// that come close together share one.
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

/// The most characters of a block's id that a notification, or a page's record of whom its
/// text mentions, keeps: a mention in a block of a longer id is kept and told without it.
pub const MAX_BLOCK_ID: usize = 128;

/// A mention in a page's text: of whom, in which block, and who wrote it, when the server
/// knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMention {
    pub user: Uuid,
    pub block_id: Option<String>,
    pub writer: Option<Uuid>,
}

/// Whom a mention in a page's text is told it is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credit {
    /// Someone other than the one it names wrote it.
    Writer(Uuid),
    /// Nobody knows who wrote it, as of text the server does not know the writer of.
    Unknown,
    /// The one it names wrote every mention of them: it tells nobody.
    Themselves,
}

impl Credit {
    fn of(mention: &TextMention) -> Self {
        match mention.writer {
            Some(writer) if writer == mention.user => Self::Themselves,
            Some(writer) => Self::Writer(writer),
            None => Self::Unknown,
        }
    }

    /// Which credit wins among a person's mentions: a writer's, then an unknown one.
    fn rank(self) -> u8 {
        match self {
            Self::Writer(_) => 2,
            Self::Unknown => 1,
            Self::Themselves => 0,
        }
    }
}

/// Someone a page's text mentions, the block it is in, and whom it is from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageMention {
    pub user: Uuid,
    pub block_id: Option<String>,
    pub credit: Credit,
}

/// Whom a page's text mentions, each once, in the order of their first mention: credited to
/// the first who wrote a mention of them and is not them, then, failing one, to nobody known
/// when some mention of them has no known writer, and to themselves only when they wrote
/// every one. The block is that of the mention credited, kept only while its id is at most
/// [`MAX_BLOCK_ID`] characters.
pub fn page_mentions(found: impl IntoIterator<Item = TextMention>) -> Vec<PageMention> {
    let mut mentioned: Vec<PageMention> = Vec::new();
    for mention in found {
        let credit = Credit::of(&mention);
        let block_id = mention
            .block_id
            .filter(|id| id.chars().count() <= MAX_BLOCK_ID);
        match mentioned
            .iter_mut()
            .find(|known| known.user == mention.user)
        {
            Some(known) if credit.rank() > known.credit.rank() => {
                known.credit = credit;
                known.block_id = block_id;
            }
            Some(_) => {}
            None => mentioned.push(PageMention {
                user: mention.user,
                block_id,
                credit,
            }),
        }
    }
    mentioned
}

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

    fn written(user: Uuid, block: &str, writer: Option<Uuid>) -> TextMention {
        TextMention {
            user,
            block_id: Some(block.to_owned()),
            writer,
        }
    }

    #[test]
    fn a_mention_in_a_pages_text_is_from_whoever_wrote_it() {
        let mentioned = page_mentions([
            // Bea's own mention of herself, then Ada's of her: Ada's counts.
            written(BEA, "b1", Some(BEA)),
            written(CY, "b2", Some(ADA)),
            written(BEA, "b3", Some(ADA)),
            // Cy again, by Dee: the first writer stays.
            written(CY, "b4", Some(DEE)),
            // Dee, by nobody known, then by herself: unknown beats her own.
            written(DEE, "b5", None),
            written(DEE, "b6", Some(DEE)),
            // Ada, by herself alone.
            written(ADA, "b7", Some(ADA)),
        ]);
        assert_eq!(
            mentioned,
            [
                PageMention {
                    user: BEA,
                    block_id: Some("b3".into()),
                    credit: Credit::Writer(ADA),
                },
                PageMention {
                    user: CY,
                    block_id: Some("b2".into()),
                    credit: Credit::Writer(ADA),
                },
                PageMention {
                    user: DEE,
                    block_id: Some("b5".into()),
                    credit: Credit::Unknown,
                },
                PageMention {
                    user: ADA,
                    block_id: Some("b7".into()),
                    credit: Credit::Themselves,
                },
            ]
        );
    }

    #[test]
    fn a_block_id_too_long_to_keep_is_left_out_and_the_mention_kept() {
        let long = "L".repeat(MAX_BLOCK_ID + 1);
        let kept = "é".repeat(MAX_BLOCK_ID);
        let mentioned = page_mentions([
            written(BEA, &long, Some(ADA)),
            written(CY, &kept, Some(ADA)),
        ]);
        assert_eq!(mentioned[0].block_id, None);
        assert_eq!(mentioned[0].credit, Credit::Writer(ADA));
        assert_eq!(mentioned[1].block_id.as_deref(), Some(kept.as_str()));
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
