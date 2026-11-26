//! Who hears of what is written and assigned: the mentions in a page's text once it is
//! stored, each from whoever wrote it, the mentions, replies and comments of a comment, and
//! the people a row's change assigns, each in the transaction that stores it.

use std::collections::HashMap;

use neon_blocks::{Document, Inline, Mention, MentionKind};
use neon_domain::databases::Schema;
use neon_domain::notifications::{
    CommentAudience, Credit, NotificationKind, PageMention, TextMention, comment_audience,
};
use neon_domain::webhooks::EventType;
use neon_store::comments::{self as comments_repo, Comment, Thread};
use neon_store::notifications::{self as repo, Notification};
use neon_store::pages as pages_repo;
use serde_json::Value as Json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use super::notify::{Notifier, sees};
use crate::pages::{PageError, PageService};
use crate::webhooks::events::{self as webhooks, PageChange};

/// The people, agents and integrations a comment's markdown mentions, each once, in order.
pub(crate) fn mentioned_in(body: &str) -> Vec<Uuid> {
    mentioned_in_content(&neon_blocks::markdown::inline_from_markdown(body))
}

/// The people, agents and integrations inline content mentions, each once, in order.
pub(crate) fn mentioned_in_content(content: &[Inline]) -> Vec<Uuid> {
    let mut found = Vec::new();
    for run in content {
        if let Inline::Mention(mention) = run
            && let Some(user) = mentioned_user(mention)
            && !found.contains(&user)
        {
            found.push(user);
        }
    }
    found
}

/// Whom a mention names, when it names a person, an agent or an integration.
pub(crate) fn mentioned_user(mention: &Mention) -> Option<Uuid> {
    (mention.props.kind == MentionKind::User)
        .then(|| Uuid::try_parse(&mention.props.target).ok())
        .flatten()
}

/// The mentions of a document's text that name people, agents and integrations, in order,
/// each with its block, whoever wrote them not known.
pub(crate) fn in_document(document: &Document) -> Vec<TextMention> {
    document
        .mentions()
        .into_iter()
        .filter_map(|(block, mention)| {
            Some(TextMention {
                user: mentioned_user(mention)?,
                block_id: block.map(str::to_owned),
                writer: None,
            })
        })
        .collect()
}

/// A page's text as a snapshot stores it: whom it mentions, each with their block and whom
/// their mention is from, and when.
pub(crate) struct Mentions<'a> {
    pub(crate) page_id: Uuid,
    pub(crate) mentioned: &'a [PageMention],
    pub(crate) at: OffsetDateTime,
}

/// Records whom a page's text mentions now, telling nobody, as an import of the text does:
/// those it did not mention before, each with their block.
pub(crate) async fn record(
    conn: &mut PgConnection,
    page_id: Uuid,
    mentioned: &[PageMention],
) -> sqlx::Result<Vec<(Uuid, Option<String>)>> {
    let pairs: Vec<(Uuid, Option<String>)> = mentioned
        .iter()
        .map(|mention| (mention.user, mention.block_id.clone()))
        .collect();
    comments_repo::set_page_mentions(&mut *conn, page_id, &pairs).await
}

/// Records whom a page's text mentions now, and tells each it mentions for the first time,
/// who may see the page, but those who wrote every mention of themselves: the
/// `page.mentioned` event, and a notification from the mention's writer unless one of a
/// mention on the page waits unread. They follow the page's comments from then on, unless
/// they stopped.
pub(crate) async fn page_text(
    conn: &mut PgConnection,
    pages: &PageService,
    notifier: &mut Notifier,
    mentions: Mentions<'_>,
) -> sqlx::Result<()> {
    let new = record(&mut *conn, mentions.page_id, mentions.mentioned).await?;
    if new.is_empty() {
        return Ok(());
    }
    let Some(page) = pages_repo::get(&mut *conn, mentions.page_id).await? else {
        return Ok(());
    };
    let mut followers = Vec::new();
    for (user, block_id) in new {
        let credit = mentions
            .mentioned
            .iter()
            .find(|mention| mention.user == user)
            .map_or(Credit::Unknown, |mention| mention.credit);
        let actor = match credit {
            Credit::Themselves => continue,
            Credit::Writer(writer) => Some(writer),
            Credit::Unknown => None,
        };
        if !sees(pages, &mut *conn, user, page.workspace_id, page.id)
            .await
            .map_err(into_sqlx)?
        {
            continue;
        }
        if !repo::has_unread(&mut *conn, user, NotificationKind::Mention, page.id, None).await? {
            let mut notification = Notification::new(
                user,
                page.workspace_id,
                NotificationKind::Mention,
                mentions.at,
            );
            notification.actor_id = actor;
            notification.page_id = Some(page.id);
            notification.block_id = block_id;
            notifier.record(&mut *conn, &notification).await?;
        }
        webhooks::queue_page(
            &mut *conn,
            PageChange {
                kind: EventType::Mentioned,
                workspace_id: page.workspace_id,
                page_id: page.id,
                identity: user,
                actor,
                thread_id: None,
                comment_id: None,
                at: mentions.at,
            },
        )
        .await?;
        followers.push(user);
    }
    comments_repo::follow_unless_chosen(
        &mut *conn,
        page.workspace_id,
        page.id,
        &followers,
        mentions.at,
    )
    .await
}

/// A comment just stored, as its notifications need it.
pub(crate) struct Written<'a> {
    pub(crate) thread: &'a Thread,
    pub(crate) comment: &'a Comment,
    /// The thread's comments before it, for a new comment; none for an edit.
    pub(crate) earlier: &'a [Comment],
    /// Those the comment mentioned before an edit, who are told nothing again.
    pub(crate) mentioned_before: &'a [Uuid],
    /// An edit tells only those it mentions anew.
    pub(crate) edit: bool,
}

/// Who a comment told, and whom it mentions that may not see its page, who are told
/// nothing.
#[derive(Debug, Default)]
pub(crate) struct Told {
    pub(crate) unseen: Vec<Uuid>,
}

/// Tells of a comment on the transaction that stores it, as
/// [`comment_audience`] says, each person once and only while they may see the page:
/// those it mentions, those in its thread, by writing in it or being mentioned in it, and
/// the page's followers. Each one it mentions gets the `page.mentioned` event, and each who
/// wrote earlier in the thread `comment.replied`. Its author and those it mentions who may
/// see the page follow the page from then on, unless they stopped.
pub(crate) async fn comment(
    conn: &mut PgConnection,
    pages: &PageService,
    notifier: &mut Notifier,
    written: Written<'_>,
) -> Result<Told, PageError> {
    let thread = written.thread;
    let comment = written.comment;
    let Some(author) = comment.author_id else {
        return Ok(Told::default());
    };
    let (workspace_id, page_id) = (thread.workspace_id, thread.page_id);
    let mut seen: HashMap<Uuid, bool> = HashMap::new();
    let mut may_see = async |conn: &mut PgConnection, user: Uuid| -> Result<bool, PageError> {
        if let Some(known) = seen.get(&user) {
            return Ok(*known);
        }
        let sees = sees(pages, conn, user, workspace_id, page_id).await?;
        seen.insert(user, sees);
        Ok(sees)
    };

    let mut told = Told::default();
    let mut mentioned = Vec::new();
    for user in mentioned_in(&comment.body) {
        if user == author || written.mentioned_before.contains(&user) {
            continue;
        }
        if may_see(&mut *conn, user).await? {
            mentioned.push(user);
        } else {
            told.unseen.push(user);
        }
    }
    let (mut in_thread, mut repliers) = (Vec::new(), Vec::new());
    for earlier in written.earlier {
        if let Some(writer) = earlier.author_id {
            in_thread.push(writer);
            repliers.push(writer);
        }
        in_thread.extend(mentioned_in(&earlier.body));
    }
    let followers = if written.edit {
        Vec::new()
    } else {
        comments_repo::followers(&mut *conn, page_id).await?
    };
    let audience = comment_audience(CommentAudience {
        author,
        mentioned: &mentioned,
        in_thread: if written.edit { &[] } else { &in_thread },
        followers: &followers,
    });
    // An edit tells as of when it was made, and its mail waits from then.
    let at = comment.edited_at.unwrap_or(comment.created_at);
    for (user, kind) in audience {
        if kind != NotificationKind::Mention && !may_see(&mut *conn, user).await? {
            continue;
        }
        let mut notification = Notification::new(user, workspace_id, kind, at);
        notification.actor_id = Some(author);
        notification.page_id = Some(page_id);
        notification.thread_id = Some(thread.id);
        notification.comment_id = Some(comment.id);
        notifier.record(&mut *conn, &notification).await?;
    }
    let event = |kind, identity| PageChange {
        kind,
        workspace_id,
        page_id,
        identity,
        actor: Some(author),
        thread_id: Some(thread.id),
        comment_id: Some(comment.id),
        at,
    };
    for user in &mentioned {
        webhooks::queue_page(&mut *conn, event(EventType::Mentioned, *user)).await?;
    }
    repliers.sort_unstable();
    repliers.dedup();
    for user in repliers.into_iter().filter(|user| *user != author) {
        if !written.edit && may_see(&mut *conn, user).await? {
            webhooks::queue_page(&mut *conn, event(EventType::Replied, user)).await?;
        }
    }
    let mut following = vec![author];
    following.extend(&mentioned);
    comments_repo::follow_unless_chosen(&mut *conn, workspace_id, page_id, &following, at).await?;
    Ok(told)
}

/// A change to a row's values, as the database service makes it.
pub(crate) struct Assignment<'a> {
    pub(crate) workspace_id: Uuid,
    pub(crate) row_id: Uuid,
    pub(crate) actor: Uuid,
    pub(crate) schema: &'a Schema,
    /// The row's cells before, `None` for a new row.
    pub(crate) before: Option<&'a Json>,
    pub(crate) after: &'a Json,
    pub(crate) at: OffsetDateTime,
}

/// Tells each identity a change adds to a person property of a row, who may see the row,
/// but whoever made the change: a notification for each property, unless one of the same
/// waits unread.
pub(crate) async fn assignment(
    conn: &mut PgConnection,
    pages: &PageService,
    notifier: &mut Notifier,
    change: Assignment<'_>,
) -> Result<(), PageError> {
    for (identity, property) in webhooks::assignments(change.schema, change.before, change.after) {
        if identity == change.actor
            || !sees(
                pages,
                &mut *conn,
                identity,
                change.workspace_id,
                change.row_id,
            )
            .await?
            || repo::has_unread(
                &mut *conn,
                identity,
                NotificationKind::Assigned,
                change.row_id,
                Some(property),
            )
            .await?
        {
            continue;
        }
        let mut notification = Notification::new(
            identity,
            change.workspace_id,
            NotificationKind::Assigned,
            change.at,
        );
        notification.actor_id = Some(change.actor);
        notification.page_id = Some(change.row_id);
        notification.property_id = Some(property);
        notifier.record(&mut *conn, &notification).await?;
    }
    Ok(())
}

/// A page use case's failure as the database's, for the snapshot's transaction, whose
/// other steps fail so.
fn into_sqlx(error: PageError) -> sqlx::Error {
    match error {
        PageError::Database(error) => error,
        other => sqlx::Error::Protocol(other.to_string()),
    }
}
