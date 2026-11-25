//! Sorts: rows in the order of one property's values, then the next's, and last in their
//! manual order.
//!
//! Texts compare ignoring case; options in their property's order; dates by their start,
//! a day as its midnight UTC; people by the name of the first, then the next; checkboxes
//! unchecked before checked. Rows without a value come last, whichever the direction.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::filter::Fit;
use super::property::{Property, PropertyKind};
use super::value::{Row, Value};
use super::{InvalidView, MAX_SORTS, Schema};

text_enum! {
    Direction {
        Ascending => "ascending",
        Descending => "descending",
    }
}

/// A sort as it comes and as a view keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortSpec {
    pub property: String,
    pub direction: Direction,
}

/// A sort resolved against a schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub property: Uuid,
    pub direction: Direction,
}

impl Sort {
    pub fn to_spec(self) -> SortSpec {
        SortSpec {
            property: self.property.to_string(),
            direction: self.direction,
        }
    }
}

/// Sorts resolved against `schema`: at most [`MAX_SORTS`], each on a property of it, and
/// none twice on one.
pub fn resolve_sorts(
    specs: &[SortSpec],
    schema: &Schema,
    fit: Fit,
) -> Result<Vec<Sort>, InvalidView> {
    if specs.len() > MAX_SORTS && fit == Fit::Strict {
        return Err(InvalidView(format!(
            "a view sorts by at most {MAX_SORTS} properties"
        )));
    }
    let mut sorts: Vec<Sort> = Vec::with_capacity(specs.len());
    for spec in specs.iter().take(MAX_SORTS) {
        let property = match schema.find(&spec.property) {
            Some(property) => property,
            None if fit == Fit::Lenient => continue,
            None => {
                return Err(InvalidView(format!(
                    "the sorts name no property of the database: {:?}",
                    spec.property
                )));
            }
        };
        if sorts.iter().any(|sort| sort.property == property.id) {
            if fit == Fit::Lenient {
                continue;
            }
            return Err(InvalidView(format!(
                "the sorts name {:?} twice",
                property.name
            )));
        }
        sorts.push(Sort {
            property: property.id,
            direction: spec.direction,
        });
    }
    Ok(sorts)
}

/// How two rows compare by `sorts`, then by their manual order, then by id, so the order
/// is the same every time.
pub fn compare(
    a: &Row,
    b: &Row,
    sorts: &[Sort],
    schema: &Schema,
    names: &HashMap<Uuid, String>,
) -> Ordering {
    for sort in sorts {
        let Some(property) = schema.get(sort.property) else {
            continue;
        };
        let order = compare_by(property, a, b, sort.direction, names);
        if order != Ordering::Equal {
            return order;
        }
    }
    a.position
        .as_bytes()
        .cmp(b.position.as_bytes())
        .then(a.id.cmp(&b.id))
}

fn compare_by(
    property: &Property,
    a: &Row,
    b: &Row,
    direction: Direction,
    names: &HashMap<Uuid, String>,
) -> Ordering {
    let flip = |order: Ordering| match direction {
        Direction::Ascending => order,
        Direction::Descending => order.reverse(),
    };
    if property.kind() == PropertyKind::Checkbox {
        let checked = |row: &Row| row.value(property).is_some();
        return flip(checked(a).cmp(&checked(b)));
    }
    match (row_key(property, a, names), row_key(property, b, names)) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => flip(a.partial_cmp(&b).unwrap_or(Ordering::Equal)),
    }
}

/// What a value sorts by.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
enum Key {
    Text(String, String),
    Number(Total),
    Indexes(Vec<usize>),
    Names(Vec<(String, Uuid)>),
    Time(time::OffsetDateTime, Option<time::OffsetDateTime>),
}

/// A number ordered by `total_cmp`, so every value has a place.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Total(f64);

impl PartialOrd for Total {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.0.total_cmp(&other.0))
    }
}

fn row_key(property: &Property, row: &Row, names: &HashMap<Uuid, String>) -> Option<Key> {
    let value = row.value(property)?;
    let index = |id: Uuid| property.ty.option_index(id).unwrap_or(usize::MAX);
    Some(match value {
        Value::Text(text) => Key::Text(text.to_lowercase(), text),
        Value::Number(number) => Key::Number(Total(number)),
        // Row numbers stay far below where f64 loses whole numbers.
        Value::Serial(number) => Key::Number(Total(number as f64)),
        Value::Choice(id) => Key::Indexes(vec![index(id)]),
        Value::Choices(ids) => Key::Indexes(ids.into_iter().map(index).collect()),
        Value::People(ids) => Key::Names(
            ids.into_iter()
                .map(|id| {
                    let name = names.get(&id).map(|name| name.to_lowercase());
                    (name.unwrap_or_default(), id)
                })
                .collect(),
        ),
        Value::Date(date) => Key::Time(date.start.instant(), date.end.map(|end| end.instant())),
        Value::Time(at) => Key::Time(at, None),
        Value::Checked => Key::Number(Total(1.0)),
    })
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::*;
    use crate::databases::property::{Choice, PropertyType};
    use crate::databases::value::{DatePoint, DateValue};

    const TITLE: u128 = 1;
    const NUMBER: u128 = 2;
    const SELECT: u128 = 3;
    const PEOPLE: u128 = 4;
    const CHECK: u128 = 5;
    const DATE: u128 = 6;
    const MULTI: u128 = 7;

    fn options() -> Vec<Choice> {
        ["Low", "High"]
            .iter()
            .enumerate()
            .map(|(i, name)| Choice {
                id: Uuid::from_u128(100 + i as u128),
                name: (*name).into(),
                hue: None,
                group: None,
            })
            .collect()
    }

    fn schema() -> Schema {
        let property = |id, name: &str, ty| Property {
            id: Uuid::from_u128(id),
            name: name.into(),
            ty,
        };
        Schema {
            properties: vec![
                property(TITLE, "Name", PropertyType::Title),
                property(NUMBER, "Points", PropertyType::of(PropertyKind::Number)),
                property(
                    SELECT,
                    "Priority",
                    PropertyType::Select { options: options() },
                ),
                property(PEOPLE, "Owner", PropertyType::Person),
                property(CHECK, "Done", PropertyType::Checkbox),
                property(DATE, "Due", PropertyType::Date),
                property(
                    MULTI,
                    "Tags",
                    PropertyType::MultiSelect { options: options() },
                ),
            ],
        }
    }

    fn row(id: u128, position: &str, title: &str, cells: Vec<(u128, Value)>) -> Row {
        let at = OffsetDateTime::UNIX_EPOCH;
        Row {
            id: Uuid::from_u128(id),
            position: position.into(),
            number: id as i64,
            title: title.into(),
            created_at: at,
            created_by: None,
            last_edited_at: at,
            last_edited_by: None,
            cells: cells
                .into_iter()
                .map(|(property, value)| (Uuid::from_u128(property), value))
                .collect(),
        }
    }

    fn order(rows: &[Row], sorts: &[(u128, Direction)]) -> Vec<u128> {
        let schema = schema();
        let names = HashMap::from([
            (Uuid::from_u128(50), "bea".to_owned()),
            (Uuid::from_u128(51), "Ada".to_owned()),
        ]);
        let sorts: Vec<Sort> = sorts
            .iter()
            .map(|(property, direction)| Sort {
                property: Uuid::from_u128(*property),
                direction: *direction,
            })
            .collect();
        let mut rows = rows.to_vec();
        rows.sort_by(|a, b| compare(a, b, &sorts, &schema, &names));
        rows.iter().map(|row| row.id.as_u128()).collect()
    }

    use Direction::{Ascending as Up, Descending as Down};

    #[test]
    fn without_sorts_rows_keep_their_manual_order() {
        let rows = [
            row(1, "a2", "c", vec![]),
            row(2, "a0", "b", vec![]),
            row(3, "a1", "a", vec![]),
            row(4, "a1", "a", vec![]),
        ];
        assert_eq!(order(&rows, &[]), [2, 3, 4, 1]);
    }

    #[test]
    fn texts_sort_ignoring_case_and_empty_values_go_last() {
        let rows = [
            row(1, "a0", "beta", vec![]),
            row(2, "a1", "", vec![]),
            row(3, "a2", "Alpha", vec![]),
            row(4, "a3", "alpha", vec![]),
        ];
        assert_eq!(order(&rows, &[(TITLE, Up)]), [3, 4, 1, 2]);
        assert_eq!(order(&rows, &[(TITLE, Down)]), [1, 4, 3, 2]);
    }

    #[test]
    fn numbers_options_dates_and_people_sort_by_their_meaning() {
        let day = |d: u8| {
            Value::Date(DateValue {
                start: DatePoint::Day(
                    time::Date::from_calendar_date(2026, time::Month::November, d).expect("a day"),
                ),
                end: None,
            })
        };
        let rows = [
            row(
                1,
                "a0",
                "",
                vec![
                    (NUMBER, Value::Number(10.0)),
                    (SELECT, Value::Choice(Uuid::from_u128(101))),
                    (DATE, day(3)),
                    (PEOPLE, Value::People(vec![Uuid::from_u128(50)])),
                ],
            ),
            row(
                2,
                "a1",
                "",
                vec![
                    (NUMBER, Value::Number(-2.5)),
                    (SELECT, Value::Choice(Uuid::from_u128(100))),
                    (DATE, day(1)),
                    (PEOPLE, Value::People(vec![Uuid::from_u128(51)])),
                ],
            ),
            row(3, "a2", "", vec![(NUMBER, Value::Number(3.0))]),
        ];
        assert_eq!(order(&rows, &[(NUMBER, Up)]), [2, 3, 1]);
        assert_eq!(order(&rows, &[(NUMBER, Down)]), [1, 3, 2]);
        // Options in their order: Low, then High.
        assert_eq!(order(&rows, &[(SELECT, Up)]), [2, 1, 3]);
        assert_eq!(order(&rows, &[(SELECT, Down)]), [1, 2, 3]);
        assert_eq!(order(&rows, &[(DATE, Up)]), [2, 1, 3]);
        // By name, ignoring case: Ada before bea.
        assert_eq!(order(&rows, &[(PEOPLE, Up)]), [2, 1, 3]);
        assert_eq!(order(&rows, &[(PEOPLE, Down)]), [1, 2, 3]);
    }

    #[test]
    fn checkboxes_sort_unchecked_first_and_sorts_chain() {
        let rows = [
            row(1, "a0", "b", vec![(CHECK, Value::Checked)]),
            row(2, "a1", "a", vec![]),
            row(3, "a2", "a", vec![(CHECK, Value::Checked)]),
            row(
                4,
                "a3",
                "c",
                vec![(
                    MULTI,
                    Value::Choices(vec![Uuid::from_u128(101), Uuid::from_u128(100)]),
                )],
            ),
            row(
                5,
                "a4",
                "c",
                vec![(MULTI, Value::Choices(vec![Uuid::from_u128(101)]))],
            ),
        ];
        assert_eq!(order(&rows, &[(CHECK, Up)]), [2, 4, 5, 1, 3]);
        assert_eq!(order(&rows, &[(CHECK, Down)]), [1, 3, 2, 4, 5]);
        assert_eq!(order(&rows, &[(CHECK, Down), (TITLE, Up)]), [3, 1, 2, 4, 5]);
        assert_eq!(order(&rows, &[(MULTI, Up)]), [5, 4, 1, 2, 3]);
    }

    #[test]
    fn sorts_are_resolved_against_the_schema() {
        let schema = schema();
        let spec = |property: &str| SortSpec {
            property: property.into(),
            direction: Up,
        };
        assert_eq!(
            resolve_sorts(&[spec("points"), spec("Name")], &schema, Fit::Strict),
            Ok(vec![
                Sort {
                    property: Uuid::from_u128(NUMBER),
                    direction: Up
                },
                Sort {
                    property: Uuid::from_u128(TITLE),
                    direction: Up
                }
            ])
        );
        assert!(resolve_sorts(&[spec("Nope")], &schema, Fit::Strict).is_err());
        assert!(resolve_sorts(&[spec("Name"), spec("name")], &schema, Fit::Strict).is_err());
        assert_eq!(
            resolve_sorts(
                &[spec("Nope"), spec("Name"), spec("name")],
                &schema,
                Fit::Lenient
            )
            .map(|sorts| sorts.len()),
            Ok(1)
        );
        let many: Vec<SortSpec> = (0..=MAX_SORTS).map(|_| spec("Name")).collect();
        assert!(resolve_sorts(&many, &schema, Fit::Strict).is_err());
        assert_eq!(
            Sort {
                property: Uuid::from_u128(TITLE),
                direction: Down
            }
            .to_spec(),
            SortSpec {
                property: Uuid::from_u128(TITLE).to_string(),
                direction: Down
            }
        );
    }
}
