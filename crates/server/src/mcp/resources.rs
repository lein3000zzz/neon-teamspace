//! The MCP server's resources, named by `neon://` URIs:
//!
//! | URI | What it reads |
//! |---|---|
//! | `neon://page/{id}` | the page's blocks as markdown with their ids, then its page as JSON, with a row's properties |
//! | `neon://database/{id}` | the database's page, properties and views, as JSON |
//! | `neon://database/{id}/view/{view}` | the rows the view shows, as JSON, at most 1000 |
//! | `neon://me/tasks` | the rows assigned to the caller, as JSON, at most 100 |
//! | `neon://me/inbox` | the caller's notifications in the workspace, as JSON, the newest 50 |
//!
//! Listing gives `neon://me/tasks`, `neon://me/inbox`, the caller's favorites, the pages
//! changed last and each task, each once and bounded. Each resource can be subscribed to: see
//! [`watch`](super::watch).

use std::collections::HashSet;

use neon_app::databases::{MAX_PAGE, QueryInput};
use neon_app::notifications::InboxQuery;
use neon_domain::auth::Principal;
use neon_domain::pages::PageKind;
use rmcp::model::{Resource as Listed, ResourceContents, ResourceTemplate};
use serde_json::json;
use uuid::Uuid;

use super::Services;
use super::tools::ToolError;
use crate::http::content::page_markdown;
use crate::http::database_rows::{QueryAnswer, Task, TaskList};
use crate::http::databases::Database;
use crate::http::inbox::NotificationList;

const SCHEME: &str = "neon://";
const MARKDOWN: &str = "text/markdown";
const JSON: &str = "application/json";

/// How many of each kind of resource a list holds, at most.
const LISTED_FAVORITES: usize = 50;
const LISTED_RECENT: usize = 20;
const LISTED_TASKS: usize = 50;

/// What a resource URI names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Resource {
    Page(Uuid),
    Database(Uuid),
    View { database: Uuid, view: Uuid },
    Tasks,
    Inbox,
}

impl Resource {
    /// What a URI names; one the server has no resource at is not found. A resource has one
    /// URI, its ids lowercase and hyphenated, which subscriptions are told of changes by: any
    /// other way of writing it is not found either.
    pub(crate) fn parse(uri: &str) -> Result<Self, ToolError> {
        let unknown = || ToolError::invalid("not_found", format!("there is no resource {uri}"));
        let path = uri.strip_prefix(SCHEME).ok_or_else(unknown)?;
        let parts: Vec<&str> = path.split('/').collect();
        let id = |text: &str| Uuid::try_parse(text).map_err(|_| unknown());
        let resource = match parts.as_slice() {
            ["page", page] => Self::Page(id(page)?),
            ["database", database] => Self::Database(id(database)?),
            ["database", database, "view", view] => Self::View {
                database: id(database)?,
                view: id(view)?,
            },
            ["me", "tasks"] => Self::Tasks,
            ["me", "inbox"] => Self::Inbox,
            _ => return Err(unknown()),
        };
        if resource.uri() != uri {
            return Err(unknown());
        }
        Ok(resource)
    }

    pub(crate) fn uri(&self) -> String {
        match self {
            Self::Page(id) => format!("{SCHEME}page/{id}"),
            Self::Database(id) => format!("{SCHEME}database/{id}"),
            Self::View { database, view } => format!("{SCHEME}database/{database}/view/{view}"),
            Self::Tasks => format!("{SCHEME}me/tasks"),
            Self::Inbox => format!("{SCHEME}me/inbox"),
        }
    }

    /// The resource of a page: a database's, or else the page's.
    fn of(id: Uuid, kind: PageKind) -> Self {
        match kind {
            PageKind::Database => Self::Database(id),
            PageKind::Page | PageKind::Row => Self::Page(id),
        }
    }

    fn mime_type(&self) -> &'static str {
        match self {
            Self::Page(_) => MARKDOWN,
            _ => JSON,
        }
    }
}

pub(crate) fn templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new("neon://page/{id}", "page")
            .with_title("A page")
            .with_description(
                "A page's blocks as markdown, each block's id on a line `<!-- block <id> -->` \
                 before it, then the page as JSON, with a row's properties.",
            )
            .with_mime_type(MARKDOWN),
        ResourceTemplate::new("neon://database/{id}", "database")
            .with_title("A database")
            .with_description("A database's page, properties and views, as JSON.")
            .with_mime_type(JSON),
        ResourceTemplate::new("neon://database/{id}/view/{view}", "view")
            .with_title("A view's rows")
            .with_description("The rows a view of a database shows, in its order, at most 1000.")
            .with_mime_type(JSON),
        ResourceTemplate::new("neon://me/tasks", "tasks")
            .with_title("My tasks")
            .with_description(
                "The rows of the workspace's databases whose person properties name you, the \
                 most recently changed first, at most 100.",
            )
            .with_mime_type(JSON),
        ResourceTemplate::new("neon://me/inbox", "inbox")
            .with_title("My inbox")
            .with_description(
                "Your notifications in the workspace, the newest first, at most 50: mentions, \
                 replies, comments on pages you follow and assignments, read or not.",
            )
            .with_mime_type(JSON),
    ]
}

fn workspace(principal: &Principal) -> Result<Uuid, ToolError> {
    principal.workspace().ok_or_else(|| {
        ToolError::invalid(
            "token_not_bound",
            "the MCP server works in the one workspace a token is bound to",
        )
    })
}

fn refused(error: impl Into<crate::http::ApiError>) -> ToolError {
    ToolError::from(error.into())
}

/// The resources to list for the caller: their tasks and inbox, favorites and the pages
/// changed last.
pub(crate) async fn list(s: &Services, principal: &Principal) -> Result<Vec<Listed>, ToolError> {
    let ws = workspace(principal)?;
    let mut seen = HashSet::new();
    let mut listed = Vec::new();
    let mut add = |resource: Resource, name: &str, description: String| {
        if seen.insert(resource) {
            let title = if name.is_empty() { "Untitled" } else { name };
            listed.push(
                Listed::new(resource.uri(), title)
                    .with_title(title)
                    .with_description(description)
                    .with_mime_type(resource.mime_type()),
            );
        }
    };
    add(
        Resource::Tasks,
        "My tasks",
        "The rows assigned to you.".to_owned(),
    );
    add(
        Resource::Inbox,
        "My inbox",
        "Your notifications: mentions, replies, comments and assignments.".to_owned(),
    );
    let favorites = s
        .pages
        .favorites(principal, Some(ws))
        .await
        .map_err(refused)?;
    for view in favorites.into_iter().take(LISTED_FAVORITES) {
        let page = view.page;
        add(
            Resource::of(page.id, page.kind),
            &page.title,
            "One of your favorites.".to_owned(),
        );
    }
    let recent = s
        .pages
        .recent_pages(principal, ws, LISTED_RECENT)
        .await
        .map_err(refused)?;
    for view in recent {
        let page = view.page;
        add(
            Resource::of(page.id, page.kind),
            &page.title,
            "Changed lately.".to_owned(),
        );
    }
    let tasks = s.databases.tasks(principal, ws).await.map_err(refused)?;
    for task in tasks.into_iter().take(LISTED_TASKS) {
        let status = task
            .status
            .map(|status| format!(", {}", status.name))
            .unwrap_or_default();
        add(
            Resource::Page(task.row.id),
            &task.row.title,
            format!("Assigned to you: {}{status}.", task.row.unique_id),
        );
    }
    Ok(listed)
}

/// What a resource holds, as the caller sees it.
pub(crate) async fn read(
    s: &Services,
    principal: &Principal,
    resource: &Resource,
) -> Result<Vec<ResourceContents>, ToolError> {
    let uri = resource.uri();
    let json = |value: serde_json::Value| {
        ResourceContents::text(value.to_string(), uri.clone()).with_mime_type(JSON)
    };
    Ok(match *resource {
        Resource::Page(id) => {
            let read = page_markdown(&s.sync, &s.databases, principal, id).await?;
            let about = json!({
                "page": read.page,
                "revision": read.revision,
                "properties": read.properties,
            });
            vec![
                ResourceContents::text(read.markdown, uri.clone()).with_mime_type(MARKDOWN),
                json(about),
            ]
        }
        Resource::Database(id) => {
            let database = s.databases.database(principal, id).await.map_err(refused)?;
            vec![json(json!(Database::from(database)))]
        }
        Resource::View { database, view } => {
            let input = QueryInput {
                view_id: Some(view),
                limit: Some(MAX_PAGE),
                ..QueryInput::default()
            };
            let rows = s
                .databases
                .query(principal, database, input)
                .await
                .map_err(refused)?;
            vec![json(json!(QueryAnswer::from(rows)))]
        }
        Resource::Tasks => {
            let tasks = s
                .databases
                .tasks(principal, workspace(principal)?)
                .await
                .map_err(refused)?;
            vec![json(json!(TaskList {
                tasks: tasks.into_iter().map(Task::from).collect(),
            }))]
        }
        Resource::Inbox => {
            let query = InboxQuery {
                workspace_id: Some(workspace(principal)?),
                ..InboxQuery::default()
            };
            let page = s.inbox.inbox(principal, query).await.map_err(refused)?;
            vec![json(json!(NotificationList::from(page)))]
        }
    })
}

/// Whether the caller sees the resource now: not found otherwise, as if it were not there.
pub(crate) async fn check(
    s: &Services,
    principal: &Principal,
    resource: &Resource,
) -> Result<(), ToolError> {
    match *resource {
        Resource::Page(id) => {
            let view = s.pages.page(principal, id).await.map_err(refused)?;
            if view.page.kind == PageKind::Database {
                return Err(ToolError::invalid(
                    "invalid_input",
                    format!("this is a database: its resource is neon://database/{id}"),
                ));
            }
        }
        Resource::Database(id) => {
            s.databases.database(principal, id).await.map_err(refused)?;
        }
        Resource::View { database, view } => {
            let found = s
                .databases
                .database(principal, database)
                .await
                .map_err(refused)?;
            if !found.views.iter().any(|shown| shown.id == view) {
                return Err(ToolError::invalid("not_found", "no such view"));
            }
        }
        Resource::Tasks | Resource::Inbox => {
            s.workspaces
                .admits(principal, workspace(principal)?)
                .await
                .map_err(refused)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_name_their_resources_and_back() {
        let page = Uuid::from_u128(1);
        let view = Uuid::from_u128(2);
        for resource in [
            Resource::Page(page),
            Resource::Database(page),
            Resource::View {
                database: page,
                view,
            },
            Resource::Tasks,
            Resource::Inbox,
        ] {
            assert_eq!(Resource::parse(&resource.uri()).ok(), Some(resource));
        }
        assert_eq!(Resource::Inbox.uri(), "neon://me/inbox");
        assert_eq!(
            Resource::Page(page).uri(),
            "neon://page/00000000-0000-0000-0000-000000000001"
        );
        for unknown in [
            "neon://page/",
            "neon://page/nope",
            "neon://pages/00000000-0000-0000-0000-000000000001",
            "neon://database/00000000-0000-0000-0000-000000000001/view",
            "neon://me/notes",
            "https://example.com/page/00000000-0000-0000-0000-000000000001",
            "",
            // The ids written another way than the resource's own URI has them.
            "neon://page/00000000-0000-0000-0000-00000000000A",
            "neon://page/00000000000000000000000000000001",
            "neon://page/{00000000-0000-0000-0000-000000000001}",
            "neon://page/urn:uuid:00000000-0000-0000-0000-000000000001",
            "neon://database/00000000-0000-0000-0000-000000000001/view/00000000-0000-0000-0000-00000000000B",
        ] {
            let error = Resource::parse(unknown).expect_err(unknown);
            assert_eq!(error.code, "not_found", "{unknown}");
        }
    }
}
