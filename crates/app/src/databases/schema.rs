//! Making a database, reading its schema, and changing its properties.

use std::collections::HashMap;

use neon_domain::auth::Principal;
use neon_domain::databases::convert::convert;
use neon_domain::databases::{
    ChoiceSpec, NumberFormat, Property, PropertyKind, Schema, Value, ViewSpec, check_options,
    default_properties, filter::Fit, view::default_views,
};
use neon_domain::pages::{
    MAX_DEPTH, Page, PageAccess, PageKind, PageStyle, Placement, check_title, position,
};
use neon_domain::workspaces::{Action, check_icon};
use neon_store::databases::{self as repo, Database, StoredView};
use neon_store::is_unique_violation;
use neon_store::pages::{self as pages_repo, Siblings};
use serde_json::Value as Json;
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::events::{self, Event};
use super::rows::check_size;
use super::{
    DatabaseError, DatabaseService, DatabaseView, NO_PROPERTY, Opened, key_for, new_type,
    settings_type, shown_views,
};
use crate::pages::{Location, PageError, PageView, Target, in_workspace};

/// What a new database starts with, besides its default properties and views.
#[derive(Debug, Clone, Default)]
pub struct NewDatabase {
    pub title: String,
    pub icon: Option<String>,
    /// Embedded in the page it goes under, which it must name.
    pub inline: bool,
}

/// A property's kind with its settings: a number's format and currency, a unique id's
/// prefix. Settings of another kind are refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertySpec {
    pub kind: PropertyKind,
    pub number_format: Option<NumberFormat>,
    pub currency: Option<String>,
    pub prefix: Option<String>,
}

/// A property to add.
#[derive(Debug, Clone)]
pub struct NewProperty {
    pub name: String,
    pub spec: PropertySpec,
    /// For a select, multi-select or status, its options; a status without them gets the
    /// default ones.
    pub options: Option<Vec<ChoiceSpec>>,
    pub placement: Placement,
}

/// What a change to a property sets; each part left out stays as it is.
#[derive(Debug, Clone, Default)]
pub struct PropertyChange {
    pub name: Option<String>,
    /// Another kind converts the values, or the same kind with other settings.
    pub spec: Option<PropertySpec>,
    /// Every option, in its new order: those named by id keep their values, and the values
    /// of those left out are cleared.
    pub options: Option<Vec<ChoiceSpec>>,
    /// Changes the kind even when values are lost.
    pub confirm: bool,
}

impl DatabaseService {
    /// Makes a database under a page the caller edits, at the top of a teamspace they are
    /// in, or among their private pages, with a title and a status property, a table view
    /// and a board grouped by status. An inline one is embedded in the page it goes under.
    pub async fn create_database(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        location: Location,
        placement: Placement,
        new: NewDatabase,
    ) -> Result<DatabaseView, DatabaseError> {
        self.observe("create_database", async {
            let title = check_title(&new.title).map_err(PageError::invalid)?;
            let icon = match new.icon {
                Some(icon) => check_icon(&icon).map_err(PageError::invalid)?,
                None => None,
            };
            if new.inline && !matches!(location, Location::Under(_)) {
                return Err(DatabaseError::InvalidInput(
                    "an inline database is embedded in the page it goes under: name its \
                     parent_id"
                        .to_owned(),
                ));
            }
            let pages = self.pages();
            let user = principal.user.id;
            let mut tx = pages.pool().begin().await?;
            let standing = in_workspace(&mut tx, principal, workspace_id).await?;
            pages_repo::lock_tree(&mut *tx, workspace_id).await?;
            let Target {
                space,
                parent_id,
                depth,
            } = pages
                .target(&mut tx, principal, workspace_id, &standing, location)
                .await?;
            if depth >= MAX_DEPTH {
                return Err(PageError::TooDeep.into());
            }
            let siblings = Siblings::of(workspace_id, parent_id, space);
            let position = pages.position(&mut tx, siblings, None, placement).await?;
            let now = self.now();
            let page = Page {
                id: Uuid::now_v7(),
                workspace_id,
                kind: PageKind::Database,
                space,
                parent_id,
                position,
                title,
                icon,
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
            pages_repo::insert(&mut *tx, &page).await?;
            repo::insert(
                &mut *tx,
                &Database {
                    page_id: page.id,
                    workspace_id,
                    inline: new.inline,
                    next_number: 1,
                },
            )
            .await?;
            let schema = Schema {
                properties: default_properties(&mut Uuid::now_v7),
            };
            let keys = position::sequence(schema.properties.len());
            for (property, key) in schema.properties.iter().zip(&keys) {
                repo::insert_property(&mut *tx, page.id, property, key).await?;
            }
            let defaults = default_views(&schema);
            let keys = position::sequence(defaults.len());
            for ((name, layout, spec), key) in defaults.into_iter().zip(keys) {
                let settings = spec.resolve(&schema, layout, Fit::Strict)?;
                settings.check_layout(layout)?;
                repo::insert_view(
                    &mut *tx,
                    &StoredView {
                        id: Uuid::now_v7(),
                        database_id: page.id,
                        position: key,
                        name,
                        layout,
                        config: config_json(&settings.to_spec())?,
                    },
                )
                .await?;
            }
            let members_hidden = standing.members_hidden();
            let views = shown_views(
                &schema,
                repo::views(&mut *tx, page.id).await?,
                members_hidden,
            );
            tx.commit().await?;
            info!(
                user_id = %user,
                workspace_id = %workspace_id,
                database_id = %page.id,
                inline = new.inline,
                "database created"
            );
            Ok(DatabaseView {
                page: PageView {
                    page,
                    access: PageAccess::Edit,
                    members_hidden,
                },
                inline: new.inline,
                properties: schema.properties,
                views,
            })
        })
        .await
    }

    /// A database the caller sees: its page, its properties and its views. A guest reads
    /// the views without the people their settings name.
    pub async fn database(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<DatabaseView, DatabaseError> {
        self.observe("get_database", async {
            let mut conn = self.pages().pool().acquire().await?;
            let opened = self.open(&mut conn, principal, id, false).await?;
            Self::check(&opened.context, Action::ViewPage)?;
            let views = shown_views(
                &opened.schema,
                repo::views(&mut *conn, id).await?,
                opened.context.members_hidden(),
            );
            Ok(DatabaseView {
                inline: opened.database.inline,
                properties: opened.schema.properties,
                views,
                page: opened.context.view()?,
            })
        })
        .await
    }

    /// Adds a property, for those who edit the database, placed among its properties.
    pub async fn add_property(
        &self,
        principal: &Principal,
        database_id: Uuid,
        new: NewProperty,
    ) -> Result<Property, DatabaseError> {
        self.observe("add_property", async {
            let mut tx = self.pages().pool().begin().await?;
            let opened = self.open(&mut tx, principal, database_id, true).await?;
            Self::check(&opened.context, Action::EditPage)?;
            let ty = new_type(&new.spec, new.options)?;
            let (name, ty) = opened.schema.check_new(&new.name, ty)?;
            let property = Property {
                id: Uuid::now_v7(),
                name,
                ty,
            };
            let key = place(&mut tx, &opened, None, new.placement).await?;
            repo::insert_property(&mut *tx, database_id, &property, &key)
                .await
                .map_err(name_taken)?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                property_id = %property.id,
                kind = %property.kind(),
                "database property added"
            );
            self.publish(database_id, &Event::SchemaChanged).await;
            Ok(property)
        })
        .await
    }

    /// Changes a property's name, kind, settings or options, for those who edit the
    /// database. Another kind converts each row's value, as the domain's conversions say;
    /// a change that loses values needs `confirm`, and one that would make a row's values
    /// larger than [`MAX_ROW_BYTES`](neon_domain::databases::MAX_ROW_BYTES) is refused. A
    /// guest, who knows no other member, does not change a person property's kind, whose
    /// values would become people's names. Options left out are cleared from the rows.
    /// Views that no longer fit the schema leave out what does not.
    pub async fn update_property(
        &self,
        principal: &Principal,
        database_id: Uuid,
        property_id: Uuid,
        change: PropertyChange,
    ) -> Result<Property, DatabaseError> {
        self.observe("update_property", async {
            let mut tx = self.pages().pool().begin().await?;
            let opened = self.open(&mut tx, principal, database_id, true).await?;
            Self::check(&opened.context, Action::EditPage)?;
            let current = opened
                .schema
                .get(property_id)
                .ok_or(DatabaseError::NotFound(NO_PROPERTY))?
                .clone();
            let mut property = current.clone();
            if let Some(name) = &change.name {
                property.name = opened.schema.check_rename(property_id, name)?;
            }
            let mut lost = 0;
            if let Some(spec) = &change.spec {
                opened.schema.check_retype(property_id, spec.kind)?;
                if spec.kind != current.kind() && change.options.is_some() {
                    return Err(DatabaseError::InvalidInput(
                        "change a property's type first, then its options".to_owned(),
                    ));
                }
                let names_people = current.kind().names_people() && current.kind().is_stored();
                if names_people && spec.kind != current.kind() && opened.context.members_hidden() {
                    return Err(DatabaseError::Forbidden(
                        "guests know no other member: they do not change the type of a property \
                         that names people",
                    ));
                }
                let target = settings_type(spec)?;
                let values = stored_values(&mut tx, database_id, &current).await?;
                let names = if names_people {
                    member_names(&mut tx, opened.database.workspace_id).await?
                } else {
                    HashMap::new()
                };
                let conversion = convert(&current, target, &values, &names, &mut Uuid::now_v7);
                if conversion.lost > 0 && !change.confirm {
                    return Err(DatabaseError::LossyChange {
                        lost: conversion.lost,
                    });
                }
                lost = conversion.lost;
                property.ty = conversion.ty;
                write_values(&mut tx, property_id, conversion.values).await?;
            }
            if let Some(specs) = change.options {
                let kind = property.kind();
                let options = check_options(kind, property.ty.options(), specs, &mut Uuid::now_v7)?;
                let removed = opened.schema.removed_options(property_id, &options);
                if let Some(current) = property.ty.options_mut() {
                    *current = options;
                }
                if !removed.is_empty() {
                    clear_options(&mut tx, database_id, &property, &removed).await?;
                }
            }
            repo::update_property(&mut *tx, &property)
                .await
                .map_err(name_taken)?;
            let mut schema = opened.schema.clone();
            if let Some(slot) = schema.properties.iter_mut().find(|p| p.id == property_id) {
                *slot = property.clone();
            }
            let refit = refit_views(&mut tx, database_id, &schema).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                property_id = %property_id,
                kind = %property.kind(),
                lost,
                views = refit,
                "database property changed"
            );
            self.publish(database_id, &Event::SchemaChanged).await;
            Ok(property)
        })
        .await
    }

    /// Places a property elsewhere among the database's properties.
    pub async fn move_property(
        &self,
        principal: &Principal,
        database_id: Uuid,
        property_id: Uuid,
        placement: Placement,
    ) -> Result<Property, DatabaseError> {
        self.observe("move_property", async {
            let mut tx = self.pages().pool().begin().await?;
            let opened = self.open(&mut tx, principal, database_id, true).await?;
            Self::check(&opened.context, Action::EditPage)?;
            let property = opened
                .schema
                .get(property_id)
                .ok_or(DatabaseError::NotFound(NO_PROPERTY))?
                .clone();
            let key = place(&mut tx, &opened, Some(property_id), placement).await?;
            repo::set_property_positions(&mut *tx, &[property_id], &[key]).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                property_id = %property_id,
                "database property moved"
            );
            self.publish(database_id, &Event::SchemaChanged).await;
            Ok(property)
        })
        .await
    }

    /// Deletes a property but the title, with its values on every row. Views leave out
    /// what named it.
    pub async fn delete_property(
        &self,
        principal: &Principal,
        database_id: Uuid,
        property_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.observe("delete_property", async {
            let mut tx = self.pages().pool().begin().await?;
            let opened = self.open(&mut tx, principal, database_id, true).await?;
            Self::check(&opened.context, Action::EditPage)?;
            opened.schema.check_delete(property_id)?;
            repo::delete_property(&mut *tx, property_id).await?;
            let rows = repo::remove_cells_of(&mut *tx, database_id, property_id).await?;
            let mut schema = opened.schema.clone();
            schema
                .properties
                .retain(|property| property.id != property_id);
            let refit = refit_views(&mut tx, database_id, &schema).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                property_id = %property_id,
                rows,
                views = refit,
                "database property deleted"
            );
            self.publish(database_id, &Event::SchemaChanged).await;
            Ok(())
        })
        .await
    }

    /// Tells those following the database of a change, once it committed.
    pub(crate) async fn publish(&self, database_id: Uuid, event: &Event<'_>) {
        events::publish(
            self.pages().broadcaster().as_ref(),
            database_id,
            None,
            event,
        );
    }
}

/// A view's settings as the database keeps them.
pub(crate) fn config_json(spec: &ViewSpec) -> Result<Json, DatabaseError> {
    serde_json::to_value(spec)
        .map_err(|error| DatabaseError::Internal(format!("view settings: {error}")))
}

/// The key for a property placed among the others, `except` the one that moves, saving
/// fresh keys first when they grew too long.
async fn place(
    conn: &mut PgConnection,
    opened: &Opened,
    except: Option<Uuid>,
    placement: Placement,
) -> Result<String, DatabaseError> {
    let list: Vec<(Uuid, String)> = opened
        .positions
        .iter()
        .filter(|(id, _)| Some(*id) != except)
        .cloned()
        .collect();
    let (key, fresh) = key_for(
        &list,
        placement,
        "the property to place it beside is not one of the database's",
    )?;
    if !fresh.is_empty() {
        let (ids, keys): (Vec<Uuid>, Vec<String>) = fresh.into_iter().unzip();
        repo::set_property_positions(&mut *conn, &ids, &keys).await?;
    }
    Ok(key)
}

/// Another property's name, from the database's constraint, when two changes race.
fn name_taken(error: sqlx::Error) -> DatabaseError {
    if is_unique_violation(&error, repo::PROPERTY_NAME_TAKEN) {
        DatabaseError::NameTaken("the database has a property of that name already".to_owned())
    } else {
        DatabaseError::Database(error)
    }
}

/// The values every row of the database, in the trash or not, keeps of a property.
pub(crate) async fn stored_values(
    conn: &mut PgConnection,
    database_id: Uuid,
    property: &Property,
) -> Result<Vec<(Uuid, Value)>, DatabaseError> {
    Ok(repo::cells_of(&mut *conn, database_id, property.id)
        .await?
        .into_iter()
        .filter_map(|(row, json)| Some((row, Value::stored(&property.ty, &json)?)))
        .collect())
}

/// Writes the new value of a property on each row; `None` removes it. Refuses values that
/// would make a row larger than [`MAX_ROW_BYTES`](neon_domain::databases::MAX_ROW_BYTES),
/// which the transaction then undoes.
pub(crate) async fn write_values(
    conn: &mut PgConnection,
    property_id: Uuid,
    values: Vec<(Uuid, Option<Value>)>,
) -> Result<(), DatabaseError> {
    if values.is_empty() {
        return Ok(());
    }
    let (rows, values): (Vec<Uuid>, Vec<Json>) = values
        .into_iter()
        .map(|(row, value)| (row, value.map_or(Json::Null, |value| value.to_json())))
        .unzip();
    let largest = repo::set_cells_of(&mut *conn, property_id, &rows, &values).await?;
    check_size(largest)
}

/// The display names of a workspace's members, by id.
pub(crate) async fn member_names(
    conn: &mut PgConnection,
    workspace_id: Uuid,
) -> Result<HashMap<Uuid, String>, DatabaseError> {
    Ok(repo::member_names(&mut *conn, workspace_id)
        .await?
        .into_iter()
        .collect())
}

/// Clears deleted options from the rows: a select's or status's value, or the options of a
/// multi-select's.
async fn clear_options(
    conn: &mut PgConnection,
    database_id: Uuid,
    property: &Property,
    removed: &[Uuid],
) -> Result<(), DatabaseError> {
    // Read as ids: the options removed are no longer the property's.
    let mut changed = Vec::new();
    for (row, json) in repo::cells_of(&mut *conn, database_id, property.id).await? {
        let ids: Vec<Uuid> = match &json {
            Json::String(text) => Uuid::try_parse(text).into_iter().collect(),
            Json::Array(items) => items
                .iter()
                .filter_map(|item| item.as_str().and_then(|text| Uuid::try_parse(text).ok()))
                .collect(),
            _ => Vec::new(),
        };
        if !ids.iter().any(|id| removed.contains(id)) {
            continue;
        }
        let kept: Vec<Uuid> = ids.into_iter().filter(|id| !removed.contains(id)).collect();
        let value = match (property.kind(), kept.as_slice()) {
            (_, []) => None,
            (PropertyKind::MultiSelect, _) => Some(Value::Choices(kept)),
            (_, [first, ..]) => Some(Value::Choice(*first)),
        };
        changed.push((row, value));
    }
    write_values(conn, property.id, changed).await
}

/// Saves each view whose settings no longer fit the schema with what does, and says how
/// many changed.
async fn refit_views(
    conn: &mut PgConnection,
    database_id: Uuid,
    schema: &Schema,
) -> Result<usize, DatabaseError> {
    let mut changed = 0;
    for mut stored in repo::views(&mut *conn, database_id).await? {
        let spec: ViewSpec = serde_json::from_value(stored.config.clone()).unwrap_or_default();
        let config = config_json(&spec.fit(schema, stored.layout).to_spec())?;
        if config != stored.config {
            stored.config = config;
            repo::update_view(&mut *conn, &stored).await?;
            changed += 1;
        }
    }
    Ok(changed)
}
