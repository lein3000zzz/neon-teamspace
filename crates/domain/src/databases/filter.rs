//! Filters: a tree of `and` and `or` groups of conditions, each a property, an operator its
//! type takes, and a value to compare with.
//!
//! A filter comes as a [`FilterSpec`], properties named by id or name and values as JSON,
//! and is resolved against the database's schema into a [`Filter`], which is checked and
//! evaluated. A view keeps its filter as the spec of the resolved filter; read back after
//! the schema changed, conditions that no longer fit are left out.
//!
//! | Kinds | Operators | Value |
//! |---|---|---|
//! | title, text, URL | `is`, `is_not`, `contains`, `does_not_contain`, `starts_with`, `ends_with` | a string, compared ignoring case |
//! | number, unique id | `equals`, `not_equals`, `greater_than`, `greater_than_or_equal`, `less_than`, `less_than_or_equal` | a number; a unique id compares its number |
//! | select | `is`, `is_not` | an option, by id or name |
//! | status | `is`, `is_not`; `in_group` | an option; a group, `to_do`, `in_progress` or `done` |
//! | multi-select | `contains`, `does_not_contain` | an option |
//! | date, created time, last edited time | `is`, `before`, `after`, `on_or_before`, `on_or_after`; `within` | a day, `2026-11-22`, or a day relative to today; a range relative to today |
//! | person, created by, last edited by | `contains`, `does_not_contain` | a member's id, or `me` |
//! | checkbox | `is` | `true` or `false` |
//!
//! Every kind but the checkbox also takes `is_empty` and `is_not_empty`, without a value.
//! A negative operator, `is_not`, `does_not_contain` or `not_equals`, holds for a row without
//! a value; the others do not. Dates compare by day, the day a moment falls on at the
//! caller's UTC offset, and a date with an end compares by its start.

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use time::macros::format_description;
use time::{Date, Duration, UtcOffset};
use uuid::Uuid;

use super::property::{PropertyKind, StatusGroup};
use super::value::{DatePoint, Row, Value};
use super::{MAX_CONDITIONS, MAX_FILTER_DEPTH, MAX_FILTER_TEXT, Schema};

text_enum! {
    /// How a condition compares a row's value with its own.
    Operator {
        Is => "is",
        IsNot => "is_not",
        Contains => "contains",
        DoesNotContain => "does_not_contain",
        StartsWith => "starts_with",
        EndsWith => "ends_with",
        IsEmpty => "is_empty",
        IsNotEmpty => "is_not_empty",
        Equals => "equals",
        NotEquals => "not_equals",
        GreaterThan => "greater_than",
        GreaterThanOrEqual => "greater_than_or_equal",
        LessThan => "less_than",
        LessThanOrEqual => "less_than_or_equal",
        Before => "before",
        After => "after",
        OnOrBefore => "on_or_before",
        OnOrAfter => "on_or_after",
        Within => "within",
        InGroup => "in_group",
    }
}

text_enum! {
    /// A day relative to the caller's today.
    RelativeDay {
        Today => "today",
        Tomorrow => "tomorrow",
        Yesterday => "yesterday",
        OneWeekAgo => "one_week_ago",
        OneWeekFromNow => "one_week_from_now",
        OneMonthAgo => "one_month_ago",
        OneMonthFromNow => "one_month_from_now",
    }
}

text_enum! {
    /// Days relative to the caller's today, today included: the week, month or year up to
    /// it or from it.
    RelativeRange {
        PastWeek => "past_week",
        PastMonth => "past_month",
        PastYear => "past_year",
        NextWeek => "next_week",
        NextMonth => "next_month",
        NextYear => "next_year",
    }
}

impl RelativeDay {
    pub fn on(self, today: Date) -> Date {
        match self {
            Self::Today => today,
            Self::Tomorrow => today.saturating_add(Duration::days(1)),
            Self::Yesterday => today.saturating_sub(Duration::days(1)),
            Self::OneWeekAgo => today.saturating_sub(Duration::days(7)),
            Self::OneWeekFromNow => today.saturating_add(Duration::days(7)),
            Self::OneMonthAgo => add_months(today, -1),
            Self::OneMonthFromNow => add_months(today, 1),
        }
    }
}

impl RelativeRange {
    /// The first and last days of the range.
    pub fn days(self, today: Date) -> (Date, Date) {
        match self {
            Self::PastWeek => (today.saturating_sub(Duration::days(7)), today),
            Self::PastMonth => (add_months(today, -1), today),
            Self::PastYear => (add_months(today, -12), today),
            Self::NextWeek => (today, today.saturating_add(Duration::days(7))),
            Self::NextMonth => (today, add_months(today, 1)),
            Self::NextYear => (today, add_months(today, 12)),
        }
    }
}

/// The same day `months` months away, or the last day of that month when it is shorter.
fn add_months(date: Date, months: i32) -> Date {
    let index = date.year() * 12 + i32::from(u8::from(date.month())) - 1 + months;
    let (year, month) = (index.div_euclid(12), index.rem_euclid(12) + 1);
    let month = u8::try_from(month)
        .ok()
        .and_then(|month| time::Month::try_from(month).ok())
        .unwrap_or(time::Month::January);
    let day = date.day().min(time::util::days_in_month(month, year));
    Date::from_calendar_date(year, month, day).unwrap_or(date)
}

/// A filter as it comes and as a view keeps it: properties by id or name, values as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilterSpec {
    And {
        and: Vec<FilterSpec>,
    },
    Or {
        or: Vec<FilterSpec>,
    },
    Condition {
        property: String,
        operator: Operator,
        #[serde(default, skip_serializing_if = "Json::is_null")]
        value: Json,
    },
}

/// A filter resolved against a schema: every condition on a property it has, with an
/// operator and value its type takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Condition(Condition),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub property: Uuid,
    pub operator: Operator,
    pub operand: Operand,
}

/// What a condition compares with.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// Nothing: for `is_empty` and `is_not_empty`.
    None,
    /// As given; compared ignoring case.
    Text(String),
    Number(f64),
    Choice(Uuid),
    Group(StatusGroup),
    Me,
    Person(Uuid),
    Checked(bool),
    Day(Date),
    Relative(RelativeDay),
    Range(RelativeRange),
}

impl Operand {
    fn to_json(&self) -> Json {
        match self {
            Self::None => Json::Null,
            Self::Text(text) => json!(text),
            Self::Number(number) => json!(number),
            Self::Choice(id) | Self::Person(id) => json!(id.to_string()),
            Self::Group(group) => json!(group.as_str()),
            Self::Me => json!("me"),
            Self::Checked(checked) => json!(checked),
            Self::Day(day) => json!(day.format(DAY).unwrap_or_default()),
            Self::Relative(day) => json!(day.as_str()),
            Self::Range(range) => json!(range.as_str()),
        }
    }
}

const DAY: &[time::format_description::BorrowedFormatItem<'static>] =
    format_description!("[year]-[month]-[day]");

/// A filter that does not fit the schema, or its bounds.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidFilter(pub String);

/// What resolving does with a condition that does not fit: refuse the filter, for one that
/// comes in a request, or leave the condition out, for one a view kept from before the
/// schema changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    Strict,
    Lenient,
}

/// The operators a kind of property takes.
pub fn operators(kind: PropertyKind) -> &'static [Operator] {
    use Operator::*;
    match kind {
        PropertyKind::Title | PropertyKind::Text | PropertyKind::Url => &[
            Is,
            IsNot,
            Contains,
            DoesNotContain,
            StartsWith,
            EndsWith,
            IsEmpty,
            IsNotEmpty,
        ],
        PropertyKind::Number | PropertyKind::UniqueId => &[
            Equals,
            NotEquals,
            GreaterThan,
            GreaterThanOrEqual,
            LessThan,
            LessThanOrEqual,
            IsEmpty,
            IsNotEmpty,
        ],
        PropertyKind::Select => &[Is, IsNot, IsEmpty, IsNotEmpty],
        PropertyKind::Status => &[Is, IsNot, InGroup, IsEmpty, IsNotEmpty],
        PropertyKind::MultiSelect => &[Contains, DoesNotContain, IsEmpty, IsNotEmpty],
        PropertyKind::Date | PropertyKind::CreatedTime | PropertyKind::LastEditedTime => &[
            Is, Before, After, OnOrBefore, OnOrAfter, Within, IsEmpty, IsNotEmpty,
        ],
        PropertyKind::Person | PropertyKind::CreatedBy | PropertyKind::LastEditedBy => {
            &[Contains, DoesNotContain, IsEmpty, IsNotEmpty]
        }
        PropertyKind::Checkbox => &[Is],
    }
}

impl FilterSpec {
    /// The filter resolved against `schema`, or `None` when nothing is left of it, such as
    /// an empty group.
    pub fn resolve(&self, schema: &Schema, fit: Fit) -> Result<Option<Filter>, InvalidFilter> {
        let mut conditions = 0;
        self.resolve_at(schema, fit, 1, &mut conditions)
    }

    fn resolve_at(
        &self,
        schema: &Schema,
        fit: Fit,
        depth: usize,
        conditions: &mut usize,
    ) -> Result<Option<Filter>, InvalidFilter> {
        let group = |items: &[FilterSpec], conditions: &mut usize| {
            if depth > MAX_FILTER_DEPTH {
                return Err(InvalidFilter(format!(
                    "filter groups nest at most {MAX_FILTER_DEPTH} deep"
                )));
            }
            let mut resolved = Vec::with_capacity(items.len());
            for item in items {
                if let Some(filter) = item.resolve_at(schema, fit, depth + 1, conditions)? {
                    resolved.push(filter);
                }
            }
            Ok(resolved)
        };
        match self {
            Self::And { and } => {
                let items = group(and, conditions)?;
                Ok((!items.is_empty()).then_some(Filter::And(items)))
            }
            Self::Or { or } => {
                let items = group(or, conditions)?;
                Ok((!items.is_empty()).then_some(Filter::Or(items)))
            }
            Self::Condition {
                property,
                operator,
                value,
            } => {
                *conditions += 1;
                if *conditions > MAX_CONDITIONS {
                    return Err(InvalidFilter(format!(
                        "a filter has at most {MAX_CONDITIONS} conditions"
                    )));
                }
                match condition(schema, property, *operator, value) {
                    Ok(condition) => Ok(Some(Filter::Condition(condition))),
                    Err(_) if fit == Fit::Lenient => Ok(None),
                    Err(error) => Err(error),
                }
            }
        }
    }
}

/// A condition resolved against the schema.
fn condition(
    schema: &Schema,
    key: &str,
    operator: Operator,
    value: &Json,
) -> Result<Condition, InvalidFilter> {
    let property = schema.find(key).ok_or_else(|| {
        InvalidFilter(format!(
            "the filter names no property of the database: {key:?}"
        ))
    })?;
    let kind = property.kind();
    if !operators(kind).contains(&operator) {
        return Err(InvalidFilter(format!(
            "a {kind} property takes no {operator} condition: it takes {}",
            operators(kind)
                .iter()
                .map(|operator| operator.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let wrong = |what: &str| {
        InvalidFilter(format!(
            "a {operator} condition on {:?} compares with {what}",
            property.name
        ))
    };
    let operand = match (kind, operator) {
        (_, Operator::IsEmpty | Operator::IsNotEmpty) => Operand::None,
        (PropertyKind::Title | PropertyKind::Text | PropertyKind::Url, _) => match value.as_str() {
            Some(text) if !text.is_empty() && text.chars().count() <= MAX_FILTER_TEXT => {
                Operand::Text(text.to_owned())
            }
            _ => {
                return Err(wrong(&format!(
                    "a text of 1 to {MAX_FILTER_TEXT} characters"
                )));
            }
        },
        (PropertyKind::Number | PropertyKind::UniqueId, _) => match value.as_f64() {
            Some(number) if number.is_finite() => Operand::Number(number),
            _ => return Err(wrong("a number")),
        },
        (PropertyKind::Status, Operator::InGroup) => value
            .as_str()
            .and_then(StatusGroup::parse)
            .map(Operand::Group)
            .ok_or_else(|| wrong("a group: to_do, in_progress or done"))?,
        (PropertyKind::Select | PropertyKind::Status | PropertyKind::MultiSelect, _) => value
            .as_str()
            .and_then(|key| property.ty.option(key))
            .map(|option| Operand::Choice(option.id))
            .ok_or_else(|| wrong("one of the property's options, by id or name"))?,
        (
            PropertyKind::Date | PropertyKind::CreatedTime | PropertyKind::LastEditedTime,
            Operator::Within,
        ) => value
            .as_str()
            .and_then(RelativeRange::parse)
            .map(Operand::Range)
            .ok_or_else(|| {
                wrong(
                    "a range: past_week, past_month, past_year, next_week, next_month or next_year",
                )
            })?,
        (PropertyKind::Date | PropertyKind::CreatedTime | PropertyKind::LastEditedTime, _) => {
            let text = value.as_str().unwrap_or_default();
            if let Some(day) = RelativeDay::parse(text) {
                Operand::Relative(day)
            } else {
                Date::parse(text, DAY)
                    .map(Operand::Day)
                    .map_err(|_| wrong("a day such as 2026-11-22, or today, tomorrow, yesterday, one_week_ago, one_week_from_now, one_month_ago or one_month_from_now"))?
            }
        }
        (PropertyKind::Person | PropertyKind::CreatedBy | PropertyKind::LastEditedBy, _) => {
            match value.as_str() {
                Some("me") => Operand::Me,
                Some(text) => Uuid::try_parse(text)
                    .map(Operand::Person)
                    .map_err(|_| wrong("a member's id, or me"))?,
                None => return Err(wrong("a member's id, or me")),
            }
        }
        (PropertyKind::Checkbox, _) => value
            .as_bool()
            .map(Operand::Checked)
            .ok_or_else(|| wrong("true or false"))?,
    };
    Ok(Condition {
        property: property.id,
        operator,
        operand,
    })
}

/// What evaluating a filter needs of the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    /// Who `me` is.
    pub me: Uuid,
    /// Their UTC offset, which days are told at.
    pub offset: UtcOffset,
    /// Their today, at that offset.
    pub today: Date,
}

impl Filter {
    /// The filter as a view keeps it.
    pub fn to_spec(&self) -> FilterSpec {
        match self {
            Self::And(items) => FilterSpec::And {
                and: items.iter().map(Self::to_spec).collect(),
            },
            Self::Or(items) => FilterSpec::Or {
                or: items.iter().map(Self::to_spec).collect(),
            },
            Self::Condition(condition) => FilterSpec::Condition {
                property: condition.property.to_string(),
                operator: condition.operator,
                value: condition.operand.to_json(),
            },
        }
    }

    /// Whether `row` passes the filter.
    pub fn matches(&self, schema: &Schema, row: &Row, context: &Context) -> bool {
        match self {
            Self::And(items) => items.iter().all(|item| item.matches(schema, row, context)),
            Self::Or(items) => items.iter().any(|item| item.matches(schema, row, context)),
            Self::Condition(condition) => condition.matches(schema, row, context),
        }
    }

    /// The conditions, every one.
    pub fn conditions(&self) -> Vec<&Condition> {
        match self {
            Self::And(items) | Self::Or(items) => items.iter().flat_map(Self::conditions).collect(),
            Self::Condition(condition) => vec![condition],
        }
    }
}

impl Condition {
    fn matches(&self, schema: &Schema, row: &Row, context: &Context) -> bool {
        // A resolved filter names properties of the schema; one deleted meanwhile filters
        // nothing.
        let Some(property) = schema.get(self.property) else {
            return true;
        };
        let value = row.value(property);
        use Operator::*;
        match self.operator {
            IsEmpty => return value.is_none(),
            IsNotEmpty => return value.is_some(),
            _ => {}
        }
        let negative = matches!(self.operator, IsNot | DoesNotContain | NotEquals);
        let Some(value) = value else {
            // Without a value only a negative condition holds, and a checkbox that is
            // unchecked.
            return negative || self.operand == Operand::Checked(false);
        };
        match (&value, &self.operand) {
            (Value::Text(text), Operand::Text(wanted)) => {
                let (text, wanted) = (text.to_lowercase(), wanted.to_lowercase());
                match self.operator {
                    Is => text == wanted,
                    IsNot => text != wanted,
                    Contains => text.contains(wanted.as_str()),
                    DoesNotContain => !text.contains(wanted.as_str()),
                    StartsWith => text.starts_with(wanted.as_str()),
                    EndsWith => text.ends_with(wanted.as_str()),
                    _ => false,
                }
            }
            (Value::Number(_) | Value::Serial(_), Operand::Number(wanted)) => {
                let number = match value {
                    Value::Number(number) => number,
                    // Row numbers stay far below where f64 loses whole numbers.
                    Value::Serial(number) => number as f64,
                    _ => return false,
                };
                match self.operator {
                    Equals => number == *wanted,
                    NotEquals => number != *wanted,
                    GreaterThan => number > *wanted,
                    GreaterThanOrEqual => number >= *wanted,
                    LessThan => number < *wanted,
                    LessThanOrEqual => number <= *wanted,
                    _ => false,
                }
            }
            (Value::Choice(id), Operand::Choice(wanted)) => match self.operator {
                Is => id == wanted,
                IsNot => id != wanted,
                _ => false,
            },
            (Value::Choice(id), Operand::Group(group)) => property
                .ty
                .options()
                .iter()
                .find(|option| option.id == *id)
                .is_some_and(|option| option.group() == Some(*group)),
            (Value::Choices(ids), Operand::Choice(wanted)) => match self.operator {
                Contains => ids.contains(wanted),
                DoesNotContain => !ids.contains(wanted),
                _ => false,
            },
            (Value::People(ids), Operand::Me | Operand::Person(_)) => {
                let wanted = match self.operand {
                    Operand::Person(id) => id,
                    _ => context.me,
                };
                match self.operator {
                    Contains => ids.contains(&wanted),
                    DoesNotContain => !ids.contains(&wanted),
                    _ => false,
                }
            }
            (Value::Checked, Operand::Checked(wanted)) => *wanted,
            (Value::Date(_) | Value::Time(_), _) => {
                let day = match value {
                    Value::Date(date) => date.start.day(context.offset),
                    Value::Time(at) => DatePoint::Moment(at).day(context.offset),
                    _ => return false,
                };
                let wanted = match self.operand {
                    Operand::Day(day) => day,
                    Operand::Relative(relative) => relative.on(context.today),
                    Operand::Range(range) => {
                        let (from, to) = range.days(context.today);
                        return self.operator == Within && from <= day && day <= to;
                    }
                    _ => return false,
                };
                match self.operator {
                    Is => day == wanted,
                    Before => day < wanted,
                    After => day > wanted,
                    OnOrBefore => day <= wanted,
                    OnOrAfter => day >= wanted,
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
