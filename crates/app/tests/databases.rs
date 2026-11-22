//! Databases: making them, their properties of every type and their values, type changes,
//! options, views, queries, rows made, changed, moved, trashed and found by unique id,
//! who may do what, the cap on rows, the changes told to followers, and the metrics.

mod support;

use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::databases::{
    DatabaseError, DatabaseView, GroupMove, NewDatabase, NewProperty, NewRow, NewView,
    PropertyChange, PropertySpec, QueryInput, RowMove, RowView, ViewChange,
};
use neon_app::pages::{Location, NewPage, PageError};
use neon_app::sync::{Broadcaster, ClientId, Next, Outbox, OutboxReceiver, Room};
use neon_app::workspaces::NewTeamspace;
use neon_domain::auth::Principal;
use neon_domain::databases::filter::{FilterSpec, Operator};
use neon_domain::databases::sort::{Direction, SortSpec};
use neon_domain::databases::view::GroupingSpec;
use neon_domain::databases::{
    ChoiceSpec, Layout, MAX_NAME, MAX_ROW_BYTES, MAX_ROWS, MAX_TEXT, NumberFormat, PropertyKind,
    StatusGroup, ViewSpec,
};
use neon_domain::pages::{PageAccess, PageKind, Placement};
use neon_domain::workspaces::{TeamspaceAccess, WorkspaceRole};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use support::Harness;
use uuid::Uuid;

/// Ada owns Design, with Dee as a member and Gus as a guest. Cy made Secret, a private
/// teamspace Dee is not in. Tasks is a database in General.
struct Team {
    h: Harness,
    id: Uuid,
    general: Uuid,
    secret: Uuid,
    ada: Principal,
    cy: Principal,
    dee: Principal,
    gus: Principal,
    tasks: DatabaseView,
}

async fn team(pool: PgPool) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let cy = h.person("cy@example.com").await;
    let dee = h.person("dee@example.com").await;
    let gus = h.person("gus@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;
    h.bring_in(&ada, id, &dee, WorkspaceRole::Member).await;
    h.bring_in(&ada, id, &gus, WorkspaceRole::Guest).await;
    let general = h
        .workspaces
        .list_teamspaces(&ada, id)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    let secret = h
        .workspaces
        .create_teamspace(
            &cy,
            id,
            NewTeamspace {
                name: "Secret".to_owned(),
                icon: None,
                hue: None,
                access: TeamspaceAccess::Private,
            },
        )
        .await
        .expect("created")
        .teamspace
        .id;
    let tasks = h
        .databases
        .create_database(
            &ada,
            id,
            Location::Teamspace(general),
            Placement::Last,
            NewDatabase {
                title: "Tasks".to_owned(),
                icon: Some("✅".to_owned()),
                inline: false,
            },
        )
        .await
        .expect("created");
    Team {
        h,
        id,
        general,
        secret,
        ada,
        cy,
        dee,
        gus,
        tasks,
    }
}

fn spec(kind: PropertyKind) -> PropertySpec {
    PropertySpec {
        kind,
        number_format: None,
        currency: None,
        prefix: None,
    }
}

fn option(name: &str) -> ChoiceSpec {
    ChoiceSpec {
        id: None,
        name: name.to_owned(),
        hue: None,
        group: None,
    }
}

fn values(json: Value) -> Map<String, Value> {
    json.as_object().cloned().expect("an object")
}

impl Team {
    fn db(&self) -> Uuid {
        self.tasks.page.page.id
    }

    fn property(&self, name: &str) -> Uuid {
        self.tasks
            .properties
            .iter()
            .find(|property| property.name == name)
            .map(|property| property.id)
            .unwrap_or_else(|| panic!("no property {name}"))
    }

    fn option_id(&self, property: &str, name: &str) -> Uuid {
        self.tasks
            .properties
            .iter()
            .find(|p| p.name == property)
            .and_then(|p| p.ty.option(name))
            .map(|option| option.id)
            .unwrap_or_else(|| panic!("no option {name} of {property}"))
    }

    fn board(&self) -> Uuid {
        self.tasks.views[1].id
    }

    async fn add(
        &mut self,
        name: &str,
        spec: PropertySpec,
        options: Option<Vec<ChoiceSpec>>,
    ) -> Uuid {
        let property = self
            .h
            .databases
            .add_property(
                &self.ada,
                self.db(),
                NewProperty {
                    name: name.to_owned(),
                    spec,
                    options,
                    placement: Placement::Last,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        self.reload().await;
        property.id
    }

    async fn reload(&mut self) {
        self.tasks = self
            .h
            .databases
            .database(&self.ada, self.db())
            .await
            .expect("read");
    }

    async fn row(&self, by: &Principal, values: Value) -> Result<RowView, DatabaseError> {
        self.h
            .databases
            .create_row(
                by,
                self.db(),
                NewRow {
                    values: self::values(values),
                    ..NewRow::default()
                },
            )
            .await
    }

    async fn made_by(&self, by: &Principal) -> Result<RowView, DatabaseError> {
        self.row(by, json!({"Name": "More"})).await
    }

    async fn made(&self, title: &str) -> RowView {
        self.row(&self.ada, json!({"Name": title}))
            .await
            .expect("a row")
    }

    async fn update(
        &self,
        by: &Principal,
        row: Uuid,
        values: Value,
    ) -> Result<RowView, DatabaseError> {
        self.h
            .databases
            .update_row(by, row, self::values(values))
            .await
    }

    async fn query(&self, by: &Principal, input: QueryInput) -> Vec<String> {
        self.h
            .databases
            .query(by, self.db(), input)
            .await
            .expect("queried")
            .rows
            .into_iter()
            .map(|row| row.title)
            .collect()
    }

    async fn titles(&self) -> Vec<String> {
        self.query(&self.ada, QueryInput::default()).await
    }
}

fn condition(property: &str, operator: Operator, value: Value) -> FilterSpec {
    FilterSpec::Condition {
        property: property.to_owned(),
        operator,
        value,
    }
}

#[track_caller]
fn assert_code<T: std::fmt::Debug>(result: Result<T, DatabaseError>, code: &str) {
    match result {
        Err(error) => assert_eq!(error.code(), code, "{error}"),
        Ok(value) => panic!("expected {code}, got {value:?}"),
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_database_starts_with_a_title_a_status_a_table_and_a_board(pool: PgPool) {
    let t = team(pool).await;
    let tasks = &t.tasks;
    assert_eq!(tasks.page.page.kind, PageKind::Database);
    assert_eq!(tasks.page.page.title, "Tasks");
    assert_eq!(tasks.page.access, PageAccess::Edit);
    assert!(!tasks.inline);
    let properties: Vec<(&str, PropertyKind)> = tasks
        .properties
        .iter()
        .map(|p| (p.name.as_str(), p.kind()))
        .collect();
    assert_eq!(
        properties,
        [
            ("Name", PropertyKind::Title),
            ("Status", PropertyKind::Status)
        ]
    );
    let options: Vec<(&str, Option<StatusGroup>)> = tasks.properties[1]
        .ty
        .options()
        .iter()
        .map(|o| (o.name.as_str(), o.group()))
        .collect();
    assert_eq!(
        options,
        [
            ("Not started", Some(StatusGroup::ToDo)),
            ("In progress", Some(StatusGroup::InProgress)),
            ("Done", Some(StatusGroup::Done))
        ]
    );
    let views: Vec<(&str, Layout)> = tasks
        .views
        .iter()
        .map(|v| (v.name.as_str(), v.layout))
        .collect();
    assert_eq!(views, [("Table", Layout::Table), ("Board", Layout::Board)]);
    assert_eq!(
        tasks.views[1]
            .settings
            .grouping
            .as_ref()
            .map(|g| g.property),
        Some(t.property("Status"))
    );

    // The database is in the tree, its rows are not.
    t.made("A row").await;
    let tree =
        t.h.pages
            .teamspace_tree(&t.dee, t.general, None, None)
            .await
            .expect("listed");
    let listed: Vec<(&str, PageKind, i64)> = tree
        .pages
        .iter()
        .map(|p| (p.title.as_str(), p.kind, p.child_count))
        .collect();
    assert_eq!(listed, [("Tasks", PageKind::Database, 0)]);
    let read = t.h.databases.database(&t.dee, t.db()).await.expect("read");
    assert_eq!(read.page.access, PageAccess::Edit);
    assert_eq!(read.views.len(), 2);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn inline_databases_are_embedded_in_the_page_they_are_under(pool: PgPool) {
    let t = team(pool).await;
    let notes =
        t.h.pages
            .create_page(
                &t.ada,
                t.id,
                Location::Teamspace(t.general),
                Placement::Last,
                NewPage {
                    title: "Notes".to_owned(),
                    ..NewPage::default()
                },
            )
            .await
            .expect("a page");
    let make = |location, inline| {
        t.h.databases.create_database(
            &t.ada,
            t.id,
            location,
            Placement::Last,
            NewDatabase {
                title: "Inline".to_owned(),
                icon: None,
                inline,
            },
        )
    };
    assert_code(
        make(Location::Teamspace(t.general), true).await,
        "invalid_input",
    );
    assert_code(make(Location::Under(t.db()), false).await, "invalid_input");
    let inline = make(Location::Under(notes.page.id), true)
        .await
        .expect("made");
    assert!(inline.inline);
    assert_eq!(inline.page.page.parent_id, Some(notes.page.id));
    // Moved out from under its page, it is a page of its own.
    t.h.pages
        .move_page(
            &t.ada,
            inline.page.page.id,
            Location::Teamspace(t.general),
            Placement::Last,
        )
        .await
        .expect("moved");
    let moved =
        t.h.databases
            .database(&t.ada, inline.page.page.id)
            .await
            .expect("read");
    assert!(!moved.inline);
    // A page is no database.
    assert_code(
        t.h.databases.database(&t.ada, notes.page.id).await,
        "not_found",
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn nothing_goes_under_a_database_or_a_row_but_its_rows(pool: PgPool) {
    let t = team(pool).await;
    let row = t.made("Row").await;
    let page =
        t.h.pages
            .create_page(
                &t.ada,
                t.id,
                Location::Teamspace(t.general),
                Placement::Last,
                NewPage::default(),
            )
            .await
            .expect("a page");
    for under in [t.db(), row.id] {
        let made =
            t.h.pages
                .create_page(
                    &t.ada,
                    t.id,
                    Location::Under(under),
                    Placement::Last,
                    NewPage::default(),
                )
                .await;
        assert!(matches!(made, Err(PageError::InvalidInput(_))), "{made:?}");
        let moved =
            t.h.pages
                .move_page(
                    &t.ada,
                    page.page.id,
                    Location::Under(under),
                    Placement::Last,
                )
                .await;
        assert!(
            matches!(moved, Err(PageError::InvalidInput(_))),
            "{moved:?}"
        );
        let place =
            t.h.pages
                .check_place(&t.ada, t.id, Location::Under(under))
                .await;
        assert!(
            matches!(place, Err(PageError::InvalidInput(_))),
            "{place:?}"
        );
    }
    let row_moved =
        t.h.pages
            .move_page(
                &t.ada,
                row.id,
                Location::Teamspace(t.general),
                Placement::Last,
            )
            .await;
    assert!(
        matches!(row_moved, Err(PageError::InvalidInput(_))),
        "{row_moved:?}"
    );
    // The row is a page all the same.
    let read = t.h.pages.page(&t.dee, row.id).await.expect("a page");
    assert_eq!(read.page.kind, PageKind::Row);
    assert_eq!(read.page.parent_id, Some(t.db()));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn properties_of_every_type_take_their_values(pool: PgPool) {
    let mut t = team(pool).await;
    let text = t.add("Notes", spec(PropertyKind::Text), None).await;
    let number = t
        .add(
            "Budget",
            PropertySpec {
                number_format: Some(NumberFormat::Currency),
                currency: Some("eur".into()),
                ..spec(PropertyKind::Number)
            },
            None,
        )
        .await;
    let select = t
        .add(
            "Type",
            spec(PropertyKind::Select),
            Some(vec![option("Bug"), option("Feature")]),
        )
        .await;
    let multi = t
        .add(
            "Tags",
            spec(PropertyKind::MultiSelect),
            Some(vec![option("UI"), option("API")]),
        )
        .await;
    let review = t.add("Review", spec(PropertyKind::Status), None).await;
    let due = t.add("Due", spec(PropertyKind::Date), None).await;
    let owner = t.add("Owner", spec(PropertyKind::Person), None).await;
    let done = t.add("Shipped", spec(PropertyKind::Checkbox), None).await;
    let link = t.add("Link", spec(PropertyKind::Url), None).await;
    let id = t
        .add(
            "ID",
            PropertySpec {
                prefix: Some("nw".into()),
                ..spec(PropertyKind::UniqueId)
            },
            None,
        )
        .await;
    let created = t
        .add("Created", spec(PropertyKind::CreatedTime), None)
        .await;
    let creator = t.add("Creator", spec(PropertyKind::CreatedBy), None).await;
    let edited = t
        .add("Edited", spec(PropertyKind::LastEditedTime), None)
        .await;
    let editor = t
        .add("Editor", spec(PropertyKind::LastEditedBy), None)
        .await;
    assert_eq!(t.tasks.properties.len(), 16);
    let budget = t
        .tasks
        .properties
        .iter()
        .find(|p| p.id == number)
        .expect("budget");
    assert_eq!(budget.ty.kind(), PropertyKind::Number);
    // A status starts with the default options.
    let review_property = t
        .tasks
        .properties
        .iter()
        .find(|p| p.id == review)
        .expect("review");
    assert_eq!(review_property.ty.options().len(), 3);

    let dee = t.dee.user.id.to_string();
    let row = t
        .row(
            &t.dee,
            json!({
                "Name": "Fix login",
                "Notes": "Steps:\n1. sign in",
                "Budget": 12.5,
                "Type": "bug",
                "Tags": ["UI", "api"],
                "Status": "In progress",
                "Review": "Done",
                "Due": {"start": "2026-11-20", "end": "2026-11-30"},
                "Owner": [dee],
                "Shipped": true,
                "Link": "https://example.com/issue/1",
            }),
        )
        .await
        .expect("a row");
    assert_eq!(row.title, "Fix login");
    assert_eq!(row.number, 1);
    assert_eq!(row.unique_id, "NW-1");
    assert_eq!(row.access, PageAccess::Edit);
    let v = |id: Uuid| {
        row.values
            .get(&id.to_string())
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(v(t.property("Name")), json!("Fix login"));
    assert_eq!(v(text), json!("Steps:\n1. sign in"));
    assert_eq!(v(number), json!(12.5));
    assert_eq!(v(select), json!(t.option_id("Type", "Bug").to_string()));
    assert_eq!(
        v(multi),
        json!([
            t.option_id("Tags", "UI").to_string(),
            t.option_id("Tags", "API").to_string()
        ])
    );
    assert_eq!(
        v(t.property("Status")),
        json!(t.option_id("Status", "In progress").to_string())
    );
    assert_eq!(v(review), json!(t.option_id("Review", "Done").to_string()));
    assert_eq!(v(due), json!({"start": "2026-11-20", "end": "2026-11-30"}));
    assert_eq!(v(owner), json!([dee]));
    assert_eq!(v(done), json!(true));
    assert_eq!(v(link), json!("https://example.com/issue/1"));
    assert_eq!(v(id), json!("NW-1"));
    assert_eq!(v(created), json!("2026-11-22T12:00:00Z"));
    assert_eq!(v(edited), json!("2026-11-22T12:00:00Z"));
    assert_eq!(v(creator), json!(dee));
    assert_eq!(v(editor), json!(dee));
    assert_eq!(row.created_by, Some(t.dee.user.id));

    // Read back as it was written.
    let read = t.h.databases.row(&t.ada, row.id).await.expect("read");
    assert_eq!(read.values, row.values);
    // Cleared values are no values.
    let cleared = t
        .update(
            &t.ada,
            row.id,
            json!({"Notes": null, "Shipped": false, "Tags": []}),
        )
        .await
        .expect("cleared");
    for gone in [text, done, multi] {
        assert!(!cleared.values.contains_key(&gone.to_string()));
    }
    assert_eq!(
        cleared.values[&editor.to_string()],
        json!(t.ada.user.id.to_string())
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn values_are_checked_against_their_types(pool: PgPool) {
    let mut t = team(pool).await;
    t.add("Points", spec(PropertyKind::Number), None).await;
    t.add("Owner", spec(PropertyKind::Person), None).await;
    t.add("Link", spec(PropertyKind::Url), None).await;
    t.add("Created", spec(PropertyKind::CreatedTime), None)
        .await;
    let outsider = t.h.person("eve@example.com").await;
    let row = t.made("Row").await;
    for bad in [
        json!({"Points": "five"}),
        json!({"Status": "Blocked"}),
        json!({"Nope": 1}),
        json!({"Created": "2026-11-22T12:00:00Z"}),
        json!({"Link": "javascript:alert(1)"}),
        json!({"Name": "two\nlines"}),
        json!({"Owner": [outsider.user.id.to_string()]}),
        json!({"Owner": ["not an id"]}),
        json!({}),
    ] {
        assert_code(t.update(&t.ada, row.id, bad.clone()).await, "invalid_input");
    }
    assert_code(
        t.row(&t.ada, json!({"Points": true})).await,
        "invalid_input",
    );
    // Nothing of a refused change was kept: the row has its title and when it was made.
    let read = t.h.databases.row(&t.ada, row.id).await.expect("read");
    assert_eq!(read.values.len(), 2, "{:?}", read.values);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn row_numbers_grow_are_never_given_again_and_find_their_row(pool: PgPool) {
    let mut t = team(pool).await;
    let first = t.made("One").await;
    let second = t.made("Two").await;
    let third = t.made("Three").await;
    assert_eq!((first.number, second.number, third.number), (1, 2, 3));
    // Without a unique id property, the number alone is the id.
    assert_eq!(third.unique_id, "3");
    t.h.pages
        .trash_page(&t.ada, third.id)
        .await
        .expect("trashed");
    t.h.pages
        .delete_page(&t.ada, third.id)
        .await
        .expect("deleted");
    let fourth = t.made("Four").await;
    assert_eq!(fourth.number, 4);

    t.add(
        "ID",
        PropertySpec {
            prefix: Some("NW".into()),
            ..spec(PropertyKind::UniqueId)
        },
        None,
    )
    .await;
    let found = |text: &'static str| t.h.databases.row_by_unique_id(&t.dee, t.db(), text);
    assert_eq!(found("NW-2").await.expect("found").id, second.id);
    assert_eq!(found("nw-2").await.expect("found").id, second.id);
    assert_eq!(found("2").await.expect("found").unique_id, "NW-2");
    assert_code(found("NW-3").await, "not_found");
    assert_code(found("XX-2").await, "invalid_input");
    assert_code(found("NW-x").await, "invalid_input");
    // A second unique id is refused.
    let second_id =
        t.h.databases
            .add_property(
                &t.ada,
                t.db(),
                NewProperty {
                    name: "Other".into(),
                    spec: spec(PropertyKind::UniqueId),
                    options: None,
                    placement: Placement::Last,
                },
            )
            .await;
    assert_code(second_id, "invalid_input");
    // Hidden from those who may not see the database.
    let outsider = t.h.person("eve@example.com").await;
    assert_code(
        t.h.databases
            .row_by_unique_id(&outsider, t.db(), "NW-2")
            .await,
        "not_found",
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_change_sets_only_the_properties_it_names(pool: PgPool) {
    let mut t = team(pool).await;
    let notes = t.add("Notes", spec(PropertyKind::Text), None).await;
    let points = t.add("Points", spec(PropertyKind::Number), None).await;
    let row = t.made("Draft").await;
    // Two people change different properties: both hold.
    t.update(&t.ada, row.id, json!({"Notes": "from Ada"}))
        .await
        .expect("changed");
    t.update(&t.dee, row.id, json!({"Points": 3}))
        .await
        .expect("changed");
    // Of two changes to one property, the last wins.
    t.update(&t.dee, row.id, json!({"Notes": "from Dee"}))
        .await
        .expect("changed");
    let renamed = t
        .update(&t.ada, row.id, json!({"Name": "Final"}))
        .await
        .expect("renamed");
    assert_eq!(renamed.values[&notes.to_string()], json!("from Dee"));
    assert_eq!(renamed.values[&points.to_string()], json!(3.0));
    assert_eq!(renamed.title, "Final");
    assert_eq!(renamed.last_edited_by, Some(t.ada.user.id));
    // The title is the row's page's.
    let page = t.h.pages.page(&t.ada, row.id).await.expect("a page");
    assert_eq!(page.page.title, "Final");
    assert_eq!(page.page.last_edited_by, Some(t.ada.user.id));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn properties_are_renamed_placed_and_deleted_with_their_values(pool: PgPool) {
    let mut t = team(pool).await;
    let notes = t.add("Notes", spec(PropertyKind::Text), None).await;
    let row = t.made("Row").await;
    t.update(&t.ada, row.id, json!({"Notes": "kept"}))
        .await
        .expect("changed");
    let renamed =
        t.h.databases
            .update_property(
                &t.ada,
                t.db(),
                notes,
                PropertyChange {
                    name: Some("Details".into()),
                    ..PropertyChange::default()
                },
            )
            .await
            .expect("renamed");
    assert_eq!(renamed.name, "Details");
    let taken =
        t.h.databases
            .update_property(
                &t.ada,
                t.db(),
                notes,
                PropertyChange {
                    name: Some("status".into()),
                    ..PropertyChange::default()
                },
            )
            .await;
    assert_code(taken, "name_taken");
    t.h.databases
        .move_property(&t.ada, t.db(), notes, Placement::First)
        .await
        .expect("moved");
    t.reload().await;
    let names: Vec<&str> = t.tasks.properties.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Details", "Name", "Status"]);

    // A view sorting by it loses the sort once it is deleted, and the rows its values.
    t.h.databases
        .update_view(
            &t.ada,
            t.db(),
            t.tasks.views[0].id,
            ViewChange {
                sorts: Some(vec![SortSpec {
                    property: "Details".into(),
                    direction: Direction::Descending,
                }]),
                ..ViewChange::default()
            },
        )
        .await
        .expect("sorted");
    t.h.databases
        .delete_property(&t.ada, t.db(), notes)
        .await
        .expect("deleted");
    t.reload().await;
    assert!(t.tasks.views[0].settings.sorts.is_empty());
    assert_eq!(t.tasks.properties.len(), 2);
    let read = t.h.databases.row(&t.ada, row.id).await.expect("read");
    assert!(!read.values.contains_key(&notes.to_string()));
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM database_rows WHERE cells ? $1")
        .bind(notes.to_string())
        .fetch_one(&t.h.pool)
        .await
        .expect("counted");
    assert_eq!(kept, 0);
    assert_code(
        t.h.databases
            .delete_property(&t.ada, t.db(), t.property("Name"))
            .await,
        "invalid_input",
    );
    assert_code(
        t.h.databases
            .delete_property(&t.ada, t.db(), Uuid::nil())
            .await,
        "not_found",
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn type_changes_convert_values_and_ask_before_losing_any(pool: PgPool) {
    let mut t = team(pool).await;
    let notes = t.add("Estimate", spec(PropertyKind::Text), None).await;
    let rows = [t.made("A").await, t.made("B").await, t.made("C").await];
    for (row, text) in rows.iter().zip(["3", "1,500", "a lot"]) {
        t.update(&t.ada, row.id, json!({"Estimate": text}))
            .await
            .expect("set");
    }
    let to_number = |confirm| PropertyChange {
        spec: Some(spec(PropertyKind::Number)),
        confirm,
        ..PropertyChange::default()
    };
    let refused =
        t.h.databases
            .update_property(&t.ada, t.db(), notes, to_number(false))
            .await;
    assert!(
        matches!(refused, Err(DatabaseError::LossyChange { lost: 1 })),
        "{refused:?}"
    );
    // Nothing changed.
    let read = t.h.databases.row(&t.ada, rows[2].id).await.expect("read");
    assert_eq!(read.values[&notes.to_string()], json!("a lot"));
    let changed =
        t.h.databases
            .update_property(&t.ada, t.db(), notes, to_number(true))
            .await
            .expect("changed");
    assert_eq!(changed.kind(), PropertyKind::Number);
    let values: Vec<Value> =
        t.h.databases
            .query(&t.ada, t.db(), QueryInput::default())
            .await
            .expect("queried")
            .rows
            .iter()
            .map(|row| {
                row.values
                    .get(&notes.to_string())
                    .cloned()
                    .unwrap_or(Value::Null)
            })
            .collect();
    assert_eq!(values, [json!(3.0), json!(1500.0), Value::Null]);

    // Numbers become text without loss, then options named after them.
    t.h.databases
        .update_property(
            &t.ada,
            t.db(),
            notes,
            PropertyChange {
                spec: Some(spec(PropertyKind::Text)),
                ..PropertyChange::default()
            },
        )
        .await
        .expect("text again");
    let select =
        t.h.databases
            .update_property(
                &t.ada,
                t.db(),
                notes,
                PropertyChange {
                    spec: Some(spec(PropertyKind::Select)),
                    ..PropertyChange::default()
                },
            )
            .await
            .expect("a select");
    let names: Vec<&str> = select
        .ty
        .options()
        .iter()
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(names, ["3", "1500"]);
    // The title keeps its type, and options come after a type change, not with it.
    assert_code(
        t.h.databases
            .update_property(
                &t.ada,
                t.db(),
                t.property("Name"),
                PropertyChange {
                    spec: Some(spec(PropertyKind::Text)),
                    ..PropertyChange::default()
                },
            )
            .await,
        "invalid_input",
    );
    assert_code(
        t.h.databases
            .update_property(
                &t.ada,
                t.db(),
                notes,
                PropertyChange {
                    spec: Some(spec(PropertyKind::Status)),
                    options: Some(vec![]),
                    ..PropertyChange::default()
                },
            )
            .await,
        "invalid_input",
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn options_keep_their_values_by_id_and_deleted_ones_are_cleared(pool: PgPool) {
    let mut t = team(pool).await;
    let tags = t
        .add(
            "Tags",
            spec(PropertyKind::MultiSelect),
            Some(vec![option("UI"), option("API"), option("Docs")]),
        )
        .await;
    let (ui, docs) = (t.option_id("Tags", "UI"), t.option_id("Tags", "Docs"));
    let done = t.option_id("Status", "Done");
    let row = t
        .row(
            &t.ada,
            json!({"Name": "R", "Tags": ["UI", "API"], "Status": "Done"}),
        )
        .await
        .expect("a row");
    let other = t
        .row(&t.ada, json!({"Name": "S", "Tags": ["API"]}))
        .await
        .expect("a row");
    // UI renamed, API deleted, a new option added, in a new order.
    let changed =
        t.h.databases
            .update_property(
                &t.ada,
                t.db(),
                tags,
                PropertyChange {
                    options: Some(vec![
                        ChoiceSpec {
                            id: Some(docs),
                            ..option("Docs")
                        },
                        ChoiceSpec {
                            id: Some(ui),
                            hue: Some(185),
                            ..option("Interface")
                        },
                        option("Infra"),
                    ]),
                    ..PropertyChange::default()
                },
            )
            .await
            .expect("changed");
    let names: Vec<&str> = changed
        .ty
        .options()
        .iter()
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(names, ["Docs", "Interface", "Infra"]);
    let read = t.h.databases.row(&t.ada, row.id).await.expect("read");
    assert_eq!(read.values[&tags.to_string()], json!([ui.to_string()]));
    let read = t.h.databases.row(&t.ada, other.id).await.expect("read");
    assert!(!read.values.contains_key(&tags.to_string()));

    // Deleting a status option clears it from the rows.
    let status = t.property("Status");
    let kept: Vec<ChoiceSpec> = t.tasks.properties[1]
        .ty
        .options()
        .iter()
        .filter(|o| o.id != done)
        .map(|o| ChoiceSpec {
            id: Some(o.id),
            name: o.name.clone(),
            hue: o.hue,
            group: o.group(),
        })
        .collect();
    t.h.databases
        .update_property(
            &t.ada,
            t.db(),
            status,
            PropertyChange {
                options: Some(kept),
                ..PropertyChange::default()
            },
        )
        .await
        .expect("changed");
    let read = t.h.databases.row(&t.ada, row.id).await.expect("read");
    assert!(!read.values.contains_key(&status.to_string()));
    // Two options of one name are refused.
    let twice =
        t.h.databases
            .update_property(
                &t.ada,
                t.db(),
                tags,
                PropertyChange {
                    options: Some(vec![option("A"), option("a")]),
                    ..PropertyChange::default()
                },
            )
            .await;
    assert_code(twice, "name_taken");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn views_are_made_changed_placed_duplicated_and_deleted(pool: PgPool) {
    let mut t = team(pool).await;
    let create = |name: &str, layout, settings: ViewSpec| {
        t.h.databases.create_view(
            &t.ada,
            t.db(),
            NewView {
                name: name.to_owned(),
                layout,
                settings,
                placement: Placement::Last,
            },
        )
    };
    assert_code(
        create("Bad board", Layout::Board, ViewSpec::default()).await,
        "invalid_input",
    );
    let list = create(
        "Open",
        Layout::List,
        ViewSpec {
            filter: Some(condition("Status", Operator::IsNot, json!("Done"))),
            ..ViewSpec::default()
        },
    )
    .await
    .expect("made");
    assert_eq!(list.layout, Layout::List);
    assert!(list.settings.filter.is_some());
    let copy =
        t.h.databases
            .duplicate_view(&t.ada, t.db(), t.board(), None)
            .await
            .expect("copied");
    assert_eq!(copy.name, "Board (copy)");
    assert_eq!(copy.settings.grouping, t.tasks.views[1].settings.grouping);
    t.reload().await;
    let names: Vec<&str> = t.tasks.views.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["Table", "Board", "Board (copy)", "Open"]);
    let changed =
        t.h.databases
            .update_view(
                &t.ada,
                t.db(),
                list.id,
                ViewChange {
                    name: Some("Everything".into()),
                    filter: Some(None),
                    layout: Some(Layout::Table),
                    ..ViewChange::default()
                },
            )
            .await
            .expect("changed");
    assert_eq!(
        (changed.name.as_str(), changed.layout),
        ("Everything", Layout::Table)
    );
    assert_eq!(changed.settings.filter, None);
    // A board keeps its grouping.
    assert_code(
        t.h.databases
            .update_view(
                &t.ada,
                t.db(),
                t.board(),
                ViewChange {
                    group: Some(None),
                    ..ViewChange::default()
                },
            )
            .await,
        "invalid_input",
    );
    t.h.databases
        .move_view(&t.ada, t.db(), list.id, Placement::First)
        .await
        .expect("moved");
    t.reload().await;
    assert_eq!(t.tasks.views[0].id, list.id);
    for view in t.tasks.views.clone().iter().skip(1) {
        t.h.databases
            .delete_view(&t.ada, t.db(), view.id)
            .await
            .expect("deleted");
    }
    assert_code(
        t.h.databases.delete_view(&t.ada, t.db(), list.id).await,
        "invalid_input",
    );
    assert_code(
        t.h.databases.delete_view(&t.ada, t.db(), Uuid::nil()).await,
        "not_found",
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn queries_filter_sort_group_and_page(pool: PgPool) {
    let mut t = team(pool).await;
    t.add("Points", spec(PropertyKind::Number), None).await;
    for (title, status, points) in [
        ("Login", "Done", 3),
        ("Search", "In progress", 8),
        ("Export", "Not started", 5),
        ("Import", "Done", 1),
        ("Themes", "", 2),
    ] {
        t.row(
            &t.ada,
            json!({"Name": title, "Status": status, "Points": points}),
        )
        .await
        .expect("a row");
    }
    // A board grouped by status, its saved filter and sorts, and the groups in its order.
    t.h.databases
        .update_view(
            &t.ada,
            t.db(),
            t.board(),
            ViewChange {
                filter: Some(Some(condition("Points", Operator::GreaterThan, json!(1)))),
                sorts: Some(vec![SortSpec {
                    property: "Points".into(),
                    direction: Direction::Descending,
                }]),
                group: Some(Some(GroupingSpec {
                    property: "Status".into(),
                    order: vec![Some("Done".into())],
                    hidden: vec![Some("Not started".into())],
                })),
                ..ViewChange::default()
            },
        )
        .await
        .expect("changed");
    let board =
        t.h.databases
            .query(
                &t.dee,
                t.db(),
                QueryInput {
                    view_id: Some(t.board()),
                    ..QueryInput::default()
                },
            )
            .await
            .expect("queried");
    let listed: Vec<(&str, Option<Option<Uuid>>)> = board
        .rows
        .iter()
        .map(|row| (row.title.as_str(), row.group))
        .collect();
    let done = Some(t.option_id("Status", "Done"));
    let doing = Some(t.option_id("Status", "In progress"));
    let todo = Some(t.option_id("Status", "Not started"));
    assert_eq!(
        listed,
        [
            ("Login", Some(done)),
            ("Themes", Some(None)),
            ("Search", Some(doing))
        ]
    );
    let groups: Vec<(Option<Uuid>, usize, bool)> = board
        .groups
        .expect("grouped")
        .iter()
        .map(|g| (g.key, g.count, g.hidden))
        .collect();
    assert_eq!(
        groups,
        [
            (done, 1, false),
            (None, 1, false),
            (todo, 1, true),
            (doing, 1, false)
        ]
    );
    assert_eq!(board.total, 3);

    // An ad-hoc filter and sort in place of the view's, by pages of two.
    let first =
        t.h.databases
            .query(
                &t.ada,
                t.db(),
                QueryInput {
                    filter: Some(FilterSpec::Or {
                        or: vec![
                            condition("Status", Operator::InGroup, json!("done")),
                            condition("Name", Operator::StartsWith, json!("ex")),
                        ],
                    }),
                    sorts: Some(vec![SortSpec {
                        property: "Name".into(),
                        direction: Direction::Ascending,
                    }]),
                    limit: Some(2),
                    ..QueryInput::default()
                },
            )
            .await
            .expect("queried");
    let titles: Vec<&str> = first.rows.iter().map(|row| row.title.as_str()).collect();
    assert_eq!(titles, ["Export", "Import"]);
    assert_eq!(first.total, 3);
    let rest = t
        .query(
            &t.ada,
            QueryInput {
                filter: Some(condition("Status", Operator::InGroup, json!("done"))),
                sorts: Some(vec![SortSpec {
                    property: "Name".into(),
                    direction: Direction::Ascending,
                }]),
                after: Some("1".into()),
                ..QueryInput::default()
            },
        )
        .await;
    assert_eq!(rest, ["Login"]);
    assert_eq!(first.next.as_deref(), Some("2"));
    // Without a view, every row in manual order.
    assert_eq!(
        t.titles().await,
        ["Login", "Search", "Export", "Import", "Themes"]
    );
    for bad in [
        QueryInput {
            filter: Some(condition("Nope", Operator::IsEmpty, Value::Null)),
            ..QueryInput::default()
        },
        QueryInput {
            limit: Some(0),
            ..QueryInput::default()
        },
        QueryInput {
            after: Some("x".into()),
            ..QueryInput::default()
        },
        QueryInput {
            utc_offset_minutes: Some(900),
            ..QueryInput::default()
        },
    ] {
        assert_code(
            t.h.databases.query(&t.ada, t.db(), bad).await,
            "invalid_input",
        );
    }
    assert_code(
        t.h.databases
            .query(
                &t.ada,
                t.db(),
                QueryInput {
                    view_id: Some(Uuid::nil()),
                    ..QueryInput::default()
                },
            )
            .await,
        "not_found",
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn dates_and_me_are_the_caller_s(pool: PgPool) {
    let mut t = team(pool).await;
    t.add("Due", spec(PropertyKind::Date), None).await;
    t.add("Owner", spec(PropertyKind::Person), None).await;
    // It is 2026-11-22 12:00 UTC: 2026-11-23 at UTC+13.
    t.row(
        &t.ada,
        json!({"Name": "Today", "Due": "2026-11-22", "Owner": [t.dee.user.id.to_string()]}),
    )
    .await
    .expect("a row");
    t.row(&t.ada, json!({"Name": "Tomorrow", "Due": "2026-11-23"}))
        .await
        .expect("a row");
    let due_today = |offset| QueryInput {
        filter: Some(condition("Due", Operator::Is, json!("today"))),
        utc_offset_minutes: Some(offset),
        ..QueryInput::default()
    };
    assert_eq!(t.query(&t.ada, due_today(0)).await, ["Today"]);
    assert_eq!(t.query(&t.ada, due_today(13 * 60)).await, ["Tomorrow"]);
    let mine = || QueryInput {
        filter: Some(condition("Owner", Operator::Contains, json!("me"))),
        ..QueryInput::default()
    };
    assert_eq!(t.query(&t.dee, mine()).await, ["Today"]);
    assert!(t.query(&t.ada, mine()).await.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn rows_move_among_the_rows_and_between_groups(pool: PgPool) {
    let mut t = team(pool).await;
    let owner = t.add("Owner", spec(PropertyKind::Person), None).await;
    let a = t.made("A").await;
    let b = t.made("B").await;
    let c = t.made("C").await;
    let moved =
        t.h.databases
            .move_row(
                &t.dee,
                c.id,
                RowMove {
                    placement: Some(Placement::Before(a.id)),
                    group: Some(GroupMove {
                        view_id: t.board(),
                        from: None,
                        to: Some(t.option_id("Status", "Done")),
                    }),
                },
            )
            .await
            .expect("moved");
    assert_eq!(
        moved.values[&t.property("Status").to_string()],
        json!(t.option_id("Status", "Done").to_string())
    );
    assert_eq!(t.titles().await, ["C", "A", "B"]);
    t.h.databases
        .move_row(
            &t.ada,
            a.id,
            RowMove {
                placement: Some(Placement::Last),
                group: None,
            },
        )
        .await
        .expect("moved");
    assert_eq!(t.titles().await, ["C", "B", "A"]);
    // Out of every group: no value.
    let cleared =
        t.h.databases
            .move_row(
                &t.ada,
                c.id,
                RowMove {
                    placement: None,
                    group: Some(GroupMove {
                        view_id: t.board(),
                        from: Some(t.option_id("Status", "Done")),
                        to: None,
                    }),
                },
            )
            .await
            .expect("moved");
    assert!(
        !cleared
            .values
            .contains_key(&t.property("Status").to_string())
    );

    // On a board by person, one person makes room for another and the others stay.
    let people =
        t.h.databases
            .create_view(
                &t.ada,
                t.db(),
                NewView {
                    name: "People".into(),
                    layout: Layout::Board,
                    settings: ViewSpec {
                        group: Some(GroupingSpec {
                            property: "Owner".into(),
                            order: vec![],
                            hidden: vec![],
                        }),
                        ..ViewSpec::default()
                    },
                    placement: Placement::Last,
                },
            )
            .await
            .expect("a board");
    let (ada, dee, cy) = (t.ada.user.id, t.dee.user.id, t.cy.user.id);
    t.update(
        &t.ada,
        b.id,
        json!({"Owner": [ada.to_string(), dee.to_string()]}),
    )
    .await
    .expect("set");
    let regrouped =
        t.h.databases
            .move_row(
                &t.ada,
                b.id,
                RowMove {
                    placement: None,
                    group: Some(GroupMove {
                        view_id: people.id,
                        from: Some(ada),
                        to: Some(cy),
                    }),
                },
            )
            .await
            .expect("moved");
    assert_eq!(
        regrouped.values[&owner.to_string()],
        json!([dee.to_string(), cy.to_string()])
    );
    let outsider = t.h.person("eve@example.com").await;
    for bad in [
        RowMove::default(),
        RowMove {
            placement: Some(Placement::After(Uuid::nil())),
            group: None,
        },
        RowMove {
            placement: None,
            group: Some(GroupMove {
                view_id: t.tasks.views[0].id,
                from: None,
                to: None,
            }),
        },
        RowMove {
            placement: None,
            group: Some(GroupMove {
                view_id: people.id,
                from: None,
                to: Some(outsider.user.id),
            }),
        },
        RowMove {
            placement: None,
            group: Some(GroupMove {
                view_id: t.board(),
                from: None,
                to: Some(Uuid::nil()),
            }),
        },
    ] {
        assert_code(
            t.h.databases.move_row(&t.ada, a.id, bad).await,
            "invalid_input",
        );
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn access_to_rows_comes_from_the_database_page(pool: PgPool) {
    let mut t = team(pool).await;
    t.add("Owner", spec(PropertyKind::Person), None).await;
    let row = t
        .row(
            &t.dee,
            json!({"Name": "Shared", "Owner": [t.dee.user.id.to_string()]}),
        )
        .await
        .expect("a row");
    let other = t.made("Other").await;

    // A guest reaches nothing until the database is shared with them.
    assert_code(t.h.databases.database(&t.gus, t.db()).await, "not_found");
    assert_code(t.h.databases.row(&t.gus, row.id).await, "not_found");
    t.h.pages
        .share_page(&t.ada, t.db(), t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    let seen = t.h.databases.database(&t.gus, t.db()).await.expect("seen");
    assert_eq!(seen.page.access, PageAccess::View);
    let queried =
        t.h.databases
            .query(&t.gus, t.db(), QueryInput::default())
            .await
            .expect("queried");
    // Without the people the rows name, or who made them.
    let first = &queried.rows[0];
    assert_eq!(first.values.len(), 1, "{:?}", first.values);
    assert_eq!(first.created_by, None);
    assert_eq!(first.access, PageAccess::View);
    assert_code(t.row(&t.gus, json!({"Name": "Nope"})).await, "forbidden");
    assert_code(
        t.update(&t.gus, row.id, json!({"Name": "Nope"})).await,
        "forbidden",
    );

    // A row shared to edit is theirs to change, but not to place among the others.
    t.h.pages
        .share_page(&t.ada, row.id, t.gus.user.id, PageAccess::Edit)
        .await
        .expect("shared");
    let listed =
        t.h.databases
            .query(&t.gus, t.db(), QueryInput::default())
            .await
            .expect("queried");
    let access: Vec<PageAccess> = listed.rows.iter().map(|row| row.access).collect();
    assert_eq!(access, [PageAccess::Edit, PageAccess::View]);
    t.update(&t.gus, row.id, json!({"Name": "Renamed by Gus"}))
        .await
        .expect("changed");
    assert_code(
        t.update(
            &t.gus,
            row.id,
            json!({"Owner": [t.gus.user.id.to_string()]}),
        )
        .await,
        "forbidden",
    );
    assert_code(
        t.h.databases
            .move_row(
                &t.gus,
                row.id,
                RowMove {
                    placement: Some(Placement::After(other.id)),
                    group: None,
                },
            )
            .await,
        "forbidden",
    );
    t.h.databases
        .move_row(
            &t.gus,
            row.id,
            RowMove {
                placement: None,
                group: Some(GroupMove {
                    view_id: t.board(),
                    from: None,
                    to: Some(t.option_id("Status", "Done")),
                }),
            },
        )
        .await
        .expect("moved to another group");
    assert_code(
        t.update(&t.gus, other.id, json!({"Name": "x"})).await,
        "forbidden",
    );

    // A database in a private teamspace is hidden from those outside it.
    let hidden =
        t.h.databases
            .create_database(
                &t.cy,
                t.id,
                Location::Teamspace(t.secret),
                Placement::Last,
                NewDatabase::default(),
            )
            .await
            .expect("made");
    let id = hidden.page.page.id;
    assert_code(t.h.databases.database(&t.dee, id).await, "not_found");
    assert_code(
        t.h.databases.query(&t.dee, id, QueryInput::default()).await,
        "not_found",
    );
    assert_code(
        t.h.databases
            .create_row(&t.dee, id, NewRow::default())
            .await,
        "not_found",
    );
    // Those who view a database do not change it.
    let mine =
        t.h.databases
            .create_database(
                &t.dee,
                t.id,
                Location::Private,
                Placement::Last,
                NewDatabase::default(),
            )
            .await
            .expect("made");
    t.h.pages
        .share_page(&t.dee, mine.page.page.id, t.cy.user.id, PageAccess::View)
        .await
        .expect("shared");
    assert_code(
        t.h.databases
            .add_property(
                &t.cy,
                mine.page.page.id,
                NewProperty {
                    name: "X".into(),
                    spec: spec(PropertyKind::Text),
                    options: None,
                    placement: Placement::Last,
                },
            )
            .await,
        "forbidden",
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn trashed_rows_leave_queries_and_come_back_with_their_database(pool: PgPool) {
    let t = team(pool).await;
    let a = t.made("A").await;
    t.made("B").await;
    t.h.pages.trash_page(&t.ada, a.id).await.expect("trashed");
    assert_eq!(t.titles().await, ["B"]);
    t.h.pages
        .restore_page(&t.ada, a.id)
        .await
        .expect("restored");
    assert_eq!(t.titles().await, ["A", "B"]);

    // A row trashed on its own stays in the trash with its database.
    t.h.pages.trash_page(&t.ada, a.id).await.expect("trashed");
    t.h.pages.trash_page(&t.ada, t.db()).await.expect("trashed");
    let restored = t.h.pages.restore_page(&t.ada, a.id).await;
    assert!(
        matches!(restored, Err(PageError::Forbidden(_))),
        "{restored:?}"
    );
    // An editor still reads the trashed database, with the rows that went with it.
    assert_eq!(t.titles().await, ["B"]);
    assert_code(t.made_by(&t.ada).await, "forbidden");
    assert_code(t.h.databases.database(&t.gus, t.db()).await, "not_found");
    t.h.pages
        .restore_page(&t.ada, t.db())
        .await
        .expect("restored");
    assert_eq!(t.titles().await, ["B"]);
    t.h.pages
        .restore_page(&t.ada, a.id)
        .await
        .expect("restored");
    assert_eq!(t.titles().await, ["A", "B"]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_database_holds_at_most_its_cap_of_rows(pool: PgPool) {
    let t = team(pool).await;
    let last = t.made("Last").await;
    // The rest, as the server would have made them, in one statement.
    let missing = i64::try_from(MAX_ROWS).expect("small") - 2;
    sqlx::query(
        "WITH made AS (
             INSERT INTO pages (id, workspace_id, kind, teamspace_id, parent_id, position, title,
                                font, small_text, full_width, created_at, updated_at,
                                content_updated_at)
             SELECT gen_random_uuid(), $1, 'row', $2, $3, 'a0' || lpad(n::text, 5, '0') || 'V',
                    '', 'default', false, false, now(), now(), now()
             FROM generate_series(1, $4) AS n
             RETURNING id, position
         )
         INSERT INTO database_rows (page_id, database_id, number, cells)
         SELECT id, $3, 100 + row_number() OVER (ORDER BY position), '{}'::jsonb FROM made",
    )
    .bind(t.id)
    .bind(t.general)
    .bind(t.db())
    .bind(missing)
    .execute(&t.h.pool)
    .await
    .expect("rows made");
    sqlx::query("UPDATE databases SET next_number = 100000 WHERE page_id = $1")
        .bind(t.db())
        .execute(&t.h.pool)
        .await
        .expect("numbered");
    let full = t.made("Full").await;
    assert_eq!(full.number, 100_000);
    assert_code(t.made_by(&t.ada).await, "too_many_rows");
    // Trashing one makes room; restoring it once the room is taken does not.
    t.h.pages
        .trash_page(&t.ada, last.id)
        .await
        .expect("trashed");
    t.made("Another").await;
    let restored = t.h.pages.restore_page(&t.ada, last.id).await;
    assert!(
        matches!(restored, Err(PageError::TooManyRows)),
        "{restored:?}"
    );
    let queried =
        t.h.databases
            .query(&t.ada, t.db(), QueryInput::default())
            .await
            .expect("queried");
    assert_eq!(queried.total, MAX_ROWS);
}

/// A connection in a database's room of the test's broadcaster.
struct Listener {
    receiver: OutboxReceiver,
}

impl Listener {
    fn join(t: &Team, members_hidden: bool) -> Self {
        let (outbox, receiver) = Outbox::new(64, 1 << 20);
        t.h.broadcaster.join(
            Room::Database {
                id: t.db(),
                members_hidden,
            },
            ClientId::next(),
            outbox,
        );
        Self { receiver }
    }

    /// The next message, which must have come already: each is sent before the change that
    /// made it returns.
    async fn next(&mut self) -> Value {
        let next = tokio::time::timeout(std::time::Duration::from_secs(5), self.receiver.next())
            .await
            .expect("a message came");
        match next {
            Next::Send(bytes) => serde_json::from_slice(&bytes).expect("JSON"),
            Next::Close(reason) => panic!("closed: {reason}"),
        }
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn every_change_is_told_to_those_following_and_guests_hear_no_people(pool: PgPool) {
    let mut t = team(pool).await;
    let owner = t.add("Owner", spec(PropertyKind::Person), None).await;
    let mut members = Listener::join(&t, false);
    let mut guests = Listener::join(&t, true);
    let db = t.db().to_string();

    let row = t
        .row(
            &t.ada,
            json!({"Name": "Fix login", "Owner": [t.dee.user.id.to_string()]}),
        )
        .await
        .expect("a row");
    let created = members.next().await;
    assert_eq!(created["type"], "row_created");
    assert_eq!(created["database_id"], db.as_str());
    assert_eq!(created["row"]["title"], "Fix login");
    assert_eq!(created["row"]["number"], 1);
    assert_eq!(
        created["row"]["values"][owner.to_string()],
        json!([t.dee.user.id.to_string()])
    );
    assert_eq!(created["row"]["created_by"], t.ada.user.id.to_string());
    let hidden = guests.next().await;
    assert_eq!(hidden["type"], "row_created");
    assert_eq!(hidden["row"]["values"].get(owner.to_string()), None);
    assert_eq!(hidden["row"]["created_by"], Value::Null);

    t.update(&t.dee, row.id, json!({"Owner": null, "Status": "Done"}))
        .await
        .expect("changed");
    let updated = members.next().await;
    assert_eq!(updated["type"], "row_updated");
    assert_eq!(updated["row_id"], row.id.to_string());
    assert_eq!(updated["values"][owner.to_string()], Value::Null);
    assert_eq!(
        updated["values"][t.property("Status").to_string()],
        json!(t.option_id("Status", "Done").to_string())
    );
    assert_eq!(updated.get("title"), None);
    assert_eq!(updated["last_edited_by"], t.dee.user.id.to_string());
    assert_eq!(guests.next().await["last_edited_by"], Value::Null);

    t.h.databases
        .move_row(
            &t.ada,
            row.id,
            RowMove {
                placement: Some(Placement::First),
                group: None,
            },
        )
        .await
        .expect("moved");
    let moved = members.next().await;
    assert_eq!(moved["type"], "row_moved");
    assert!(moved["position"].is_string());
    assert_eq!(moved["values"], json!({}));
    guests.next().await;

    // Renaming, trashing and restoring through the pages tell of the row too.
    t.h.pages
        .update_page(
            &t.ada,
            row.id,
            neon_app::pages::PageUpdate {
                title: Some("Fix sign-in".into()),
                ..neon_app::pages::PageUpdate::default()
            },
        )
        .await
        .expect("renamed");
    let renamed = members.next().await;
    assert_eq!(renamed["type"], "row_updated");
    assert_eq!(renamed["title"], "Fix sign-in");
    assert_eq!(
        renamed["values"][t.property("Name").to_string()],
        "Fix sign-in"
    );
    t.h.pages.trash_page(&t.ada, row.id).await.expect("trashed");
    assert_eq!(
        members.next().await,
        json!({"type": "row_trashed", "database_id": db, "row_id": row.id.to_string()})
    );
    t.h.pages
        .restore_page(&t.ada, row.id)
        .await
        .expect("restored");
    let restored = members.next().await;
    assert_eq!(restored["type"], "row_restored");
    assert_eq!(restored["row"]["title"], "Fix sign-in");
    for _ in 0..3 {
        guests.next().await;
    }

    t.add("Notes", spec(PropertyKind::Text), None).await;
    assert_eq!(
        members.next().await,
        json!({"type": "schema_changed", "database_id": db})
    );
    t.h.databases
        .duplicate_view(&t.ada, t.db(), t.board(), Some("Mine".into()))
        .await
        .expect("copied");
    let view = members.next().await;
    assert_eq!(view["type"], "view_changed");
    assert_eq!(view["deleted"], false);
}

fn count(snapshotter: &Snapshotter, operation: &str, status: &str) -> u64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            let (kind, key) = key.into_parts();
            let label = |name: &str| {
                key.labels()
                    .find(|label| label.key() == name)
                    .map(|label| label.value().to_owned())
            };
            match (kind, value) {
                (MetricKind::Counter, DebugValue::Counter(n))
                    if key.name() == "neon_databases_operations_total"
                        && label("operation").as_deref() == Some(operation)
                        && label("status").as_deref() == Some(status) =>
                {
                    Some(n)
                }
                _ => None,
            }
        })
        .sum()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn refusals_count_as_ok_and_failures_as_errors(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let t = team(pool).await;
    // Each snapshot takes the counts it reads.
    assert_eq!(count(&snapshotter, "create_database", "ok"), 1);
    assert_code(t.h.databases.database(&t.gus, t.db()).await, "not_found");
    t.h.databases.database(&t.ada, t.db()).await.expect("read");
    assert_eq!(count(&snapshotter, "get_database", "ok"), 2);

    t.h.pool.close().await;
    let failed = t.h.databases.database(&t.ada, t.db()).await;
    assert!(
        matches!(
            failed,
            Err(DatabaseError::Database(_) | DatabaseError::Page(_))
        ),
        "{failed:?}"
    );
    assert_eq!(count(&snapshotter, "get_database", "error"), 1);
}

/// Gus, a guest, edits Tasks through a share.
async fn shared_with_gus(t: &Team, access: PageAccess) {
    t.h.pages
        .share_page(&t.ada, t.db(), t.gus.user.id, access)
        .await
        .expect("shared");
}

async fn retype(
    t: &Team,
    by: &Principal,
    property: Uuid,
    kind: PropertyKind,
) -> Result<neon_domain::databases::Property, DatabaseError> {
    t.h.databases
        .update_property(
            by,
            t.db(),
            property,
            PropertyChange {
                spec: Some(spec(kind)),
                confirm: true,
                ..PropertyChange::default()
            },
        )
        .await
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn guests_do_not_change_the_type_of_a_property_that_names_people(pool: PgPool) {
    let mut t = team(pool).await;
    let owner = t.add("Owner", spec(PropertyKind::Person), None).await;
    t.row(
        &t.ada,
        json!({"Name": "Fix login", "Owner": [t.dee.user.id.to_string()]}),
    )
    .await
    .expect("a row");
    shared_with_gus(&t, PageAccess::Edit).await;
    // As text or options, the values would be the members' names.
    for kind in [
        PropertyKind::Text,
        PropertyKind::Select,
        PropertyKind::MultiSelect,
        PropertyKind::Status,
    ] {
        assert_code(retype(&t, &t.gus, owner, kind).await, "forbidden");
    }
    let seen =
        t.h.databases
            .query(&t.gus, t.db(), QueryInput::default())
            .await
            .expect("queried");
    assert_eq!(seen.rows[0].values.get(&owner.to_string()), None);
    t.reload().await;
    let kind = |t: &Team| {
        t.tasks
            .properties
            .iter()
            .find(|property| property.id == owner)
            .map(|property| property.kind())
    };
    assert_eq!(kind(&t), Some(PropertyKind::Person));
    // The guest may still rename it; a member may change its type, and its values become
    // the names of those it named.
    t.h.databases
        .update_property(
            &t.gus,
            t.db(),
            owner,
            PropertyChange {
                name: Some("Assignee".to_owned()),
                ..PropertyChange::default()
            },
        )
        .await
        .expect("renamed");
    retype(&t, &t.ada, owner, PropertyKind::Text)
        .await
        .expect("retyped");
    let read =
        t.h.databases
            .query(&t.ada, t.db(), QueryInput::default())
            .await
            .expect("queried");
    assert_eq!(
        read.rows[0].values.get(&owner.to_string()),
        Some(&json!("dee"))
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn guests_read_views_without_the_people_they_name(pool: PgPool) {
    let mut t = team(pool).await;
    t.add("Owner", spec(PropertyKind::Person), None).await;
    let dee = t.dee.user.id.to_string();
    let settings = ViewSpec {
        filter: Some(FilterSpec::Or {
            or: vec![
                condition("Owner", Operator::Contains, json!(dee)),
                condition("Owner", Operator::Contains, json!("me")),
            ],
        }),
        group: Some(GroupingSpec {
            property: "Owner".into(),
            order: vec![Some(dee.clone()), None],
            hidden: vec![Some(dee.clone())],
        }),
        ..ViewSpec::default()
    };
    let made =
        t.h.databases
            .create_view(
                &t.ada,
                t.db(),
                NewView {
                    name: "By owner".into(),
                    layout: Layout::Board,
                    settings,
                    placement: Placement::Last,
                },
            )
            .await
            .expect("a view");
    let named = |view: &neon_domain::databases::View| {
        format!("{:?}", view.settings.to_spec()).contains(&dee)
    };
    assert!(named(&made), "members read whom the view names");
    shared_with_gus(&t, PageAccess::Edit).await;
    let seen = t.h.databases.database(&t.gus, t.db()).await.expect("seen");
    let view = seen
        .views
        .iter()
        .find(|view| view.id == made.id)
        .expect("the view");
    assert!(!named(view), "{:?}", view.settings.to_spec());
    let grouping = view.settings.grouping.as_ref().expect("still grouped");
    assert_eq!(grouping.order, [None]);
    assert!(grouping.hidden.is_empty());
    assert_eq!(
        view.settings
            .filter
            .as_ref()
            .map(|filter| filter.conditions().len()),
        Some(1)
    );
    // Nor do the answers to their own changes name anyone, which keep what they did not
    // change.
    let renamed =
        t.h.databases
            .update_view(
                &t.gus,
                t.db(),
                made.id,
                ViewChange {
                    name: Some("Owners".into()),
                    ..ViewChange::default()
                },
            )
            .await
            .expect("renamed");
    assert!(!named(&renamed));
    let copy =
        t.h.databases
            .duplicate_view(&t.gus, t.db(), made.id, None)
            .await
            .expect("copied");
    assert!(!named(&copy));
    t.reload().await;
    let kept = t
        .tasks
        .views
        .iter()
        .find(|view| view.id == made.id)
        .expect("the view");
    assert!(named(kept), "the guest's rename kept the view's people");
}

/// How many bytes a row's cells take, as JSON text: how the bound on them counts.
async fn size_of(t: &Team, row: Uuid) -> usize {
    let bytes: i64 = sqlx::query_scalar(
        "SELECT octet_length(cells::text)::bigint FROM database_rows WHERE page_id = $1",
    )
    .bind(row)
    .fetch_one(&t.h.pool)
    .await
    .expect("sized");
    usize::try_from(bytes).expect("a size")
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_row_s_values_take_at_most_their_bound(pool: PgPool) {
    let mut t = team(pool).await;
    let names = ["A", "B", "C", "D", "E", "F", "G"];
    for name in names {
        t.add(name, spec(PropertyKind::Text), None).await;
    }
    let long = "x".repeat(MAX_TEXT);
    let mut six = Map::new();
    six.insert("Name".to_owned(), json!("Long"));
    for name in &names[..6] {
        six.insert((*name).to_owned(), json!(long));
    }
    let row = t
        .row(&t.ada, Value::Object(six.clone()))
        .await
        .expect("six of the longest texts fit");
    let before = size_of(&t, row.id).await;
    assert!(before <= MAX_ROW_BYTES, "{before}");
    assert_code(
        t.update(&t.ada, row.id, json!({"G": long})).await,
        "limit_reached",
    );
    assert_eq!(size_of(&t, row.id).await, before, "nothing of it is kept");
    let mut seven = six;
    seven.insert("G".to_owned(), json!(long));
    assert_code(t.row(&t.ada, Value::Object(seven)).await, "limit_reached");
    assert_eq!(t.titles().await, ["Long"]);

    // Changing a type that would make a row larger than the bound is refused too: a
    // select's option, kept by id, becomes its long name as text.
    let kind = t
        .add(
            "Kind",
            spec(PropertyKind::Select),
            Some(vec![option(&"k".repeat(MAX_NAME))]),
        )
        .await;
    t.update(&t.ada, row.id, json!({"Kind": "k".repeat(MAX_NAME)}))
        .await
        .expect("chosen");
    // Filled to a little under the bound: the seventh cell costs its key, its quotes and
    // separators, 44 bytes, besides its text.
    let room = MAX_ROW_BYTES - size_of(&t, row.id).await;
    t.update(&t.ada, row.id, json!({"G": "x".repeat(room - 44 - 32)}))
        .await
        .expect("filled");
    let filled = size_of(&t, row.id).await;
    assert!(
        (MAX_ROW_BYTES - 64..=MAX_ROW_BYTES).contains(&filled),
        "{filled}"
    );
    assert_code(
        retype(&t, &t.ada, kind, PropertyKind::Text).await,
        "limit_reached",
    );
    assert_eq!(size_of(&t, row.id).await, filled);
    t.reload().await;
    let still = t
        .tasks
        .properties
        .iter()
        .find(|property| property.id == kind)
        .map(|property| property.kind());
    assert_eq!(still, Some(PropertyKind::Select));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn queries_read_whole_rows_only_for_the_part_they_answer_with(pool: PgPool) {
    let mut t = team(pool).await;
    let points = t.add("Points", spec(PropertyKind::Number), None).await;
    let notes = t.add("Notes", spec(PropertyKind::Text), None).await;
    for (title, n) in [("A", 3), ("B", 1), ("C", 2)] {
        t.row(
            &t.ada,
            json!({"Name": title, "Points": n, "Notes": format!("About {title}")}),
        )
        .await
        .expect("a row");
    }
    // Listed for a filter, sorts and grouping, rows hold only the cells they read.
    let keys = [points.to_string()];
    let listed = neon_store::databases::rows(&t.h.pool, t.db(), None, t.ada.user.id, Some(&keys))
        .await
        .expect("listed");
    assert_eq!(listed.len(), 3);
    for row in &listed {
        let cells: Vec<&String> = row.cells.as_object().expect("cells").keys().collect();
        assert_eq!(cells, [&keys[0]]);
    }
    // The part a query answers with has every value of its rows.
    let answer =
        t.h.databases
            .query(
                &t.ada,
                t.db(),
                QueryInput {
                    sorts: Some(vec![SortSpec {
                        property: "Points".into(),
                        direction: Direction::Ascending,
                    }]),
                    limit: Some(2),
                    ..QueryInput::default()
                },
            )
            .await
            .expect("queried");
    let seen: Vec<(&str, Option<&Value>, Option<&Value>)> = answer
        .rows
        .iter()
        .map(|row| {
            (
                row.title.as_str(),
                row.values.get(&points.to_string()),
                row.values.get(&notes.to_string()),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("B", Some(&json!(1.0)), Some(&json!("About B"))),
            ("C", Some(&json!(2.0)), Some(&json!("About C"))),
        ]
    );
    assert_eq!((answer.total, answer.next.as_deref()), (3, Some("2")));
}

/// Waits until a statement of this test's database whose text holds each of `parts` waits
/// for a lock another transaction holds, or `done` says what it waited on is over.
async fn until_waiting(pool: &PgPool, parts: &[&str], done: impl Fn() -> bool) {
    let patterns: Vec<String> = parts.iter().map(|part| format!("%{part}%")).collect();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event_type = 'Lock'
                   AND query LIKE ALL($1)",
            )
            .bind(&patterns)
            .fetch_one(pool)
            .await
            .expect("asked");
            if waiting > 0 || done() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the statement came to wait");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn values_written_as_an_option_is_deleted_lose_it_with_the_rest(pool: PgPool) {
    let mut t = team(pool).await;
    let tags = t
        .add(
            "Tags",
            spec(PropertyKind::MultiSelect),
            Some(vec![option("A"), option("X")]),
        )
        .await;
    let (a, x) = (t.option_id("Tags", "A"), t.option_id("Tags", "X"));
    let row = t.made("Racy").await;

    // Another transaction holds the row's page, so the write waits once it has checked the
    // values against the schema, as it records who changed the row.
    let mut blocker = t.h.pool.begin().await.expect("begun");
    sqlx::query("SELECT 1 FROM pages WHERE id = $1 FOR UPDATE")
        .bind(row.id)
        .execute(&mut *blocker)
        .await
        .expect("locked");
    let write = tokio::spawn({
        let (databases, ada) = (t.h.databases.clone(), t.ada.clone());
        let tags = values(json!({"Tags": [a.to_string(), x.to_string()]}));
        async move { databases.update_row(&ada, row.id, tags).await }
    });
    until_waiting(&t.h.pool, &["UPDATE pages"], || false).await;

    // X is deleted meanwhile: the deletion waits for the write, or, were it not held up,
    // ends before it.
    let delete = tokio::spawn({
        let (databases, ada, db) = (t.h.databases.clone(), t.ada.clone(), t.db());
        let options = vec![ChoiceSpec {
            id: Some(a),
            ..option("A")
        }];
        async move {
            databases
                .update_property(
                    &ada,
                    db,
                    tags,
                    PropertyChange {
                        options: Some(options),
                        ..PropertyChange::default()
                    },
                )
                .await
        }
    });
    until_waiting(&t.h.pool, &["FROM databases", "FOR UPDATE"], || {
        delete.is_finished()
    })
    .await;
    blocker.rollback().await.expect("released");
    write.await.expect("joined").expect("written");
    delete.await.expect("joined").expect("X deleted");

    let stored: Value = sqlx::query_scalar("SELECT cells FROM database_rows WHERE page_id = $1")
        .bind(row.id)
        .fetch_one(&t.h.pool)
        .await
        .expect("read");
    assert_eq!(stored[tags.to_string()], json!([a.to_string()]));
    let read = t.h.databases.row(&t.ada, row.id).await.expect("read");
    assert_eq!(
        read.values.get(&tags.to_string()),
        Some(&json!([a.to_string()]))
    );
}
