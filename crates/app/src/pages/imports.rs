//! Importing: a tree of pages made in one go, each with its content as the first snapshot of
//! its document, under a page, at the top of a teamspace or among the caller's private
//! pages.

use neon_domain::auth::Principal;
use neon_domain::pages::{
    MAX_DEPTH, Page, PageAccess, PageKind, PageStyle, Placement, check_title, position,
};
use neon_store::documents::{self, NewSnapshot};
use neon_store::pages::{self as repo, Siblings};
use tracing::info;
use uuid::Uuid;

use super::operations::{Target, in_workspace};
use super::{Location, PageError, PageService, PageView};

/// A page to import, with the pages to import under it.
#[derive(Debug, Clone, Default)]
pub struct ImportedPage {
    pub title: String,
    /// Its document, or `None` for a page without content.
    pub content: Option<ImportedContent>,
    pub children: Vec<ImportedPage>,
}

impl ImportedPage {
    /// How many levels of pages it makes: 1 without children.
    pub fn height(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(Self::height)
            .max()
            .unwrap_or_default()
    }

    /// How many pages it makes, those under it included.
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Self::count).sum::<usize>()
    }
}

/// A page's document, ready to be its first snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedContent {
    /// The whole Yjs document, encoded as one update.
    pub state: Vec<u8>,
    /// Its state vector, encoded.
    pub state_vector: Vec<u8>,
    /// Its plain text, for search.
    pub text: String,
}

/// What an import made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The pages made where the import was asked to put them, in order.
    pub pages: Vec<PageView>,
    /// How many pages were made, those under them included.
    pub count: usize,
}

impl PageService {
    /// Whether the caller may add pages at `location`, as [`create_page`](Self::create_page)
    /// asks, without adding any: so that a refusal comes before the work of reading what is
    /// to be imported.
    pub async fn check_place(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        location: Location,
    ) -> Result<(), PageError> {
        self.observe("check_place", async {
            let mut conn = self.pool().acquire().await?;
            let standing = in_workspace(&mut conn, principal, workspace_id).await?;
            self.target(&mut conn, principal, workspace_id, &standing, location)
                .await?;
            Ok(())
        })
        .await
    }

    /// Makes `pages`, each with the pages under it, last at `location`, in one transaction,
    /// with the rules of [`create_page`](Self::create_page). A page's content becomes the
    /// first snapshot of its document, and its text the page's search text.
    pub async fn import_pages(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        location: Location,
        pages: Vec<ImportedPage>,
    ) -> Result<Imported, PageError> {
        self.observe("import_pages", async {
            if pages.is_empty() {
                return Err(PageError::InvalidInput("nothing to import".to_owned()));
            }
            let height = pages.iter().map(ImportedPage::height).max().unwrap_or(1);
            let user = principal.user.id;
            let mut tx = self.pool().begin().await?;
            let standing = in_workspace(&mut tx, principal, workspace_id).await?;
            let members_hidden = standing.members_hidden();
            repo::lock_tree(&mut *tx, workspace_id).await?;
            let Target {
                space,
                parent_id,
                depth,
            } = self
                .target(&mut tx, principal, workspace_id, &standing, location)
                .await?;
            if depth + height > MAX_DEPTH {
                return Err(PageError::TooDeep);
            }
            let siblings = Siblings::of(workspace_id, parent_id, space);
            let mut keys: Vec<String> = Vec::with_capacity(pages.len());
            for _ in &pages {
                let key = match keys.last() {
                    None => {
                        self.position(&mut tx, siblings, None, Placement::Last)
                            .await?
                    }
                    Some(last) => position::between(Some(last.as_str()), None)?,
                };
                keys.push(key);
            }
            let now = self.now();
            let mut made = Vec::with_capacity(pages.len());
            let mut count = 0;
            // Parents before their children, each set of siblings in order.
            let mut stack: Vec<(ImportedPage, Option<Uuid>, String, bool)> = pages
                .into_iter()
                .zip(keys)
                .rev()
                .map(|(page, key)| (page, parent_id, key, true))
                .collect();
            while let Some((imported, parent_id, position, top)) = stack.pop() {
                let page = Page {
                    id: Uuid::now_v7(),
                    workspace_id,
                    kind: PageKind::Page,
                    space,
                    parent_id,
                    position,
                    title: check_title(&imported.title).map_err(PageError::invalid)?,
                    icon: None,
                    cover: None,
                    style: PageStyle::default(),
                    created_by: Some(user),
                    last_edited_by: Some(user),
                    created_at: now,
                    updated_at: now,
                    content_updated_at: now,
                    trashed_at: None,
                    trashed_by: None,
                    trashed_with: None,
                };
                repo::insert(&mut *tx, &page).await?;
                if let Some(content) = &imported.content {
                    documents::save_snapshot(
                        &mut *tx,
                        page.id,
                        NewSnapshot {
                            state: &content.state,
                            state_vector: &content.state_vector,
                            last_seq: 0,
                            at: now,
                        },
                    )
                    .await?;
                    documents::set_content(&mut *tx, page.id, Some(&content.text), None).await?;
                }
                count += 1;
                let keys = position::sequence(imported.children.len());
                stack.extend(
                    imported
                        .children
                        .into_iter()
                        .zip(keys)
                        .rev()
                        .map(|(child, key)| (child, Some(page.id), key, false)),
                );
                if top {
                    made.push(PageView {
                        page,
                        access: PageAccess::Edit,
                        members_hidden,
                    });
                }
            }
            tx.commit().await?;
            info!(
                user_id = %user,
                workspace_id = %workspace_id,
                pages = count,
                "pages imported"
            );
            Ok(Imported { pages: made, count })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(children: Vec<ImportedPage>) -> ImportedPage {
        ImportedPage {
            children,
            ..ImportedPage::default()
        }
    }

    #[test]
    fn trees_know_their_height_and_size() {
        let leaf = page(Vec::new());
        assert_eq!((leaf.height(), leaf.count()), (1, 1));
        let tree = page(vec![page(vec![page(Vec::new())]), page(Vec::new())]);
        assert_eq!((tree.height(), tree.count()), (3, 4));
    }
}
