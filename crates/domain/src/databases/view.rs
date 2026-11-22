//! Views: a database's rows laid out as a table, a board or a list, each with its filter,
//! sorts, grouping and the properties it shows.
//!
//! A view keeps its settings as a [`ViewSpec`]. Read back, they are resolved against the
//! schema as it is then, leniently: what names a property that is gone, or no longer fits
//! its type, is left out, and the view goes on with the rest.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::filter::{Filter, FilterSpec, Fit};
use super::property::{PropertyKind, check_name};
use super::sort::{Sort, SortSpec, resolve_sorts};
use super::{InvalidView, MAX_GROUP_KEYS, MAX_WIDTH, MIN_WIDTH, Schema};

text_enum! {
    /// How a view lays its rows out.
    Layout {
        Table => "table",
        /// Cards in columns, one per group: a board always groups.
        Board => "board",
        List => "list",
    }
}

/// A group of rows: an option's id, a person's id, or `None` for the rows without a value.
pub type GroupKey = Option<Uuid>;

/// How a view groups its rows: by a property, with the groups in an order of its own and
/// some hidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grouping {
    pub property: Uuid,
    /// The groups placed by hand, first; the others follow in their natural order.
    pub order: Vec<GroupKey>,
    /// Groups whose rows the view leaves out.
    pub hidden: Vec<GroupKey>,
}

/// A grouping as it comes and as a view keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupingSpec {
    pub property: String,
    #[serde(default)]
    pub order: Vec<Option<String>>,
    #[serde(default)]
    pub hidden: Vec<Option<String>>,
}

/// A property as a view shows it: whether, and how wide, in the view's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewProperty {
    pub property: Uuid,
    pub visible: bool,
    /// In pixels, once someone resized its column.
    pub width: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewPropertySpec {
    pub property: String,
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u16>,
}

/// A view's settings as it comes and as it is kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<FilterSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sorts: Vec<SortSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupingSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<ViewPropertySpec>,
}

/// A view's settings resolved against a schema.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewSettings {
    pub filter: Option<Filter>,
    pub sorts: Vec<Sort>,
    pub grouping: Option<Grouping>,
    /// Every property of the schema, once, in the view's order.
    pub properties: Vec<ViewProperty>,
}

/// A view of a database.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub id: Uuid,
    pub database_id: Uuid,
    /// Its key among the database's views.
    pub position: String,
    pub name: String,
    pub layout: Layout,
    pub settings: ViewSettings,
}

impl ViewSpec {
    /// The settings resolved against `schema` for a view laid out as `layout`. Strictly,
    /// nothing may name what the schema lacks; leniently, what does not fit is left out.
    /// That a board groups is [`ViewSettings::check_layout`]'s to say.
    pub fn resolve(
        &self,
        schema: &Schema,
        layout: Layout,
        fit: Fit,
    ) -> Result<ViewSettings, InvalidView> {
        let filter = match &self.filter {
            Some(spec) => spec
                .resolve(schema, fit)
                .map_err(|error| InvalidView(error.0))?,
            None => None,
        };
        let sorts = resolve_sorts(&self.sorts, schema, fit)?;
        let grouping = match &self.group {
            Some(spec) => match resolve_grouping(spec, schema, fit) {
                Ok(grouping) => Some(grouping),
                Err(_) if fit == Fit::Lenient => None,
                Err(error) => return Err(error),
            },
            None => None,
        };
        let properties = resolve_properties(&self.properties, schema, layout, fit)?;
        Ok(ViewSettings {
            filter,
            sorts,
            grouping,
            properties,
        })
    }
}

impl ViewSpec {
    /// The settings as a view kept them, fitted to `schema` as it is now: what no longer
    /// fits is left out, a filter past its bounds altogether.
    pub fn fit(&self, schema: &Schema, layout: Layout) -> ViewSettings {
        ViewSettings {
            filter: self
                .filter
                .as_ref()
                .and_then(|spec| spec.resolve(schema, Fit::Lenient).ok().flatten()),
            sorts: resolve_sorts(&self.sorts, schema, Fit::Lenient).unwrap_or_default(),
            grouping: self
                .group
                .as_ref()
                .and_then(|spec| resolve_grouping(spec, schema, Fit::Lenient).ok()),
            properties: resolve_properties(&self.properties, schema, layout, Fit::Lenient)
                .unwrap_or_default(),
        }
    }
}

impl ViewSettings {
    /// Refuses settings that do not suit `layout`: a board groups its cards, in columns.
    /// Ask it when a view is made a board, or its grouping changes; a board whose grouping
    /// property was deleted or changed type shows its cards ungrouped until it is given
    /// another.
    pub fn check_layout(&self, layout: Layout) -> Result<(), InvalidView> {
        if layout == Layout::Board && self.grouping.is_none() {
            return Err(InvalidView(
                "a board groups its cards by a select, status or person property".to_owned(),
            ));
        }
        Ok(())
    }

    /// The settings as a guest reads them, who knows no other member: without the people
    /// the filter names, nor those of a person grouping's groups placed or hidden by hand.
    /// What the view shows a guest is the same, as their rows name nobody.
    pub fn without_people(mut self, schema: &Schema) -> Self {
        self.filter = self.filter.and_then(Filter::without_people);
        if let Some(grouping) = &mut self.grouping
            && schema
                .get(grouping.property)
                .is_some_and(|property| property.kind().names_people())
        {
            grouping.order.retain(Option::is_none);
            grouping.hidden.retain(Option::is_none);
        }
        self
    }

    /// The settings as kept.
    pub fn to_spec(&self) -> ViewSpec {
        let key = |key: &GroupKey| key.map(|id| id.to_string());
        ViewSpec {
            filter: self.filter.as_ref().map(Filter::to_spec),
            sorts: self.sorts.iter().map(|sort| sort.to_spec()).collect(),
            group: self.grouping.as_ref().map(|grouping| GroupingSpec {
                property: grouping.property.to_string(),
                order: grouping.order.iter().map(key).collect(),
                hidden: grouping.hidden.iter().map(key).collect(),
            }),
            properties: self
                .properties
                .iter()
                .map(|shown| ViewPropertySpec {
                    property: shown.property.to_string(),
                    visible: shown.visible,
                    width: shown.width,
                })
                .collect(),
        }
    }
}

impl View {
    /// The view as a guest reads it: see [`ViewSettings::without_people`].
    pub fn without_people(mut self, schema: &Schema) -> Self {
        self.settings = self.settings.without_people(schema);
        self
    }
}

fn resolve_grouping(
    spec: &GroupingSpec,
    schema: &Schema,
    fit: Fit,
) -> Result<Grouping, InvalidView> {
    let property = schema.find(&spec.property).ok_or_else(|| {
        InvalidView(format!(
            "the grouping names no property of the database: {:?}",
            spec.property
        ))
    })?;
    if !property.kind().groups() {
        return Err(InvalidView(format!(
            "a view groups by a select, status or person property, and {:?} is a {}",
            property.name,
            property.kind()
        )));
    }
    let keys = |list: &[Option<String>]| -> Result<Vec<GroupKey>, InvalidView> {
        if list.len() > MAX_GROUP_KEYS && fit == Fit::Strict {
            return Err(InvalidView(format!(
                "a view orders and hides at most {MAX_GROUP_KEYS} groups"
            )));
        }
        let mut seen = HashSet::new();
        let mut keys = Vec::with_capacity(list.len());
        for key in list.iter().take(MAX_GROUP_KEYS) {
            let resolved = match key {
                None => Some(None),
                Some(key) if property.kind() == PropertyKind::Person => {
                    Uuid::try_parse(key).ok().map(Some)
                }
                Some(key) => property.ty.option(key).map(|option| Some(option.id)),
            };
            match resolved {
                Some(key) => {
                    if seen.insert(key) {
                        keys.push(key);
                    }
                }
                None if fit == Fit::Lenient => {}
                None => {
                    return Err(InvalidView(format!(
                        "{key:?} is no group of {:?}: name an option, a person, or null for \
                         the rows without a value",
                        property.name
                    )));
                }
            }
        }
        Ok(keys)
    };
    Ok(Grouping {
        property: property.id,
        order: keys(&spec.order)?,
        hidden: keys(&spec.hidden)?,
    })
}

/// Every property of the schema once: those `specs` lists in its order, then the others in
/// the schema's, shown in a table and, elsewhere, but for the title, hidden.
fn resolve_properties(
    specs: &[ViewPropertySpec],
    schema: &Schema,
    layout: Layout,
    fit: Fit,
) -> Result<Vec<ViewProperty>, InvalidView> {
    let mut shown: Vec<ViewProperty> = Vec::with_capacity(schema.properties.len());
    for spec in specs {
        let Some(property) = schema.find(&spec.property) else {
            if fit == Fit::Lenient {
                continue;
            }
            return Err(InvalidView(format!(
                "the view's properties name no property of the database: {:?}",
                spec.property
            )));
        };
        if shown.iter().any(|entry| entry.property == property.id) {
            if fit == Fit::Lenient {
                continue;
            }
            return Err(InvalidView(format!(
                "the view's properties list {:?} twice",
                property.name
            )));
        }
        let width = match spec.width {
            Some(width) if !(MIN_WIDTH..=MAX_WIDTH).contains(&width) => {
                if fit == Fit::Strict {
                    return Err(InvalidView(format!(
                        "a column is {MIN_WIDTH} to {MAX_WIDTH} pixels wide"
                    )));
                }
                Some(width.clamp(MIN_WIDTH, MAX_WIDTH))
            }
            width => width,
        };
        shown.push(ViewProperty {
            property: property.id,
            visible: spec.visible,
            width,
        });
    }
    for property in &schema.properties {
        if shown.iter().all(|entry| entry.property != property.id) {
            shown.push(ViewProperty {
                property: property.id,
                visible: layout == Layout::Table || property.kind() == PropertyKind::Title,
                width: None,
            });
        }
    }
    Ok(shown)
}

/// A view's name, checked: as a property's.
pub fn check_view_name(raw: &str) -> Result<String, InvalidView> {
    check_name(raw).map_err(|error| InvalidView(error.to_string()))
}

/// The views a database starts with: a table of every property, and a board grouped by
/// its status, when it has one.
pub fn default_views(schema: &Schema) -> Vec<(String, Layout, ViewSpec)> {
    let mut views = vec![("Table".to_owned(), Layout::Table, ViewSpec::default())];
    if let Some(status) = schema
        .properties
        .iter()
        .find(|property| property.kind() == PropertyKind::Status)
    {
        views.push((
            "Board".to_owned(),
            Layout::Board,
            ViewSpec {
                group: Some(GroupingSpec {
                    property: status.id.to_string(),
                    order: Vec::new(),
                    hidden: Vec::new(),
                }),
                ..ViewSpec::default()
            },
        ));
    }
    views
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::databases::filter::Operator;
    use crate::databases::property::{Property, PropertyType};
    use crate::databases::schema::default_properties;
    use crate::databases::sort::Direction;

    fn counter() -> impl FnMut() -> Uuid {
        let mut next = 0u128;
        move || {
            next += 1;
            Uuid::from_u128(next)
        }
    }

    /// Name (1), Status (2) with options 3, 4, 5, then Notes (10) and Owner (11).
    fn schema() -> Schema {
        let mut properties = default_properties(&mut counter());
        properties.push(Property {
            id: Uuid::from_u128(10),
            name: "Notes".into(),
            ty: PropertyType::Text,
        });
        properties.push(Property {
            id: Uuid::from_u128(11),
            name: "Owner".into(),
            ty: PropertyType::Person,
        });
        Schema { properties }
    }

    fn id(n: u128) -> String {
        Uuid::from_u128(n).to_string()
    }

    #[test]
    fn a_database_starts_with_a_table_and_a_board_by_status() {
        let schema = schema();
        let views = default_views(&schema);
        let names: Vec<(&str, Layout)> = views
            .iter()
            .map(|(name, layout, _)| (name.as_str(), *layout))
            .collect();
        assert_eq!(names, [("Table", Layout::Table), ("Board", Layout::Board)]);
        let table = views[0]
            .2
            .resolve(&schema, Layout::Table, Fit::Strict)
            .expect("valid");
        assert!(table.properties.iter().all(|shown| shown.visible));
        assert_eq!(table.properties.len(), 4);
        let board = views[1]
            .2
            .resolve(&schema, Layout::Board, Fit::Strict)
            .expect("valid");
        assert_eq!(
            board.grouping.as_ref().map(|g| g.property),
            Some(Uuid::from_u128(2))
        );
        let visible: Vec<u128> = board
            .properties
            .iter()
            .filter(|shown| shown.visible)
            .map(|shown| shown.property.as_u128())
            .collect();
        assert_eq!(visible, [1]);
        let without_status = Schema {
            properties: schema.properties[..1].to_vec(),
        };
        assert_eq!(default_views(&without_status).len(), 1);
    }

    #[test]
    fn a_board_groups_by_a_select_status_or_person() {
        let schema = schema();
        let grouped = |property: &str| ViewSpec {
            group: Some(GroupingSpec {
                property: property.into(),
                order: Vec::new(),
                hidden: Vec::new(),
            }),
            ..ViewSpec::default()
        };
        let suits = |spec: ViewSpec, layout| {
            spec.resolve(&schema, layout, Fit::Strict)
                .and_then(|settings| settings.check_layout(layout))
                .is_ok()
        };
        assert!(!suits(ViewSpec::default(), Layout::Board));
        assert!(suits(ViewSpec::default(), Layout::Table));
        assert!(suits(grouped("Status"), Layout::Board));
        assert!(
            grouped("Notes")
                .resolve(&schema, Layout::Board, Fit::Strict)
                .is_err()
        );
        assert!(
            grouped("Name")
                .resolve(&schema, Layout::Table, Fit::Strict)
                .is_err()
        );
        assert!(
            grouped("Owner")
                .resolve(&schema, Layout::Board, Fit::Strict)
                .is_ok()
        );
        assert!(
            grouped("status")
                .resolve(&schema, Layout::List, Fit::Strict)
                .is_ok()
        );
        assert!(
            ViewSpec::default()
                .resolve(&schema, Layout::List, Fit::Strict)
                .is_ok()
        );
        // Kept from before its property changed, a board goes on without grouping.
        let lenient = grouped("Gone")
            .resolve(&schema, Layout::Board, Fit::Lenient)
            .expect("lenient");
        assert_eq!(lenient.grouping, None);
    }

    #[test]
    fn group_keys_are_options_people_or_null() {
        let schema = schema();
        let spec = |order: Vec<Option<String>>, hidden: Vec<Option<String>>| ViewSpec {
            group: Some(GroupingSpec {
                property: "Status".into(),
                order,
                hidden,
            }),
            ..ViewSpec::default()
        };
        let settings = spec(
            vec![Some("Done".into()), None, Some(id(5)), Some(id(3))],
            vec![Some(id(4))],
        )
        .resolve(&schema, Layout::Board, Fit::Strict)
        .expect("valid");
        let grouping = settings.grouping.expect("grouped");
        assert_eq!(
            grouping.order,
            [Some(Uuid::from_u128(5)), None, Some(Uuid::from_u128(3))]
        );
        assert_eq!(grouping.hidden, [Some(Uuid::from_u128(4))]);
        assert!(
            spec(vec![Some("Blocked".into())], vec![])
                .resolve(&schema, Layout::Board, Fit::Strict)
                .is_err()
        );
        let lenient = spec(vec![Some("Blocked".into()), None], vec![])
            .resolve(&schema, Layout::Board, Fit::Lenient)
            .expect("lenient");
        assert_eq!(lenient.grouping.map(|g| g.order), Some(vec![None]));
        let too_many = spec(vec![None; MAX_GROUP_KEYS + 1], vec![]);
        assert!(
            too_many
                .resolve(&schema, Layout::Board, Fit::Strict)
                .is_err()
        );
        let people = ViewSpec {
            group: Some(GroupingSpec {
                property: "Owner".into(),
                order: vec![Some(id(77)), Some("not an id".into())],
                hidden: vec![],
            }),
            ..ViewSpec::default()
        };
        assert!(people.resolve(&schema, Layout::Board, Fit::Strict).is_err());
        assert_eq!(
            people
                .resolve(&schema, Layout::Board, Fit::Lenient)
                .expect("lenient")
                .grouping
                .map(|g| g.order),
            Some(vec![Some(Uuid::from_u128(77))])
        );
    }

    #[test]
    fn guests_read_settings_without_the_people_they_name() {
        let schema = schema();
        let (ada, bea) = (id(77), id(78));
        let spec: ViewSpec = serde_json::from_value(json!({
            "filter": {"and": [
                {"property": "Owner", "operator": "contains", "value": ada},
                {"property": "Owner", "operator": "contains", "value": "me"}
            ]},
            "group": {"property": "Owner", "order": [bea, null], "hidden": [ada]},
        }))
        .expect("a spec");
        let settings = spec
            .resolve(&schema, Layout::Board, Fit::Strict)
            .expect("valid");
        let seen = settings.clone().without_people(&schema);
        let text = format!("{:?}", seen.to_spec());
        assert!(!text.contains(&ada) && !text.contains(&bea), "{text}");
        let grouping = seen.grouping.expect("still grouped");
        assert_eq!((grouping.order, grouping.hidden), (vec![None], vec![]));
        assert_eq!(seen.filter.map(|filter| filter.conditions().len()), Some(1));
        // A grouping by options names nobody, and stays as it is.
        let by_status: ViewSpec = serde_json::from_value(json!({
            "group": {"property": "Status", "order": [id(4)], "hidden": [id(5)]},
        }))
        .expect("a spec");
        let status = by_status
            .resolve(&schema, Layout::Board, Fit::Strict)
            .expect("valid");
        assert_eq!(status.clone().without_people(&schema), status);
    }

    #[test]
    fn properties_are_listed_once_each_in_the_view_s_order() {
        let schema = schema();
        let shown = |property: &str, visible, width| ViewPropertySpec {
            property: property.into(),
            visible,
            width,
        };
        let settings = ViewSpec {
            properties: vec![shown("Owner", true, Some(240)), shown("Name", true, None)],
            ..ViewSpec::default()
        }
        .resolve(&schema, Layout::List, Fit::Strict)
        .expect("valid");
        let listed: Vec<(u128, bool, Option<u16>)> = settings
            .properties
            .iter()
            .map(|p| (p.property.as_u128(), p.visible, p.width))
            .collect();
        assert_eq!(
            listed,
            [
                (11, true, Some(240)),
                (1, true, None),
                (2, false, None),
                (10, false, None)
            ]
        );
        for bad in [
            vec![shown("Nope", true, None)],
            vec![shown("Name", true, None), shown("name", false, None)],
            vec![shown("Name", true, Some(MIN_WIDTH - 1))],
            vec![shown("Name", true, Some(MAX_WIDTH + 1))],
        ] {
            let spec = ViewSpec {
                properties: bad,
                ..ViewSpec::default()
            };
            assert!(spec.resolve(&schema, Layout::Table, Fit::Strict).is_err());
            let lenient = spec
                .resolve(&schema, Layout::Table, Fit::Lenient)
                .expect("lenient");
            assert_eq!(lenient.properties.len(), 4);
        }
    }

    #[test]
    fn settings_are_kept_as_their_spec_and_read_back_the_same() {
        let schema = schema();
        let spec: ViewSpec = serde_json::from_value(json!({
            "filter": {"and": [
                {"property": "Status", "operator": "is", "value": "Done"},
                {"or": [
                    {"property": "Owner", "operator": "contains", "value": "me"},
                    {"property": "Notes", "operator": "is_empty"}
                ]}
            ]},
            "sorts": [{"property": "Name", "direction": "descending"}],
            "group": {"property": "Status", "order": ["Done", null], "hidden": []},
            "properties": [{"property": "Notes", "visible": false, "width": 120}]
        }))
        .expect("a spec");
        let settings = spec
            .resolve(&schema, Layout::Board, Fit::Strict)
            .expect("valid");
        assert_eq!(
            settings.sorts,
            [Sort {
                property: Uuid::from_u128(1),
                direction: Direction::Descending
            }]
        );
        let kept = serde_json::to_value(settings.to_spec()).expect("serializes");
        assert_eq!(kept["filter"]["and"][0]["value"], id(5));
        assert_eq!(kept["filter"]["and"][0]["operator"], "is");
        assert_eq!(kept["filter"]["and"][1]["or"][1].get("value"), None);
        assert_eq!(kept["group"]["order"], json!([id(5), null]));
        let back: ViewSpec = serde_json::from_value(kept).expect("reads back");
        assert_eq!(
            back.resolve(&schema, Layout::Board, Fit::Strict)
                .expect("valid"),
            settings
        );
        let Some(Filter::And(items)) = &settings.filter else {
            panic!("an and group: {:?}", settings.filter);
        };
        assert!(matches!(
            &items[1],
            Filter::Or(or) if matches!(
                &or[0],
                Filter::Condition(condition) if condition.operator == Operator::Contains
            )
        ));
    }

    #[test]
    fn a_schema_change_leaves_out_what_no_longer_fits() {
        let schema = schema();
        let spec: ViewSpec = serde_json::from_value(json!({
            "filter": {"and": [
                {"property": "Owner", "operator": "contains", "value": "me"},
                {"property": "Notes", "operator": "contains", "value": "x"}
            ]},
            "sorts": [{"property": "Owner", "direction": "ascending"}],
            "group": {"property": "Owner"},
        }))
        .expect("a spec");
        let settings = spec
            .resolve(&schema, Layout::Board, Fit::Strict)
            .expect("valid");
        // Owner is deleted, and Notes becomes a number.
        let mut changed = schema.clone();
        changed.properties.retain(|p| p.id != Uuid::from_u128(11));
        changed.properties[2].ty = PropertyType::of(PropertyKind::Number);
        let refit = settings.to_spec().fit(&changed, Layout::Board);
        assert_eq!(refit.filter, None);
        assert!(refit.sorts.is_empty());
        assert_eq!(refit.grouping, None);
        assert_eq!(refit.properties.len(), 3);
    }

    #[test]
    fn names_and_layouts_are_checked() {
        assert_eq!(check_view_name(" Sprint "), Ok("Sprint".to_owned()));
        assert!(check_view_name("").is_err());
        for layout in Layout::ALL {
            assert_eq!(Layout::parse(layout.as_str()), Some(*layout));
        }
    }
}
