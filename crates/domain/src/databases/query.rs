//! Querying a database: the rows that pass a filter, in the order of the sorts, in groups
//! when the view groups them.
//!
//! Grouped, a row is in the group of its value: an option, or, for a person property, each
//! person it names, so a row naming two people is in both their groups. The groups come
//! in the view's order: those it placed by hand first, then the rest in their natural
//! order, the group of rows without a value first, then a select's or status's options in
//! their order, or the people by name. Every option has its group, with rows or not; a
//! person has one while a row names them. A hidden group's rows are left out, and the
//! group still counts them.

use std::collections::HashMap;

use uuid::Uuid;

use super::Schema;
use super::filter::{Context, Filter};
use super::property::PropertyKind;
use super::sort::{Sort, compare};
use super::value::{Row, Value};
use super::view::{GroupKey, Grouping};

/// What to query.
#[derive(Debug, Clone, Copy, Default)]
pub struct Query<'a> {
    pub filter: Option<&'a Filter>,
    pub sorts: &'a [Sort],
    pub grouping: Option<&'a Grouping>,
}

/// A group of a grouped query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub key: GroupKey,
    /// How many rows that passed the filter are in it.
    pub count: usize,
    pub hidden: bool,
}

/// A row in a query's answer: its index among the rows queried, and its group, when the
/// query groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub row: usize,
    pub group: Option<GroupKey>,
}

/// A query's answer: its entries in order, and its groups when it groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub entries: Vec<Entry>,
    pub groups: Option<Vec<Group>>,
}

/// Runs `query` over `rows`. `names` are the members' display names, by which people sort.
pub fn run(
    schema: &Schema,
    query: Query<'_>,
    rows: &[Row],
    context: &Context,
    names: &HashMap<Uuid, String>,
) -> Outcome {
    let mut passed: Vec<usize> = (0..rows.len())
        .filter(|&i| {
            query
                .filter
                .is_none_or(|filter| filter.matches(schema, &rows[i], context))
        })
        .collect();
    passed.sort_by(|&a, &b| compare(&rows[a], &rows[b], query.sorts, schema, names));

    let grouping = query
        .grouping
        .and_then(|grouping| Some((grouping, schema.get(grouping.property)?)))
        .filter(|(_, property)| property.kind().groups());
    let Some((grouping, property)) = grouping else {
        return Outcome {
            entries: passed
                .into_iter()
                .map(|row| Entry { row, group: None })
                .collect(),
            groups: None,
        };
    };

    let keys_of = |row: &Row| -> Vec<GroupKey> {
        match row.value(property) {
            Some(Value::Choice(id)) => vec![Some(id)],
            Some(Value::People(ids)) => ids.into_iter().map(Some).collect(),
            _ => vec![None],
        }
    };
    let mut natural: Vec<GroupKey> = vec![None];
    if property.kind() == PropertyKind::Person {
        let mut people: Vec<Uuid> = Vec::new();
        for &i in &passed {
            for key in keys_of(&rows[i]).into_iter().flatten() {
                if !people.contains(&key) {
                    people.push(key);
                }
            }
        }
        people.sort_by_key(|id| {
            (
                names
                    .get(id)
                    .map(|name| name.to_lowercase())
                    .unwrap_or_default(),
                *id,
            )
        });
        natural.extend(people.into_iter().map(Some));
    } else {
        natural.extend(property.ty.options().iter().map(|option| Some(option.id)));
    }
    let mut order: Vec<GroupKey> = grouping
        .order
        .iter()
        .filter(|key| natural.contains(key))
        .copied()
        .collect();
    for key in natural {
        if !order.contains(&key) {
            order.push(key);
        }
    }

    let mut members: HashMap<GroupKey, Vec<usize>> = HashMap::new();
    for &i in &passed {
        for key in keys_of(&rows[i]) {
            members.entry(key).or_default().push(i);
        }
    }
    let mut groups = Vec::with_capacity(order.len());
    let mut entries = Vec::new();
    for key in order {
        let rows_in = members.remove(&key).unwrap_or_default();
        let hidden = grouping.hidden.contains(&key);
        groups.push(Group {
            key,
            count: rows_in.len(),
            hidden,
        });
        if !hidden {
            entries.extend(rows_in.into_iter().map(|row| Entry {
                row,
                group: Some(key),
            }));
        }
    }
    Outcome {
        entries,
        groups: Some(groups),
    }
}

#[cfg(test)]
mod tests {
    use time::{Date, OffsetDateTime, UtcOffset};

    use super::*;
    use crate::databases::filter::{Condition, Operand, Operator};
    use crate::databases::property::{Property, PropertyType};
    use crate::databases::schema::default_properties;
    use crate::databases::sort::Direction;

    const OWNER: u128 = 10;
    const POINTS: u128 = 11;
    const NOT_STARTED: u128 = 3;
    const IN_PROGRESS: u128 = 4;
    const DONE: u128 = 5;
    const ADA: u128 = 50;
    const BEA: u128 = 51;

    fn counter() -> impl FnMut() -> Uuid {
        let mut next = 0u128;
        move || {
            next += 1;
            Uuid::from_u128(next)
        }
    }

    fn schema() -> Schema {
        let mut properties = default_properties(&mut counter());
        properties.push(Property {
            id: Uuid::from_u128(OWNER),
            name: "Owner".into(),
            ty: PropertyType::Person,
        });
        properties.push(Property {
            id: Uuid::from_u128(POINTS),
            name: "Points".into(),
            ty: PropertyType::of(PropertyKind::Number),
        });
        Schema { properties }
    }

    fn row(n: u128, status: Option<u128>, owners: &[u128], points: f64) -> Row {
        let mut cells = HashMap::new();
        if let Some(status) = status {
            cells.insert(Uuid::from_u128(2), Value::Choice(Uuid::from_u128(status)));
        }
        if !owners.is_empty() {
            cells.insert(
                Uuid::from_u128(OWNER),
                Value::People(owners.iter().map(|id| Uuid::from_u128(*id)).collect()),
            );
        }
        cells.insert(Uuid::from_u128(POINTS), Value::Number(points));
        Row {
            id: Uuid::from_u128(100 + n),
            position: format!("a{n}"),
            number: n as i64,
            title: format!("Task {n}"),
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: None,
            last_edited_at: OffsetDateTime::UNIX_EPOCH,
            last_edited_by: None,
            cells,
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            row(1, Some(DONE), &[ADA], 3.0),
            row(2, Some(NOT_STARTED), &[BEA, ADA], 5.0),
            row(3, None, &[], 1.0),
            row(4, Some(DONE), &[BEA], 8.0),
            row(5, Some(IN_PROGRESS), &[], 2.0),
        ]
    }

    fn context() -> Context {
        Context {
            me: Uuid::from_u128(ADA),
            offset: UtcOffset::UTC,
            today: Date::from_calendar_date(2026, time::Month::November, 22).expect("a day"),
        }
    }

    fn names() -> HashMap<Uuid, String> {
        HashMap::from([
            (Uuid::from_u128(ADA), "Ada".to_owned()),
            (Uuid::from_u128(BEA), "Bea".to_owned()),
        ])
    }

    /// The numbers of the rows of each entry, with their group as the last digits of its id.
    fn listed(outcome: &Outcome, rows: &[Row]) -> Vec<(i64, Option<Option<u128>>)> {
        outcome
            .entries
            .iter()
            .map(|entry| {
                (
                    rows[entry.row].number,
                    entry.group.map(|key| key.map(|id| id.as_u128())),
                )
            })
            .collect()
    }

    #[test]
    fn without_a_filter_sort_or_grouping_every_row_comes_in_manual_order() {
        let rows = rows();
        let outcome = run(&schema(), Query::default(), &rows, &context(), &names());
        assert_eq!(
            listed(&outcome, &rows),
            [(1, None), (2, None), (3, None), (4, None), (5, None)]
        );
        assert_eq!(outcome.groups, None);
    }

    #[test]
    fn filters_and_sorts_apply_before_grouping() {
        let rows = rows();
        let filter = Filter::Condition(Condition {
            property: Uuid::from_u128(POINTS),
            operator: Operator::GreaterThan,
            operand: Operand::Number(1.5),
        });
        let sorts = [Sort {
            property: Uuid::from_u128(POINTS),
            direction: Direction::Descending,
        }];
        let outcome = run(
            &schema(),
            Query {
                filter: Some(&filter),
                sorts: &sorts,
                grouping: None,
            },
            &rows,
            &context(),
            &names(),
        );
        assert_eq!(
            listed(&outcome, &rows),
            [(4, None), (2, None), (1, None), (5, None)]
        );
    }

    #[test]
    fn a_board_by_status_has_a_column_per_option_and_one_without() {
        let rows = rows();
        let grouping = Grouping {
            property: Uuid::from_u128(2),
            order: vec![Some(Uuid::from_u128(DONE))],
            hidden: vec![Some(Uuid::from_u128(IN_PROGRESS))],
        };
        let outcome = run(
            &schema(),
            Query {
                filter: None,
                sorts: &[],
                grouping: Some(&grouping),
            },
            &rows,
            &context(),
            &names(),
        );
        let groups: Vec<(Option<u128>, usize, bool)> = outcome
            .groups
            .as_ref()
            .expect("grouped")
            .iter()
            .map(|g| (g.key.map(|id| id.as_u128()), g.count, g.hidden))
            .collect();
        assert_eq!(
            groups,
            [
                (Some(DONE), 2, false),
                (None, 1, false),
                (Some(NOT_STARTED), 1, false),
                (Some(IN_PROGRESS), 1, true)
            ]
        );
        assert_eq!(
            listed(&outcome, &rows),
            [
                (1, Some(Some(DONE))),
                (4, Some(Some(DONE))),
                (3, Some(None)),
                (2, Some(Some(NOT_STARTED)))
            ]
        );
    }

    #[test]
    fn a_row_naming_two_people_is_in_both_their_groups() {
        let rows = rows();
        let grouping = Grouping {
            property: Uuid::from_u128(OWNER),
            order: vec![Some(Uuid::from_u128(BEA)), Some(Uuid::from_u128(999))],
            hidden: vec![],
        };
        let outcome = run(
            &schema(),
            Query {
                filter: None,
                sorts: &[],
                grouping: Some(&grouping),
            },
            &rows,
            &context(),
            &names(),
        );
        let groups: Vec<(Option<u128>, usize)> = outcome
            .groups
            .as_ref()
            .expect("grouped")
            .iter()
            .map(|g| (g.key.map(|id| id.as_u128()), g.count))
            .collect();
        // Bea placed first; a person no row names has no group.
        assert_eq!(groups, [(Some(BEA), 2), (None, 2), (Some(ADA), 2)]);
        assert_eq!(
            listed(&outcome, &rows),
            [
                (2, Some(Some(BEA))),
                (4, Some(Some(BEA))),
                (3, Some(None)),
                (5, Some(None)),
                (1, Some(Some(ADA))),
                (2, Some(Some(ADA)))
            ]
        );
    }

    #[test]
    fn grouping_by_a_property_that_is_gone_or_does_not_group_lists_rows_plainly() {
        let rows = rows();
        for property in [999, POINTS] {
            let grouping = Grouping {
                property: Uuid::from_u128(property),
                order: vec![],
                hidden: vec![],
            };
            let outcome = run(
                &schema(),
                Query {
                    filter: None,
                    sorts: &[],
                    grouping: Some(&grouping),
                },
                &rows,
                &context(),
                &names(),
            );
            assert_eq!(outcome.groups, None);
            assert_eq!(outcome.entries.len(), 5);
        }
    }
}
