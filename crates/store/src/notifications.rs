//! The inbox: notifications, and how each person wants mail of them.

use neon_domain::notifications::{MailPreference, NotificationKind};
use neon_domain::workspaces::Person;
use sqlx::PgExecutor;
use sqlx::postgres::types::PgInterval;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode;
use crate::users::PersonColumns;

/// A notification as made, with what it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: Uuid,
    pub user_id: Uuid,
    pub workspace_id: Uuid,
    pub kind: NotificationKind,
    pub actor_id: Option<Uuid>,
    pub page_id: Option<Uuid>,
    pub thread_id: Option<Uuid>,
    pub comment_id: Option<Uuid>,
    pub block_id: Option<String>,
    /// For a mention in a page's text, the text of its block when it was told.
    pub excerpt: Option<String>,
    pub property_id: Option<Uuid>,
    pub invitation_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub read_at: Option<OffsetDateTime>,
    pub archived_at: Option<OffsetDateTime>,
    pub mailed_at: Option<OffsetDateTime>,
}

impl Notification {
    /// A new notification, unread, of `kind`, for `user_id` in `workspace_id`.
    pub fn new(
        user_id: Uuid,
        workspace_id: Uuid,
        kind: NotificationKind,
        at: OffsetDateTime,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            user_id,
            workspace_id,
            kind,
            actor_id: None,
            page_id: None,
            thread_id: None,
            comment_id: None,
            block_id: None,
            excerpt: None,
            property_id: None,
            invitation_id: None,
            created_at: at,
            read_at: None,
            archived_at: None,
            mailed_at: None,
        }
    }
}

/// A notification as an inbox lists it: with who did what it tells of, and the comment it
/// is about, as they are now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub notification: Notification,
    pub actor: Option<Person>,
    pub comment_body: Option<String>,
}

/// Whose inbox, and how much of it: one workspace's notifications or every workspace's,
/// and whether with the person's invitations into workspaces while those wait for an
/// answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inbox {
    pub user_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub invitations: bool,
}

/// Which of an inbox's notifications to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Filter {
    /// Only those not read yet.
    pub unread: bool,
    /// The archived ones, rather than the others.
    pub archived: bool,
}

/// Saves a notification, unless its person has one of the same comment already. Says
/// whether it did.
pub async fn insert(
    executor: impl PgExecutor<'_>,
    notification: &Notification,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "INSERT INTO notifications (id, user_id, workspace_id, kind, actor_id, page_id, thread_id,
                                    comment_id, block_id, excerpt, property_id, invitation_id,
                                    created_at, read_at, archived_at, mailed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
         ON CONFLICT (user_id, comment_id) WHERE comment_id IS NOT NULL DO NOTHING",
        notification.id,
        notification.user_id,
        notification.workspace_id,
        notification.kind.as_str(),
        notification.actor_id,
        notification.page_id,
        notification.thread_id,
        notification.comment_id,
        notification.block_id,
        notification.excerpt,
        notification.property_id,
        notification.invitation_id,
        notification.created_at,
        notification.read_at,
        notification.archived_at,
        notification.mailed_at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Whether someone has a notification of `kind` about a page, not read nor archived yet, and
/// for an assignment of the same property.
pub async fn has_unread(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    kind: NotificationKind,
    page_id: Uuid,
    property_id: Option<Uuid>,
) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM notifications
               WHERE user_id = $1 AND kind = $2 AND page_id = $3 AND comment_id IS NULL
                 AND property_id IS NOT DISTINCT FROM $4
                 AND read_at IS NULL AND archived_at IS NULL
           ) AS "found!""#,
        user_id,
        kind.as_str(),
        page_id,
        property_id,
    )
    .fetch_one(executor)
    .await
}

struct Row {
    id: Uuid,
    user_id: Uuid,
    workspace_id: Uuid,
    kind: String,
    actor_id: Option<Uuid>,
    page_id: Option<Uuid>,
    thread_id: Option<Uuid>,
    comment_id: Option<Uuid>,
    block_id: Option<String>,
    excerpt: Option<String>,
    property_id: Option<Uuid>,
    invitation_id: Option<Uuid>,
    created_at: OffsetDateTime,
    read_at: Option<OffsetDateTime>,
    archived_at: Option<OffsetDateTime>,
    mailed_at: Option<OffsetDateTime>,
}

impl TryFrom<Row> for Notification {
    type Error = sqlx::Error;

    fn try_from(row: Row) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.id,
            user_id: row.user_id,
            workspace_id: row.workspace_id,
            kind: decode("notification", &row.kind, NotificationKind::parse)?,
            actor_id: row.actor_id,
            page_id: row.page_id,
            thread_id: row.thread_id,
            comment_id: row.comment_id,
            block_id: row.block_id,
            excerpt: row.excerpt,
            property_id: row.property_id,
            invitation_id: row.invitation_id,
            created_at: row.created_at,
            read_at: row.read_at,
            archived_at: row.archived_at,
            mailed_at: row.mailed_at,
        })
    }
}

/// A notification by its id, when it is `user_id`'s.
pub async fn get(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Notification>> {
    sqlx::query_as!(
        Row,
        "SELECT id, user_id, workspace_id, kind, actor_id, page_id, thread_id, comment_id,
                block_id, excerpt, property_id, invitation_id, created_at, read_at, archived_at,
                mailed_at
         FROM notifications WHERE id = $1 AND user_id = $2",
        id,
        user_id,
    )
    .fetch_optional(executor)
    .await?
    .map(Notification::try_from)
    .transpose()
}

/// The notifications of `ids` that are `user_id`'s, in no order.
pub async fn by_ids(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    ids: &[Uuid],
) -> sqlx::Result<Vec<Notification>> {
    sqlx::query_as!(
        Row,
        "SELECT id, user_id, workspace_id, kind, actor_id, page_id, thread_id, comment_id,
                block_id, excerpt, property_id, invitation_id, created_at, read_at, archived_at,
                mailed_at
         FROM notifications WHERE id = ANY($1) AND user_id = $2",
        ids,
        user_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Notification::try_from)
    .collect()
}

/// An inbox's notifications, newest first, before `before` when given, at most `limit`, as
/// of `now`, which decides which invitations still wait.
pub async fn list(
    executor: impl PgExecutor<'_>,
    inbox: Inbox,
    filter: Filter,
    before: Option<Uuid>,
    limit: i64,
    now: OffsetDateTime,
) -> sqlx::Result<Vec<Listed>> {
    sqlx::query!(
        r#"SELECT n.id, n.user_id, n.workspace_id, n.kind, n.actor_id, n.page_id, n.thread_id,
                  n.comment_id, n.block_id, n.excerpt, n.property_id, n.invitation_id,
                  n.created_at, n.read_at, n.archived_at, n.mailed_at,
                  a.kind AS "actor_kind?", a.display_name AS "actor_name?", a.email AS actor_email,
                  a.hue AS actor_hue, o.id AS "owner_id?", o.display_name AS "owner_name?",
                  o.hue AS owner_hue, c.body AS "comment_body?"
           FROM notifications n
           LEFT JOIN users a ON a.id = n.actor_id
           LEFT JOIN users o ON o.id = a.owner_id
           LEFT JOIN comments c ON c.id = n.comment_id
           LEFT JOIN workspace_invitations i ON i.id = n.invitation_id
           WHERE n.user_id = $1
             AND CASE WHEN n.kind = 'invitation'
                      THEN $3 AND i.state = 'pending' AND i.expires_at > $4
                      ELSE $2::uuid IS NULL OR n.workspace_id = $2 END
             AND (NOT $5 OR n.read_at IS NULL)
             AND (n.archived_at IS NOT NULL) = $6
             AND ($7::uuid IS NULL OR n.id < $7)
           ORDER BY n.id DESC
           LIMIT $8"#,
        inbox.user_id,
        inbox.workspace_id,
        inbox.invitations,
        now,
        filter.unread,
        filter.archived,
        before,
        limit,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let actor = match (row.actor_id, row.actor_kind, row.actor_name) {
            (Some(id), Some(kind), Some(display_name)) => Some(Person::try_from(PersonColumns {
                id,
                kind,
                display_name,
                email: row.actor_email,
                hue: row.actor_hue,
                owner_id: row.owner_id,
                owner_name: row.owner_name,
                owner_hue: row.owner_hue,
            })?),
            _ => None,
        };
        Ok(Listed {
            notification: Notification::try_from(Row {
                id: row.id,
                user_id: row.user_id,
                workspace_id: row.workspace_id,
                kind: row.kind,
                actor_id: row.actor_id,
                page_id: row.page_id,
                thread_id: row.thread_id,
                comment_id: row.comment_id,
                block_id: row.block_id,
                excerpt: row.excerpt,
                property_id: row.property_id,
                invitation_id: row.invitation_id,
                created_at: row.created_at,
                read_at: row.read_at,
                archived_at: row.archived_at,
                mailed_at: row.mailed_at,
            })?,
            actor,
            comment_body: row.comment_body,
        })
    })
    .collect()
}

/// An inbox's unread notifications, at most `limit`: each one's id and page, for the
/// count.
pub async fn unread(
    executor: impl PgExecutor<'_>,
    inbox: Inbox,
    limit: i64,
    now: OffsetDateTime,
) -> sqlx::Result<Vec<(Uuid, Option<Uuid>)>> {
    Ok(sqlx::query!(
        "SELECT n.id, n.page_id FROM notifications n
         LEFT JOIN workspace_invitations i ON i.id = n.invitation_id
         WHERE n.user_id = $1 AND n.read_at IS NULL AND n.archived_at IS NULL
           AND CASE WHEN n.kind = 'invitation'
                    THEN $3 AND i.state = 'pending' AND i.expires_at > $4
                    ELSE $2::uuid IS NULL OR n.workspace_id = $2 END
         ORDER BY n.id DESC
         LIMIT $5",
        inbox.user_id,
        inbox.workspace_id,
        inbox.invitations,
        now,
        limit,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| (row.id, row.page_id))
    .collect())
}

/// Marks someone's notification read or unread, archived or not, as given. Says whether it
/// is theirs.
pub async fn mark(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    id: Uuid,
    read: Option<bool>,
    archived: Option<bool>,
    at: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE notifications SET
             read_at = CASE WHEN $3::boolean IS NULL THEN read_at
                            WHEN $3 THEN COALESCE(read_at, $5) END,
             archived_at = CASE WHEN $4::boolean IS NULL THEN archived_at
                                WHEN $4 THEN COALESCE(archived_at, $5) END
         WHERE id = $1 AND user_id = $2",
        id,
        user_id,
        read,
        archived,
        at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Marks `user_id`'s notifications of `ids` read at `at`, or unread, as [`mark`] marks one:
/// one read already keeps the time it was read.
pub async fn mark_read_by_ids(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    ids: &[Uuid],
    read: bool,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE notifications SET read_at = CASE WHEN $3 THEN COALESCE(read_at, $4) END
         WHERE id = ANY($1) AND user_id = $2",
        ids,
        user_id,
        read,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Marks read every unread notification of an inbox, its invitations those that still
/// wait at `at`. Says how many.
pub async fn mark_all_read(
    executor: impl PgExecutor<'_>,
    inbox: Inbox,
    at: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE notifications n SET read_at = $4
         WHERE n.user_id = $1 AND n.read_at IS NULL
           AND CASE WHEN n.kind = 'invitation'
                    THEN $3 AND EXISTS (SELECT 1 FROM workspace_invitations i
                                        WHERE i.id = n.invitation_id AND i.state = 'pending'
                                          AND i.expires_at > $4)
                    ELSE $2::uuid IS NULL OR n.workspace_id = $2 END",
        inbox.user_id,
        inbox.workspace_id,
        inbox.invitations,
        at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Whose notifications are of a comment: the inboxes its deletion changes.
pub async fn told_of_comment(
    executor: impl PgExecutor<'_>,
    comment_id: Uuid,
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        "SELECT DISTINCT user_id FROM notifications WHERE comment_id = $1",
        comment_id,
    )
    .fetch_all(executor)
    .await
}

/// Whose notifications are of a thread or a comment in it: the inboxes its deletion
/// changes.
pub async fn told_of_thread(
    executor: impl PgExecutor<'_>,
    thread_id: Uuid,
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        "SELECT DISTINCT n.user_id FROM notifications n
         WHERE n.thread_id = $1
            OR n.comment_id IN (SELECT c.id FROM comments c WHERE c.thread_id = $1)",
        thread_id,
    )
    .fetch_all(executor)
    .await
}

/// Marks notifications told of by mail.
pub async fn mark_mailed(
    executor: impl PgExecutor<'_>,
    ids: &[Uuid],
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE notifications SET mailed_at = $2 WHERE id = ANY($1) AND mailed_at IS NULL",
        ids,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Someone's notifications no mail told of yet, unread and not archived, made before
/// `before`, oldest first, at most `limit`.
pub async fn unmailed(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    before: OffsetDateTime,
    limit: i64,
) -> sqlx::Result<Vec<Notification>> {
    sqlx::query_as!(
        Row,
        "SELECT id, user_id, workspace_id, kind, actor_id, page_id, thread_id, comment_id,
                block_id, excerpt, property_id, invitation_id, created_at, read_at, archived_at,
                mailed_at
         FROM notifications
         WHERE user_id = $1 AND mailed_at IS NULL AND read_at IS NULL AND archived_at IS NULL
           AND created_at <= $2
         ORDER BY id
         LIMIT $3",
        user_id,
        before,
        limit,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Notification::try_from)
    .collect()
}

/// Asks for a mail of someone's notifications at `due`, or `gap` after their last one went
/// should that be later, unless one waits already, which takes in what comes meanwhile; one
/// due before `lost` that has not gone is taken as lost, and asked for again. Returns when
/// the mail asked for is due, for its job; `None` when one waits.
pub async fn schedule_mail(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    due: OffsetDateTime,
    gap: time::Duration,
    lost: OffsetDateTime,
) -> sqlx::Result<Option<OffsetDateTime>> {
    let gap = PgInterval::try_from(gap).map_err(sqlx::Error::Encode)?;
    Ok(sqlx::query_scalar!(
        "INSERT INTO notification_mail (user_id, due_at) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE
             SET due_at = GREATEST($2, notification_mail.sent_at + $3)
             WHERE notification_mail.due_at IS NULL OR notification_mail.due_at < $4
         RETURNING due_at",
        user_id,
        due,
        gap,
        lost,
    )
    .fetch_optional(executor)
    .await?
    .flatten())
}

/// When someone's mail that waits is due, and when their last one went, locked for the
/// transaction.
pub async fn lock_mail(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
) -> sqlx::Result<Option<(Option<OffsetDateTime>, Option<OffsetDateTime>)>> {
    Ok(sqlx::query!(
        "SELECT due_at, sent_at FROM notification_mail WHERE user_id = $1 FOR UPDATE",
        user_id,
    )
    .fetch_optional(executor)
    .await?
    .map(|row| (row.due_at, row.sent_at)))
}

/// Records someone's mail as it is now: when the next is due, if one waits, and when the
/// last went.
pub async fn set_mail(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    due_at: Option<OffsetDateTime>,
    sent_at: Option<OffsetDateTime>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE notification_mail SET due_at = $2, sent_at = $3 WHERE user_id = $1",
        user_id,
        due_at,
        sent_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// When the oldest of someone's notifications no mail told of yet, unread and not
/// archived, made after `after`, came.
pub async fn next_unmailed(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    after: OffsetDateTime,
) -> sqlx::Result<Option<OffsetDateTime>> {
    sqlx::query_scalar!(
        "SELECT min(created_at) FROM notifications
         WHERE user_id = $1 AND mailed_at IS NULL AND read_at IS NULL AND archived_at IS NULL
           AND created_at > $2",
        user_id,
        after,
    )
    .fetch_one(executor)
    .await
}

/// Whether someone wants a mail for each notification: a person with an address, not
/// disabled, whose preference is `each`, or who chose none and `default` is.
pub async fn wants_each_mail(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    default: MailPreference,
) -> sqlx::Result<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT (u.kind = 'human' AND u.email IS NOT NULL AND u.disabled_at IS NULL
                   AND COALESCE(p.mail, $2) = 'each') AS "wants!"
           FROM users u LEFT JOIN notification_preferences p ON p.user_id = u.id
           WHERE u.id = $1"#,
        user_id,
        default.as_str(),
    )
    .fetch_optional(executor)
    .await?
    .unwrap_or(false))
}

/// How someone wants mail of their inbox, once they chose, and when their last digest went.
pub async fn preference(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
) -> sqlx::Result<Option<(MailPreference, Option<OffsetDateTime>)>> {
    sqlx::query!(
        "SELECT mail, digest_sent_at FROM notification_preferences WHERE user_id = $1",
        user_id,
    )
    .fetch_optional(executor)
    .await?
    .map(|row| {
        let mail = decode("mail preference", &row.mail, MailPreference::parse)?;
        Ok((mail, row.digest_sent_at))
    })
    .transpose()
}

pub async fn set_preference(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    mail: MailPreference,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO notification_preferences (user_id, mail, updated_at) VALUES ($1, $2, $3)
         ON CONFLICT (user_id) DO UPDATE SET mail = $2, updated_at = $3",
        user_id,
        mail.as_str(),
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Notes that someone's digest went at `at`.
pub async fn digest_sent(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE notification_preferences SET digest_sent_at = $2 WHERE user_id = $1",
        user_id,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Who wants a digest whose last one went before `before`, or never, and has a
/// notification no mail told of yet: at most `limit`, locked for the transaction, skipping
/// those another digest run holds.
pub async fn digests_due(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
    limit: i64,
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        "SELECT p.user_id FROM notification_preferences p
         WHERE p.mail = 'digest' AND (p.digest_sent_at IS NULL OR p.digest_sent_at <= $1)
           AND EXISTS (SELECT 1 FROM notifications n
                       WHERE n.user_id = p.user_id AND n.mailed_at IS NULL
                         AND n.read_at IS NULL AND n.archived_at IS NULL)
         ORDER BY p.user_id
         LIMIT $2
         FOR UPDATE OF p SKIP LOCKED",
        before,
        limit,
    )
    .fetch_all(executor)
    .await
}
