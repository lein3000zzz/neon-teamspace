//! Keeping a version: in the transaction that saves a page's snapshot, its blocks become a
//! version when they differ from the latest one's, with the files they show, and a page that
//! keeps more versions than the retention's cap is thinned at once.

use neon_blocks::Document;
use neon_blocks::diff::{Summary, diff};
use neon_domain::history::{Retention, to_delete};
use neon_store::versions::{self, NewVersion};
use sqlx::PgConnection;
use time::OffsetDateTime;
use tracing::warn;
use uuid::Uuid;

/// What a snapshot keeps as a version.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Snapshot<'a> {
    pub(crate) page_id: Uuid,
    /// The document's blocks as the editor shows them.
    pub(crate) document: &'a Document,
    /// Who changed it since the last snapshot.
    pub(crate) authors: &'a [Uuid],
    /// When its latest change was made.
    pub(crate) at: OffsetDateTime,
    /// The version a restore made it from, and when that one was made.
    pub(crate) restored_from: Option<(Uuid, OffsetDateTime)>,
}

/// Keeps the snapshot's blocks as a version, compared with the latest version, unless they
/// are the same, with the files they show, which the attachments' cleanup keeps while the
/// version is kept; says whether it kept one. A latest version that does not read, which
/// the server never writes, counts as an empty page.
pub(crate) async fn record(
    conn: &mut PgConnection,
    snapshot: Snapshot<'_>,
) -> Result<bool, sqlx::Error> {
    let page_id = snapshot.page_id;
    let latest = versions::latest_content(&mut *conn, page_id).await?;
    // The latest version read and compared, and the blocks encoded: off the async workers.
    let compared = crate::sync::blocking(|| {
        let previous = latest.map(|content| {
            serde_json::from_value::<Document>(content).unwrap_or_else(|error| {
                warn!(%page_id, %error, "the latest version does not read; compared with an empty page");
                Document::default()
            })
        });
        if previous.as_ref() == Some(snapshot.document) {
            return Ok(None);
        }
        let summary = Summary::of(&diff(&previous.unwrap_or_default(), snapshot.document));
        serde_json::to_value(snapshot.document).map(|content| Some((summary, content)))
    })
    .map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
    let Some((summary, content)) = compared else {
        return Ok(false);
    };
    let count = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
    let mut authors = snapshot.authors.to_vec();
    authors.sort_unstable();
    authors.dedup();
    let id = Uuid::now_v7();
    versions::insert(
        &mut *conn,
        NewVersion {
            id,
            page_id: snapshot.page_id,
            created_at: snapshot.at,
            authors: &authors,
            content: &content,
            added: count(summary.added),
            changed: count(summary.changed),
            removed: count(summary.removed),
            restored_from: snapshot.restored_from,
        },
    )
    .await?;
    let files = crate::attachments::shown_files(snapshot.document);
    if !files.is_empty() {
        neon_store::files::set_version_refs(&mut *conn, id, &files).await?;
    }
    Ok(true)
}

/// Thins a page's versions as the hourly cleanup would as of `now`, when it keeps more than
/// the retention's cap, so that a page edited fast, or restored over and over, keeps no
/// more versions between cleanups than the cleanup leaves. Says how many went.
pub(crate) async fn cap(
    conn: &mut PgConnection,
    page_id: Uuid,
    retention: Retention,
    now: OffsetDateTime,
) -> Result<u64, sqlx::Error> {
    let dated = versions::dated(&mut *conn, page_id).await?;
    if dated.len() <= retention.max_versions {
        return Ok(0);
    }
    let doomed = to_delete(&dated, now, retention);
    if doomed.is_empty() {
        return Ok(0);
    }
    versions::delete(&mut *conn, &doomed).await
}
