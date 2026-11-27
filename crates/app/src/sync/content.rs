//! A page's content as the server reads and edits it for a principal, an agent's tools and
//! the API's markdown routes: its blocks as they are now, live document included, and edits
//! made through the live document, so whoever has the page open sees them at once and what
//! they type meanwhile merges with them.
//!
//! Reading needs to see the page; editing needs to edit it, out of the trash, with the write
//! scope when through a token, as editing in the browser does, and is rate limited per user.
//! A database has no body of its own: its rows do.
//!
//! Every read has a [`revision`], which changes whenever what reading shows changes. An edit
//! that may remove blocks, replacing a range or the body, names the revision it was made
//! from, and is refused once the page is at another: made from a stale read, it would remove
//! what someone added or changed since. An edit made on the revision it names tells the one
//! it leaves, so the next can be based on it without reading again. Reads and edits are RED metrics of
//! [`SUBSYSTEM`](super::SUBSYSTEM), in spans `sync read` and `sync edit`; neither logs a
//! page's content.

use std::future::Future;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use neon_blocks::Document;
use neon_blocks::edit::{Applied, Edit, EditError};
use neon_blocks::markdown::to_markdown_with_ids;
use neon_domain::auth::Principal;
use neon_domain::pages::{PageAccess, PageKind};
use neon_observability::errors::{mark_expected, record_unexpected};
use sha2::{Digest as _, Sha256};
use tokio::sync::oneshot;
use tracing::{Instrument as _, info, info_span, warn};
use uuid::Uuid;
use yrs::sync::{Message, SyncMessage};

use super::document::{Command, Made, Unedited};
use super::{CloseReason, JOIN_ATTEMPTS, LoadError, SyncService, protocol};
use crate::pages::{ImportedContent, PageError, PageView};
use crate::ratelimit::Bucket;

/// The most parts one edit has.
pub const MAX_EDITS: usize = 50;

/// Why a page's content cannot be read or edited as asked.
///
/// Every variant but [`Load`](Self::Load), [`Unavailable`](Self::Unavailable) and the
/// failures of [`Page`](Self::Page) is expected: it is the caller's to fix, and its message
/// is safe to show them. No message quotes the page.
#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error(transparent)]
    Page(#[from] PageError),
    #[error("{0}")]
    InvalidInput(String),
    /// A part of an edit, counted from 0, cannot be made; none of the edit was.
    #[error("edit {index}: {error}; nothing was changed")]
    Refused { index: usize, error: EditError },
    /// An edit that may remove blocks names no revision to be based on.
    #[error(
        "replacing a range or the body needs `base`, the revision reading the page gave: read \
         it, then edit what it holds; nothing was changed"
    )]
    NoBase,
    /// The page changed since the revision an edit that may remove blocks was based on.
    #[error(
        "the page changed since that revision was read, and replacing blocks now could remove \
         what was added or changed since: read it again and edit what it holds now; nothing \
         was changed"
    )]
    Conflict,
    #[error("a database has no body of its own: read and change its rows instead")]
    Database,
    #[error("the page would grow larger than {} bytes, the most a page holds", .0)]
    TooLarge(usize),
    /// What is to be written makes more nodes than are written at once, this many.
    #[error("{}", too_many_nodes(*.0))]
    TooManyNodes(usize),
    #[error("too many edits of pages' content: slow down")]
    RateLimited { retry_after: Duration },
    /// The live document could not open, or the server is stopping.
    #[error("the page's document is not available now: {0}")]
    Unavailable(CloseReason),
    #[error("the page's document does not load: {0}")]
    Load(#[from] LoadError),
}

impl ContentError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Page(error) => error.code(),
            Self::InvalidInput(_) | Self::Database => "invalid_input",
            Self::Refused { .. } => "edit_refused",
            Self::NoBase => "base_required",
            Self::Conflict => "edit_conflict",
            Self::TooLarge(_) | Self::TooManyNodes(_) => "document_too_large",
            Self::RateLimited { .. } => "rate_limited",
            Self::Unavailable(_) => "unavailable",
            Self::Load(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        match self {
            Self::Page(error) => error.is_expected(),
            Self::Unavailable(_) | Self::Load(_) => false,
            _ => true,
        }
    }
}

/// Why content past [`Limits::max_nodes`](super::Limits::max_nodes), `max`, is refused.
pub(crate) fn too_many_nodes(max: usize) -> String {
    format!(
        "the content makes more than {max} blocks, table rows and cells, runs of text, line \
         breaks and mentions, the most written into a page at once: write it in parts"
    )
}

/// What an edit did: the page as it is now, and what each part did, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    pub page: PageView,
    pub applied: Vec<Applied>,
    /// The page's revision once the edit was made, when it was made on the revision it was
    /// based on: then the page holds that revision's blocks and this edit's, and nobody
    /// else's since.
    pub revision: Option<String>,
}

/// The revision of a page's blocks as reading shows them: 22 characters of base64url, the
/// first 128 bits of the SHA-256 of their markdown with ids, which changes whenever what
/// reading shows does.
pub fn revision(document: &Document) -> String {
    let digest = Sha256::digest(to_markdown_with_ids(document).as_bytes());
    URL_SAFE_NO_PAD.encode(&digest[..16])
}

/// Whether a message a page's room carries changes its document, rather than telling who is
/// there and where their cursors are.
pub fn changes_document(message: &[u8]) -> bool {
    protocol::decode(message).is_ok_and(|messages| {
        messages
            .iter()
            .any(|message| matches!(message, Message::Sync(SyncMessage::Update(_))))
    })
}

impl SyncService {
    /// A page the principal may see, and its blocks as they are now: see
    /// [`content`](Self::content).
    pub async fn read(
        &self,
        principal: &Principal,
        page_id: Uuid,
    ) -> Result<(PageView, Document), ContentError> {
        self.observe("read", async {
            let view = self.inner.pages.page(principal, page_id).await?;
            if view.page.kind == PageKind::Database {
                return Err(ContentError::Database);
            }
            let document = self.content(page_id).await?;
            Ok((view, document))
        })
        .await
    }

    /// Makes `edits` to a page's blocks, in order, all or none, through its live document,
    /// which opens for them if nobody has the page open, on the [`revision`] `base` when
    /// one is given; edits that may remove blocks need it. They count as the principal's
    /// change of the page, and against their rate of edits.
    pub async fn edit(
        &self,
        principal: &Principal,
        page_id: Uuid,
        edits: Vec<Edit>,
        base: Option<String>,
    ) -> Result<Edited, ContentError> {
        self.observe("edit", async {
            if edits.is_empty() || edits.len() > MAX_EDITS {
                return Err(ContentError::InvalidInput(format!(
                    "an edit has 1 to {MAX_EDITS} parts"
                )));
            }
            if base.is_none() && edits.iter().any(Edit::removes) {
                return Err(ContentError::NoBase);
            }
            let nodes = edits.iter().map(Edit::nodes).fold(0, usize::saturating_add);
            self.inner
                .within_nodes(nodes)
                .map_err(ContentError::TooManyNodes)?;
            let view = self.editable(principal, page_id).await?;
            let parts: Vec<&'static str> = edits.iter().map(Edit::name).collect();
            let made = self
                .apply(
                    page_id,
                    view.page.workspace_id,
                    principal.user.id,
                    edits,
                    base,
                )
                .await?;
            info!(
                user_id = %principal.user.id,
                page_id = %page_id,
                parts = ?parts,
                "page edited"
            );
            Ok(Edited {
                page: view,
                applied: made.applied,
                revision: made.revision,
            })
        })
        .await
    }

    /// Makes a page hold the blocks of a version of it again, `from` naming that version and
    /// when it was made, on the revision `base` the caller compared against, refused once the
    /// page is at another: see [`crate::history`]. It counts as the principal's edit of the
    /// page, needs what editing it needs, and goes through its live document as one change,
    /// so whoever has the page open sees it at once. Restores count against a rate of their
    /// own as well, [`Bucket::PageRestore`], as each keeps a whole copy of the page.
    pub async fn restore(
        &self,
        principal: &Principal,
        page_id: Uuid,
        blocks: Vec<neon_blocks::Block>,
        from: (Uuid, time::OffsetDateTime),
        base: String,
    ) -> Result<Edited, ContentError> {
        self.observe("restore", async {
            self.inner
                .within_nodes(neon_blocks::nodes(&blocks))
                .map_err(ContentError::TooManyNodes)?;
            let view = self.editable(principal, page_id).await?;
            let user = principal.user.id.to_string();
            if let Err(limited) = self.inner.limiter.check(Bucket::PageRestore, &user).await {
                warn!(user_id = %principal.user.id, "restores of pages' versions rate limited");
                return Err(ContentError::RateLimited {
                    retry_after: limited.retry_after,
                });
            }
            let made = self
                .send(view.page.workspace_id, page_id, |done| Command::Restore {
                    author: principal.user.id,
                    blocks: blocks.clone(),
                    from,
                    base: base.clone(),
                    done,
                })
                .await?;
            Ok(Edited {
                page: view,
                applied: made.applied,
                revision: made.revision,
            })
        })
        .await
    }

    /// A page the principal may edit the content of now, its edit counted against their
    /// rate: as editing in the browser needs, edit access, out of the trash, through a
    /// token with the write scope. A database has no content of its own.
    async fn editable(
        &self,
        principal: &Principal,
        page_id: Uuid,
    ) -> Result<PageView, ContentError> {
        let view = self.inner.pages.document_access(principal, page_id).await?;
        if view.page.kind == PageKind::Database {
            return Err(ContentError::Database);
        }
        if view.page.is_trashed() {
            return Err(PageError::Forbidden("this page is in the trash: restore it first").into());
        }
        if view.access != PageAccess::Edit {
            return Err(PageError::Forbidden(
                "you may only view this page: editing it needs edit access, and a token with \
                 the write scope",
            )
            .into());
        }
        let user = principal.user.id.to_string();
        if let Err(limited) = self.inner.limiter.check(Bucket::PageEdit, &user).await {
            warn!(user_id = %principal.user.id, "edits of pages' content rate limited");
            return Err(ContentError::RateLimited {
                retry_after: limited.retry_after,
            });
        }
        Ok(view)
    }

    /// Hands the edits to the page's live document, opening it again should it unload
    /// meanwhile.
    async fn apply(
        &self,
        page_id: Uuid,
        workspace_id: Uuid,
        author: Uuid,
        edits: Vec<Edit>,
        base: Option<String>,
    ) -> Result<Made, ContentError> {
        self.send(workspace_id, page_id, |done| Command::Edit {
            author,
            edits: edits.clone(),
            base: base.clone(),
            done,
        })
        .await
    }

    /// Hands a command that edits the page to its live document, opening it again should it
    /// unload meanwhile, and reads what it made.
    async fn send(
        &self,
        workspace_id: Uuid,
        page_id: Uuid,
        command: impl Fn(oneshot::Sender<Result<Made, Unedited>>) -> Command,
    ) -> Result<Made, ContentError> {
        for _ in 0..JOIN_ATTEMPTS {
            let commands = self
                .document(page_id, workspace_id)
                .map_err(ContentError::Unavailable)?;
            let (done, answer) = oneshot::channel();
            if commands.send(command(done)).await.is_err() {
                continue;
            }
            // Dropped unanswered when the document unloaded first: it opens afresh.
            let Ok(made) = answer.await else {
                continue;
            };
            return made.map_err(|unedited| match unedited {
                Unedited::Conflict => ContentError::Conflict,
                Unedited::Refused(index, error) => ContentError::Refused { index, error },
                Unedited::TooLarge => ContentError::TooLarge(self.inner.limits.max_document),
                Unedited::Gone => PageError::NotFound("no such page").into(),
                Unedited::Unsaved => ContentError::Unavailable(CloseReason::ServerError),
            });
        }
        Err(ContentError::Unavailable(CloseReason::ServerError))
    }

    /// The first snapshot of a new page's document, holding `document`: `None` for no
    /// blocks. Refused when it makes more nodes than are written at once, before any is
    /// written, and when larger than a page may grow. It is written in one of the turns of
    /// [`Limits::parallel_writes`](super::Limits::parallel_writes), off the async threads.
    pub async fn first_snapshot(
        &self,
        document: &Document,
    ) -> Result<Option<ImportedContent>, ContentError> {
        self.inner
            .within_nodes(document.nodes())
            .map_err(ContentError::TooManyNodes)?;
        let Some(content) = self.inner.write(|| ImportedContent::of(document)).await else {
            return Ok(None);
        };
        let max = self.inner.limits.max_document;
        if content.state.len() > max {
            return Err(ContentError::TooLarge(max));
        }
        Ok(Some(content))
    }

    /// Runs a use case in its span and records its outcome: a refusal counts as `ok`.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, ContentError>>,
    ) -> Result<T, ContentError> {
        let span = info_span!(
            "sync",
            otel.name = %format!("sync {operation}"),
            sync.operation = operation,
        );
        let guard = self.inner.red.start(operation);
        let result = work.instrument(span.clone()).await;
        match &result {
            Ok(_) => guard.ok(),
            Err(error) if error.is_expected() => {
                mark_expected(&span, error.code());
                guard.ok();
            }
            Err(error) => {
                record_unexpected(&span, error);
                guard.err();
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use yrs::sync::{Awareness, AwarenessUpdate};
    use yrs::{Doc, StateVector};

    use super::*;

    #[test]
    fn only_updates_change_a_document() {
        let update = protocol::encode(&Message::Sync(SyncMessage::Update(vec![0, 0])));
        let step1 = protocol::encode(&Message::Sync(SyncMessage::SyncStep1(
            StateVector::default(),
        )));
        let awareness = Awareness::new(Doc::new());
        let states: AwarenessUpdate = awareness.update().expect("an update");
        let presence = protocol::encode(&Message::Awareness(states));

        assert!(changes_document(&update));
        assert!(!changes_document(&step1));
        assert!(!changes_document(&presence));
        assert!(!changes_document(&Bytes::from_static(b"\xff")));
        let both = [presence.to_vec(), update.to_vec()].concat();
        assert!(changes_document(&both));
    }

    #[test]
    fn refusals_are_the_caller_s_and_failures_the_server_s() {
        let refused = ContentError::Refused {
            index: 1,
            error: EditError::NotFound,
        };
        assert!(refused.is_expected());
        assert_eq!(refused.code(), "edit_refused");
        assert_eq!(
            refused.to_string(),
            "edit 1: the text to find is not in the page; nothing was changed"
        );
        assert!(ContentError::TooLarge(10).is_expected());
        let limited = ContentError::RateLimited {
            retry_after: Duration::from_secs(1),
        };
        assert!(limited.is_expected());
        assert_eq!(limited.code(), "rate_limited");
        assert!(!ContentError::Unavailable(CloseReason::Restarting).is_expected());
        assert_eq!(ContentError::Database.code(), "invalid_input");
    }
}
