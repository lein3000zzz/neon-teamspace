//! A database's views: making, changing, placing, duplicating and deleting them.

use neon_domain::auth::Principal;
use neon_domain::databases::filter::{FilterSpec, Fit};
use neon_domain::databases::sort::SortSpec;
use neon_domain::databases::view::{GroupingSpec, ViewPropertySpec, check_view_name};
use neon_domain::databases::{Layout, MAX_NAME, MAX_VIEWS, View, ViewSpec};
use neon_domain::pages::Placement;
use neon_domain::workspaces::Action;
use neon_store::databases::{self as repo, StoredView};
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::events::Event;
use super::schema::config_json;
use super::{DatabaseError, DatabaseService, NO_VIEW, Opened, key_for, shown_view};
use crate::Read;

/// A view to make.
#[derive(Debug, Clone)]
pub struct NewView {
    pub name: String,
    pub layout: Layout,
    pub settings: ViewSpec,
    pub placement: Placement,
}

/// What a change to a view sets; each part left out stays as it is. For the filter and the
/// grouping, `Some(None)` removes them.
#[derive(Debug, Clone, Default)]
pub struct ViewChange {
    pub name: Option<String>,
    pub layout: Option<Layout>,
    pub filter: Option<Option<FilterSpec>>,
    pub sorts: Option<Vec<SortSpec>>,
    pub group: Option<Option<GroupingSpec>>,
    pub properties: Option<Vec<ViewPropertySpec>>,
}

impl DatabaseService {
    /// Adds a view, for those who edit the database. A board groups by a select, status or
    /// person property.
    pub async fn create_view(
        &self,
        principal: &Principal,
        database_id: Uuid,
        new: NewView,
    ) -> Result<View, DatabaseError> {
        self.observe("create_view", async {
            let mut tx = self.pages().pool().begin().await?;
            let opened = self
                .open(&mut tx, principal, database_id, Read::Locked)
                .await?;
            Self::check(&opened.context, Action::EditPage)?;
            let name = check_view_name(&new.name)?;
            let settings = new
                .settings
                .resolve(&opened.schema, new.layout, Fit::Strict)?;
            settings.check_layout(new.layout)?;
            let stored = repo::views(&mut *tx, database_id).await?;
            if stored.len() >= MAX_VIEWS {
                return Err(too_many_views());
            }
            let key = place(&mut tx, &stored, None, new.placement).await?;
            let view = StoredView {
                id: Uuid::now_v7(),
                database_id,
                position: key,
                name,
                layout: new.layout,
                config: config_json(&settings.to_spec())?,
            };
            repo::insert_view(&mut *tx, &view).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                view_id = %view.id,
                layout = %view.layout,
                "database view created"
            );
            self.view_changed(database_id, view.id, false);
            Ok(one(&opened, view))
        })
        .await
    }

    /// Changes a view's name, layout or settings, each checked against the schema. A board
    /// must group once it is made one or its grouping changes; one whose grouping property
    /// was deleted or changed type takes other changes while it shows its cards ungrouped.
    pub async fn update_view(
        &self,
        principal: &Principal,
        database_id: Uuid,
        view_id: Uuid,
        change: ViewChange,
    ) -> Result<View, DatabaseError> {
        self.observe("update_view", async {
            let mut tx = self.pages().pool().begin().await?;
            let opened = self
                .open(&mut tx, principal, database_id, Read::Locked)
                .await?;
            Self::check(&opened.context, Action::EditPage)?;
            let mut stored = find(&mut tx, database_id, view_id).await?;
            if let Some(name) = &change.name {
                stored.name = check_view_name(name)?;
            }
            let layout = change.layout.unwrap_or(stored.layout);
            let regrouped = change.layout.is_some() || change.group.is_some();
            let mut spec = serde_json::from_value::<ViewSpec>(stored.config.clone())
                .unwrap_or_default()
                .fit(&opened.schema, stored.layout)
                .to_spec();
            if let Some(filter) = change.filter {
                spec.filter = filter;
            }
            if let Some(sorts) = change.sorts {
                spec.sorts = sorts;
            }
            if let Some(group) = change.group {
                spec.group = group;
            }
            if let Some(properties) = change.properties {
                spec.properties = properties;
            }
            let settings = spec.resolve(&opened.schema, layout, Fit::Strict)?;
            if regrouped {
                settings.check_layout(layout)?;
            }
            stored.layout = layout;
            stored.config = config_json(&settings.to_spec())?;
            repo::update_view(&mut *tx, &stored).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                view_id = %view_id,
                "database view changed"
            );
            self.view_changed(database_id, view_id, false);
            Ok(one(&opened, stored))
        })
        .await
    }

    /// Places a view elsewhere among the database's views.
    pub async fn move_view(
        &self,
        principal: &Principal,
        database_id: Uuid,
        view_id: Uuid,
        placement: Placement,
    ) -> Result<View, DatabaseError> {
        self.observe("move_view", async {
            let mut tx = self.pages().pool().begin().await?;
            let opened = self
                .open(&mut tx, principal, database_id, Read::Locked)
                .await?;
            Self::check(&opened.context, Action::EditPage)?;
            let stored = repo::views(&mut *tx, database_id).await?;
            let mut view = stored
                .iter()
                .find(|view| view.id == view_id)
                .cloned()
                .ok_or(DatabaseError::NotFound(NO_VIEW))?;
            view.position = place(&mut tx, &stored, Some(view_id), placement).await?;
            repo::set_view_positions(&mut *tx, &[view_id], &[view.position.clone()]).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                view_id = %view_id,
                "database view moved"
            );
            self.view_changed(database_id, view_id, false);
            Ok(one(&opened, view))
        })
        .await
    }

    /// Copies a view, with its settings, right after it, named `name` or after it.
    pub async fn duplicate_view(
        &self,
        principal: &Principal,
        database_id: Uuid,
        view_id: Uuid,
        name: Option<String>,
    ) -> Result<View, DatabaseError> {
        self.observe("duplicate_view", async {
            let mut tx = self.pages().pool().begin().await?;
            let opened = self
                .open(&mut tx, principal, database_id, Read::Locked)
                .await?;
            Self::check(&opened.context, Action::EditPage)?;
            let stored = repo::views(&mut *tx, database_id).await?;
            let original = stored
                .iter()
                .find(|view| view.id == view_id)
                .cloned()
                .ok_or(DatabaseError::NotFound(NO_VIEW))?;
            if stored.len() >= MAX_VIEWS {
                return Err(too_many_views());
            }
            let name = match name {
                Some(name) => check_view_name(&name)?,
                None => copy_name(&original.name),
            };
            let key = place(&mut tx, &stored, None, Placement::After(view_id)).await?;
            let spec = serde_json::from_value::<ViewSpec>(original.config.clone())
                .unwrap_or_default()
                .fit(&opened.schema, original.layout)
                .to_spec();
            let view = StoredView {
                id: Uuid::now_v7(),
                database_id,
                position: key,
                name,
                layout: original.layout,
                config: config_json(&spec)?,
            };
            repo::insert_view(&mut *tx, &view).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                view_id = %view.id,
                from = %view_id,
                "database view duplicated"
            );
            self.view_changed(database_id, view.id, false);
            Ok(one(&opened, view))
        })
        .await
    }

    /// Deletes a view; a database keeps at least one.
    pub async fn delete_view(
        &self,
        principal: &Principal,
        database_id: Uuid,
        view_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.observe("delete_view", async {
            let mut tx = self.pages().pool().begin().await?;
            let opened = self
                .open(&mut tx, principal, database_id, Read::Locked)
                .await?;
            Self::check(&opened.context, Action::EditPage)?;
            let stored = repo::views(&mut *tx, database_id).await?;
            if !stored.iter().any(|view| view.id == view_id) {
                return Err(DatabaseError::NotFound(NO_VIEW));
            }
            if stored.len() == 1 {
                return Err(DatabaseError::InvalidInput(
                    "a database keeps at least one view".to_owned(),
                ));
            }
            repo::delete_view(&mut *tx, view_id).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                view_id = %view_id,
                "database view deleted"
            );
            self.view_changed(database_id, view_id, true);
            Ok(())
        })
        .await
    }

    fn view_changed(&self, database_id: Uuid, view_id: Uuid, deleted: bool) {
        self.publish(database_id, &Event::ViewChanged { view_id, deleted });
    }
}

/// A view of the database, which the caller may see.
async fn find(
    conn: &mut PgConnection,
    database_id: Uuid,
    view_id: Uuid,
) -> Result<StoredView, DatabaseError> {
    repo::views(&mut *conn, database_id)
        .await?
        .into_iter()
        .find(|view| view.id == view_id)
        .ok_or(DatabaseError::NotFound(NO_VIEW))
}

/// A view as stored, as the caller reads it: see [`shown_view`].
fn one(opened: &Opened, view: StoredView) -> View {
    shown_view(&opened.schema, view, opened.context.members_hidden())
}

/// The key for a view placed among `views`, `except` the one that moves, saving fresh keys
/// first when they grew too long.
async fn place(
    conn: &mut PgConnection,
    views: &[StoredView],
    except: Option<Uuid>,
    placement: Placement,
) -> Result<String, DatabaseError> {
    let list: Vec<(Uuid, String)> = views
        .iter()
        .filter(|view| Some(view.id) != except)
        .map(|view| (view.id, view.position.clone()))
        .collect();
    let (key, fresh) = key_for(
        &list,
        placement,
        "the view to place it beside is not one of the database's",
    )?;
    if !fresh.is_empty() {
        let (ids, keys): (Vec<Uuid>, Vec<String>) = fresh.into_iter().unzip();
        repo::set_view_positions(&mut *conn, &ids, &keys).await?;
    }
    Ok(key)
}

fn too_many_views() -> DatabaseError {
    DatabaseError::TooMany(format!("a database has at most {MAX_VIEWS} views"))
}

/// The name of a view's copy: its own with ` (copy)`, cut to fit.
fn copy_name(name: &str) -> String {
    const SUFFIX: &str = " (copy)";
    let room = MAX_NAME - SUFFIX.chars().count();
    let base: String = name.chars().take(room).collect();
    format!("{}{SUFFIX}", base.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_are_named_after_their_view() {
        assert_eq!(copy_name("Board"), "Board (copy)");
        let long = "x".repeat(MAX_NAME);
        let copied = copy_name(&long);
        assert_eq!(copied.chars().count(), MAX_NAME);
        assert!(copied.ends_with(" (copy)"));
        assert!(check_view_name(&copied).is_ok());
    }
}
