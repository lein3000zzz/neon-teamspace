//! A row's values: what each property's type takes, as the API sends it and the database
//! keeps it, how it reads as text, and the computed values a row gives.

use std::collections::HashMap;

use serde_json::{Value as Json, json};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime, UtcOffset};
use uuid::Uuid;

use super::property::{NumberFormat, PropertyKind, PropertyType};
use super::{MAX_PEOPLE, MAX_TEXT, MAX_URL};
use crate::pages::check_title;

/// One property's value on a row. An empty value is no value: none is kept.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A title, text or URL.
    Text(String),
    Number(f64),
    /// A select's or a status's option.
    Choice(Uuid),
    /// A multi-select's options, in the order given, at least one.
    Choices(Vec<Uuid>),
    Date(DateValue),
    /// Members, at least one; for the computed created and edited by, the one person.
    People(Vec<Uuid>),
    /// Checked: unchecked is no value.
    Checked,
    /// A computed time: when the row was made or last changed.
    Time(OffsetDateTime),
    /// The row's number in its database, for a unique id.
    Serial(i64),
}

/// A day, or a moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePoint {
    Day(Date),
    Moment(OffsetDateTime),
}

impl DatePoint {
    /// The day it falls on at `offset`.
    pub fn day(self, offset: UtcOffset) -> Date {
        match self {
            Self::Day(day) => day,
            Self::Moment(at) => at.to_offset(offset).date(),
        }
    }

    /// The moment it starts at, in UTC: a day starts at its midnight, UTC.
    pub fn instant(self) -> OffsetDateTime {
        match self {
            Self::Day(day) => day.midnight().assume_utc(),
            Self::Moment(at) => at,
        }
    }

    /// A day, `2026-11-22`, or an RFC 3339 moment, which is kept in UTC.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Ok(day) = Date::parse(text, DAY) {
            return Some(Self::Day(day));
        }
        OffsetDateTime::parse(text, &Rfc3339)
            .ok()
            .map(|at| Self::Moment(at.to_offset(UtcOffset::UTC)))
    }

    fn text(self) -> String {
        match self {
            Self::Day(day) => day.format(DAY).unwrap_or_default(),
            Self::Moment(at) => at.format(&Rfc3339).unwrap_or_default(),
        }
    }

    /// For people: a day as `2026-11-22`, a moment as `2026-11-22 12:00 UTC`.
    fn display(self) -> String {
        match self {
            Self::Day(day) => day.format(DAY).unwrap_or_default(),
            Self::Moment(at) => at
                .to_offset(UtcOffset::UTC)
                .format(MOMENT)
                .map(|text| format!("{text} UTC"))
                .unwrap_or_default(),
        }
    }
}

const DAY: &[time::format_description::BorrowedFormatItem<'static>] =
    format_description!("[year]-[month]-[day]");
const MOMENT: &[time::format_description::BorrowedFormatItem<'static>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]");

/// A date: a day or a moment, and an optional end of the same sort, not before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateValue {
    pub start: DatePoint,
    pub end: Option<DatePoint>,
}

/// A value a property does not take.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidValue(pub String);

fn invalid(message: impl Into<String>) -> InvalidValue {
    InvalidValue(message.into())
}

/// Whether text has control characters other than line breaks and tabs.
fn has_controls(text: &str) -> bool {
    text.chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t' && c != '\r')
}

/// The schemes a URL value may have: none that runs code where it is opened.
const URL_SCHEMES: &[&str] = &["http", "https", "mailto", "tel"];

/// A URL as kept: trimmed, at most [`MAX_URL`] bytes, no white space or control
/// characters, with one of [`URL_SCHEMES`].
pub fn check_url(raw: &str) -> Result<String, InvalidValue> {
    let url = raw.trim();
    let scheme = url
        .split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    let allowed = scheme.is_some_and(|scheme| URL_SCHEMES.contains(&scheme.as_str()));
    if url.len() > MAX_URL || url.chars().any(|c| c.is_whitespace() || c.is_control()) || !allowed {
        return Err(invalid(format!(
            "a URL is at most {MAX_URL} bytes, without spaces, and starts with http:, https:, \
             mailto: or tel:"
        )));
    }
    Ok(url.to_owned())
}

impl Value {
    /// The value a property of type `ty` takes from `json`, as the API sends it; `None` for
    /// `null` or an empty value, which clears the property. Options are named by id or
    /// name; people by id, whose membership the caller checks. Computed properties take no
    /// value.
    pub fn parse(ty: &PropertyType, json: &Json) -> Result<Option<Self>, InvalidValue> {
        if json.is_null() {
            return Ok(None);
        }
        let kind = ty.kind();
        let text = || {
            json.as_str()
                .ok_or_else(|| invalid(format!("a {kind} property takes a string")))
        };
        match kind {
            PropertyKind::Title => {
                let title = check_title(text()?).map_err(|error| invalid(error.to_string()))?;
                Ok((!title.is_empty()).then_some(Self::Text(title)))
            }
            PropertyKind::Text => {
                let text = text()?;
                if text.chars().count() > MAX_TEXT || has_controls(text) {
                    return Err(invalid(format!(
                        "a text is at most {MAX_TEXT} characters, without control characters \
                         but line breaks and tabs"
                    )));
                }
                Ok((!text.trim().is_empty()).then(|| Self::Text(text.to_owned())))
            }
            PropertyKind::Url => {
                let text = text()?;
                if text.trim().is_empty() {
                    return Ok(None);
                }
                check_url(text).map(|url| Some(Self::Text(url)))
            }
            PropertyKind::Number => match json.as_f64() {
                Some(number) if number.is_finite() => Ok(Some(Self::Number(number))),
                _ => Err(invalid("a number property takes a number")),
            },
            PropertyKind::Select | PropertyKind::Status => {
                let key = text()?;
                if key.trim().is_empty() {
                    return Ok(None);
                }
                option_of(ty, key).map(|id| Some(Self::Choice(id)))
            }
            PropertyKind::MultiSelect => {
                let keys = strings(json, kind)?;
                let mut ids: Vec<Uuid> = Vec::with_capacity(keys.len());
                for key in keys {
                    let id = option_of(ty, key)?;
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                Ok((!ids.is_empty()).then_some(Self::Choices(ids)))
            }
            PropertyKind::Date => parse_date(json).map(|date| Some(Self::Date(date))),
            PropertyKind::Person => {
                let keys = strings(json, kind)?;
                if keys.len() > MAX_PEOPLE {
                    return Err(invalid(format!(
                        "a person property names at most {MAX_PEOPLE} people"
                    )));
                }
                let mut ids: Vec<Uuid> = Vec::with_capacity(keys.len());
                for key in keys {
                    let id = Uuid::try_parse(key.trim())
                        .map_err(|_| invalid("a person property names people by their ids"))?;
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                Ok((!ids.is_empty()).then_some(Self::People(ids)))
            }
            PropertyKind::Checkbox => match json.as_bool() {
                Some(true) => Ok(Some(Self::Checked)),
                Some(false) => Ok(None),
                None => Err(invalid("a checkbox property takes true or false")),
            },
            PropertyKind::UniqueId
            | PropertyKind::CreatedTime
            | PropertyKind::CreatedBy
            | PropertyKind::LastEditedTime
            | PropertyKind::LastEditedBy => Err(invalid(format!(
                "a {kind} property is worked out from the row: it takes no value"
            ))),
        }
    }

    /// A stored value read back for a property of type `ty`: `None` when it no longer
    /// fits the type, as a value written before a change that missed it would not. Stored
    /// options are ids of options that exist, a name that happens to match counting for
    /// nothing; a multi-select keeps those of its options that still exist.
    pub fn stored(ty: &PropertyType, json: &Json) -> Option<Self> {
        let option = |json: &Json| {
            let id = Uuid::try_parse(json.as_str()?).ok()?;
            ty.options()
                .iter()
                .any(|option| option.id == id)
                .then_some(id)
        };
        match ty.kind() {
            PropertyKind::Select | PropertyKind::Status => option(json).map(Self::Choice),
            PropertyKind::MultiSelect => {
                let mut ids: Vec<Uuid> = Vec::new();
                for id in json.as_array()?.iter().filter_map(option) {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                (!ids.is_empty()).then_some(Self::Choices(ids))
            }
            _ => Self::parse(ty, json).ok().flatten(),
        }
    }

    /// The value as the API sends it and the database keeps it.
    pub fn to_json(&self) -> Json {
        match self {
            Self::Text(text) => json!(text),
            Self::Number(number) => json!(number),
            Self::Choice(id) => json!(id.to_string()),
            Self::Choices(ids) | Self::People(ids) => {
                Json::Array(ids.iter().map(|id| json!(id.to_string())).collect())
            }
            Self::Date(date) => {
                let mut object = serde_json::Map::new();
                object.insert("start".to_owned(), json!(date.start.text()));
                if let Some(end) = date.end {
                    object.insert("end".to_owned(), json!(end.text()));
                }
                Json::Object(object)
            }
            Self::Checked => json!(true),
            Self::Time(at) => json!(at.format(&Rfc3339).unwrap_or_default()),
            Self::Serial(number) => json!(number),
        }
    }

    /// The value as people read it, in an export or as text: options by name, people by
    /// `names`, numbers in the property's format.
    pub fn display(&self, ty: &PropertyType, names: &HashMap<Uuid, String>) -> String {
        let option = |id: &Uuid| {
            ty.options()
                .iter()
                .find(|option| option.id == *id)
                .map(|option| option.name.clone())
                .unwrap_or_default()
        };
        match self {
            Self::Text(text) => text.clone(),
            Self::Number(number) => format_number(*number, ty),
            Self::Choice(id) => option(id),
            Self::Choices(ids) => ids.iter().map(option).collect::<Vec<_>>().join(", "),
            Self::Date(date) => match date.end {
                Some(end) => format!("{} → {}", date.start.display(), end.display()),
                None => date.start.display(),
            },
            Self::People(ids) => ids
                .iter()
                .map(|id| names.get(id).cloned().unwrap_or_default())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
            Self::Checked => "Yes".to_owned(),
            Self::Time(at) => DatePoint::Moment(*at).display(),
            Self::Serial(number) => match ty.prefix() {
                Some(prefix) => format!("{prefix}-{number}"),
                None => number.to_string(),
            },
        }
    }
}

/// A number in a property's format: see [`NumberFormat`].
fn format_number(number: f64, ty: &PropertyType) -> String {
    let PropertyType::Number { format, currency } = ty else {
        return plain(number);
    };
    match format {
        NumberFormat::Plain => plain(number),
        NumberFormat::Integer => plain(number.round()),
        NumberFormat::Percent => format!("{}%", plain(number * 100.0)),
        NumberFormat::Currency => {
            format!("{number:.2} {}", currency.as_deref().unwrap_or_default())
                .trim_end()
                .to_owned()
        }
    }
}

/// A number as written in JSON, without a needless `.0`.
fn plain(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

/// The option of `ty` that `key` names, by id or name.
fn option_of(ty: &PropertyType, key: &str) -> Result<Uuid, InvalidValue> {
    ty.option(key).map(|option| option.id).ok_or_else(|| {
        invalid(format!(
            "{key:?} is not an option of the property: name one by its id or name"
        ))
    })
}

/// The strings of a list, or a single string as a list of one.
fn strings(json: &Json, kind: PropertyKind) -> Result<Vec<&str>, InvalidValue> {
    let wrong = || invalid(format!("a {kind} property takes a list of strings"));
    match json {
        Json::String(text) if text.trim().is_empty() => Ok(Vec::new()),
        Json::String(text) => Ok(vec![text.as_str()]),
        Json::Array(items) => items
            .iter()
            .map(|item| item.as_str().ok_or_else(wrong))
            .collect(),
        _ => Err(wrong()),
    }
}

/// A date: `{"start": ..., "end": ...}` or the start alone, each a day `2026-11-22` or an
/// RFC 3339 moment. The end, if any, is of the same sort as the start, and not before it.
fn parse_date(json: &Json) -> Result<DateValue, InvalidValue> {
    let wrong = || {
        invalid(
            "a date is {\"start\": ..., \"end\": ...} or its start alone: a day such as \
             2026-11-22, or a moment such as 2026-11-22T12:00:00Z",
        )
    };
    let point = |json: &Json| json.as_str().and_then(DatePoint::parse).ok_or_else(wrong);
    let (start, end) = match json {
        Json::String(_) => (point(json)?, None),
        Json::Object(fields) => {
            if fields.keys().any(|key| key != "start" && key != "end") {
                return Err(wrong());
            }
            let start = point(fields.get("start").ok_or_else(wrong)?)?;
            let end = match fields.get("end") {
                None | Some(Json::Null) => None,
                Some(end) => Some(point(end)?),
            };
            (start, end)
        }
        _ => return Err(wrong()),
    };
    if let Some(end) = end {
        let same_sort = matches!(
            (start, end),
            (DatePoint::Day(_), DatePoint::Day(_)) | (DatePoint::Moment(_), DatePoint::Moment(_))
        );
        if !same_sort {
            return Err(invalid(
                "a date's start and end are both days or both moments",
            ));
        }
        if end.instant() < start.instant() {
            return Err(invalid("a date ends no earlier than it starts"));
        }
    }
    Ok(DateValue { start, end })
}

/// A row as the rules read it: its page's title and times, its number, and its cells.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: Uuid,
    /// Its key among the database's rows: the manual order.
    pub position: String,
    /// Its number in the database, for its unique id.
    pub number: i64,
    pub title: String,
    pub created_at: OffsetDateTime,
    pub created_by: Option<Uuid>,
    pub last_edited_at: OffsetDateTime,
    pub last_edited_by: Option<Uuid>,
    /// The stored values, by property id.
    pub cells: HashMap<Uuid, Value>,
}

impl Row {
    /// The row's value of a property: its cell, its title, or what it computes.
    pub fn value(&self, property: &super::Property) -> Option<Value> {
        match property.kind() {
            PropertyKind::Title => {
                (!self.title.is_empty()).then(|| Value::Text(self.title.clone()))
            }
            PropertyKind::UniqueId => Some(Value::Serial(self.number)),
            PropertyKind::CreatedTime => Some(Value::Time(self.created_at)),
            PropertyKind::LastEditedTime => Some(Value::Time(self.last_edited_at)),
            PropertyKind::CreatedBy => self.created_by.map(|id| Value::People(vec![id])),
            PropertyKind::LastEditedBy => self.last_edited_by.map(|id| Value::People(vec![id])),
            _ => self.cells.get(&property.id).cloned(),
        }
    }

    /// The row as a guest sees it, who knows no other member: without the people its
    /// properties name, or who made and changed it.
    pub fn without_people(mut self, schema: &super::Schema) -> Self {
        self.created_by = None;
        self.last_edited_by = None;
        for property in &schema.properties {
            if property.kind().names_people() {
                self.cells.remove(&property.id);
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databases::Property;
    use crate::databases::property::{Choice, StatusGroup};

    fn select() -> PropertyType {
        PropertyType::Select {
            options: vec![
                Choice {
                    id: Uuid::from_u128(1),
                    name: "Bug".into(),
                    hue: None,
                    group: None,
                },
                Choice {
                    id: Uuid::from_u128(2),
                    name: "Feature".into(),
                    hue: Some(250),
                    group: None,
                },
            ],
        }
    }

    fn parse(ty: &PropertyType, json: Json) -> Result<Option<Value>, InvalidValue> {
        Value::parse(ty, &json)
    }

    fn day(text: &str) -> Date {
        Date::parse(text, DAY).expect("a day")
    }

    #[test]
    fn null_and_empty_clear_every_stored_kind() {
        for kind in PropertyKind::ALL.iter().filter(|kind| kind.is_stored()) {
            let ty = PropertyType::of(*kind);
            assert_eq!(parse(&ty, Json::Null), Ok(None), "{kind}");
        }
        assert_eq!(parse(&PropertyType::Text, json!("  ")), Ok(None));
        assert_eq!(parse(&PropertyType::Url, json!("")), Ok(None));
        assert_eq!(parse(&select(), json!("")), Ok(None));
        assert_eq!(
            parse(&select(), json!([])),
            Err(invalid("a select property takes a string"))
        );
        assert_eq!(
            parse(&PropertyType::of(PropertyKind::MultiSelect), json!([])),
            Ok(None)
        );
        assert_eq!(parse(&PropertyType::Person, json!([])), Ok(None));
        assert_eq!(parse(&PropertyType::Checkbox, json!(false)), Ok(None));
        assert_eq!(parse(&PropertyType::Title, json!(" ")), Ok(None));
    }

    #[test]
    fn computed_kinds_take_no_value() {
        for kind in PropertyKind::ALL.iter().filter(|kind| kind.is_computed()) {
            assert!(
                parse(&PropertyType::of(*kind), json!("x")).is_err(),
                "{kind}"
            );
        }
    }

    #[test]
    fn texts_titles_and_urls_are_checked() {
        assert_eq!(
            parse(&PropertyType::Text, json!("two\nlines")),
            Ok(Some(Value::Text("two\nlines".into())))
        );
        assert!(parse(&PropertyType::Text, json!("bell\u{7}")).is_err());
        assert!(parse(&PropertyType::Text, json!("x".repeat(MAX_TEXT + 1))).is_err());
        assert!(parse(&PropertyType::Text, json!(3)).is_err());
        assert_eq!(
            parse(&PropertyType::Title, json!("  Fix login ")),
            Ok(Some(Value::Text("Fix login".into())))
        );
        assert!(parse(&PropertyType::Title, json!("a\nb")).is_err());
        assert_eq!(
            parse(&PropertyType::Url, json!(" https://example.com/a?b=c ")),
            Ok(Some(Value::Text("https://example.com/a?b=c".into())))
        );
        for good in [
            "mailto:ada@example.com",
            "tel:+15550100",
            "HTTP://example.com",
        ] {
            assert!(parse(&PropertyType::Url, json!(good)).is_ok(), "{good}");
        }
        for bad in [
            "javascript:alert(1)",
            "example.com",
            "https://exa mple.com",
            "data:text/html,x",
        ] {
            assert!(parse(&PropertyType::Url, json!(bad)).is_err(), "{bad}");
        }
        assert!(
            parse(
                &PropertyType::Url,
                json!(format!("https://e.com/{}", "a".repeat(MAX_URL)))
            )
            .is_err()
        );
    }

    #[test]
    fn numbers_are_finite() {
        let number = PropertyType::of(PropertyKind::Number);
        assert_eq!(parse(&number, json!(12.5)), Ok(Some(Value::Number(12.5))));
        assert_eq!(parse(&number, json!(-3)), Ok(Some(Value::Number(-3.0))));
        assert!(parse(&number, json!("12")).is_err());
        assert!(parse(&number, json!(true)).is_err());
    }

    #[test]
    fn options_are_named_by_id_or_name() {
        let ty = select();
        assert_eq!(
            parse(&ty, json!("feature")),
            Ok(Some(Value::Choice(Uuid::from_u128(2))))
        );
        assert_eq!(
            parse(&ty, json!(Uuid::from_u128(1).to_string())),
            Ok(Some(Value::Choice(Uuid::from_u128(1))))
        );
        assert!(parse(&ty, json!("Chore")).is_err());
        let multi = PropertyType::MultiSelect {
            options: ty.options().to_vec(),
        };
        assert_eq!(
            parse(&multi, json!(["Feature", "bug", "feature"])),
            Ok(Some(Value::Choices(vec![
                Uuid::from_u128(2),
                Uuid::from_u128(1)
            ])))
        );
        assert_eq!(
            parse(&multi, json!("Bug")),
            Ok(Some(Value::Choices(vec![Uuid::from_u128(1)])))
        );
        assert!(parse(&multi, json!([1])).is_err());
        assert!(parse(&multi, json!(["Chore"])).is_err());
        let status = PropertyType::Status {
            options: vec![Choice {
                id: Uuid::from_u128(5),
                name: "Done".into(),
                hue: None,
                group: Some(StatusGroup::Done),
            }],
        };
        assert_eq!(
            parse(&status, json!("done")),
            Ok(Some(Value::Choice(Uuid::from_u128(5))))
        );
        // Stored values are ids; a name that matches an option does not count.
        assert_eq!(Value::stored(&ty, &json!("Bug")), None);
        assert_eq!(
            Value::stored(&ty, &json!(Uuid::from_u128(1).to_string())),
            Some(Value::Choice(Uuid::from_u128(1)))
        );
        assert_eq!(
            Value::stored(&ty, &json!(Uuid::from_u128(9).to_string())),
            None
        );
    }

    #[test]
    fn a_stored_multi_select_keeps_the_options_that_still_exist() {
        let multi = PropertyType::MultiSelect {
            options: select().options().to_vec(),
        };
        let (bug, feature, gone) = (
            Uuid::from_u128(1).to_string(),
            Uuid::from_u128(2).to_string(),
            Uuid::from_u128(9).to_string(),
        );
        assert_eq!(
            Value::stored(&multi, &json!([feature, gone, bug, feature])),
            Some(Value::Choices(vec![Uuid::from_u128(2), Uuid::from_u128(1)]))
        );
        assert_eq!(Value::stored(&multi, &json!([gone])), None);
        // Names are not stored values, nor is anything but a list.
        assert_eq!(Value::stored(&multi, &json!(["Bug"])), None);
        assert_eq!(Value::stored(&multi, &json!(bug)), None);
    }

    #[test]
    fn dates_are_days_or_moments_with_an_optional_end() {
        let date = |json| parse(&PropertyType::Date, json);
        assert_eq!(
            date(json!("2026-11-22")),
            Ok(Some(Value::Date(DateValue {
                start: DatePoint::Day(day("2026-11-22")),
                end: None
            })))
        );
        let moment =
            date(json!({"start": "2026-11-22T14:00:00+02:00", "end": "2026-11-22T13:00:00Z"}))
                .expect("valid")
                .expect("a value");
        assert_eq!(
            moment.to_json(),
            json!({"start": "2026-11-22T12:00:00Z", "end": "2026-11-22T13:00:00Z"})
        );
        assert_eq!(
            date(json!({"start": "2026-11-22", "end": "2026-11-24"}))
                .expect("valid")
                .expect("a value")
                .to_json(),
            json!({"start": "2026-11-22", "end": "2026-11-24"})
        );
        assert_eq!(
            date(json!({"start": "2026-11-22", "end": null}))
                .expect("valid")
                .expect("a value")
                .to_json(),
            json!({"start": "2026-11-22"})
        );
        for bad in [
            json!("22/11/2026"),
            json!({"start": "2026-11-22", "end": "2026-11-21"}),
            json!({"start": "2026-11-22", "end": "2026-11-23T00:00:00Z"}),
            json!({"end": "2026-11-22"}),
            json!({"start": "2026-11-22", "until": "2026-11-23"}),
            json!(20261122),
            json!("2026-02-30"),
        ] {
            assert!(date(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn people_are_ids() {
        let ada = Uuid::from_u128(7);
        assert_eq!(
            parse(
                &PropertyType::Person,
                json!([ada.to_string(), ada.to_string()])
            ),
            Ok(Some(Value::People(vec![ada])))
        );
        assert_eq!(
            parse(&PropertyType::Person, json!(ada.to_string())),
            Ok(Some(Value::People(vec![ada])))
        );
        assert!(parse(&PropertyType::Person, json!(["Ada"])).is_err());
        let many: Vec<String> = (0..=MAX_PEOPLE as u128)
            .map(|i| Uuid::from_u128(i).to_string())
            .collect();
        assert!(parse(&PropertyType::Person, json!(many)).is_err());
    }

    #[test]
    fn checkboxes_are_checked_or_not() {
        assert_eq!(
            parse(&PropertyType::Checkbox, json!(true)),
            Ok(Some(Value::Checked))
        );
        assert!(parse(&PropertyType::Checkbox, json!("yes")).is_err());
        assert_eq!(Value::Checked.to_json(), json!(true));
    }

    #[test]
    fn values_read_as_people_say_them() {
        let names = HashMap::from([(Uuid::from_u128(7), "Ada".to_owned())]);
        let number = |format, currency: Option<&str>| PropertyType::Number {
            format,
            currency: currency.map(str::to_owned),
        };
        let show = |value: Value, ty: &PropertyType| value.display(ty, &names);
        assert_eq!(
            show(Value::Number(1234.5), &number(NumberFormat::Plain, None)),
            "1234.5"
        );
        assert_eq!(
            show(Value::Number(3.0), &number(NumberFormat::Plain, None)),
            "3"
        );
        assert_eq!(
            show(Value::Number(1234.5), &number(NumberFormat::Integer, None)),
            "1235"
        );
        assert_eq!(
            show(Value::Number(0.25), &number(NumberFormat::Percent, None)),
            "25%"
        );
        assert_eq!(
            show(
                Value::Number(12.5),
                &number(NumberFormat::Currency, Some("EUR"))
            ),
            "12.50 EUR"
        );
        assert_eq!(
            show(Value::Choice(Uuid::from_u128(2)), &select()),
            "Feature"
        );
        assert_eq!(
            show(
                Value::Choices(vec![Uuid::from_u128(2), Uuid::from_u128(1)]),
                &select()
            ),
            "Feature, Bug"
        );
        assert_eq!(
            show(
                Value::People(vec![Uuid::from_u128(7), Uuid::from_u128(8)]),
                &PropertyType::Person
            ),
            "Ada"
        );
        assert_eq!(show(Value::Checked, &PropertyType::Checkbox), "Yes");
        assert_eq!(
            show(
                Value::Serial(42),
                &PropertyType::UniqueId {
                    prefix: Some("NW".into())
                }
            ),
            "NW-42"
        );
        assert_eq!(
            show(Value::Serial(42), &PropertyType::UniqueId { prefix: None }),
            "42"
        );
        let at = OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a time");
        assert_eq!(
            show(Value::Time(at), &PropertyType::CreatedTime),
            "2026-11-22 12:00 UTC"
        );
        assert_eq!(Value::Time(at).to_json(), json!("2026-11-22T12:00:00Z"));
        assert_eq!(
            show(
                Value::Date(DateValue {
                    start: DatePoint::Day(day("2026-11-22")),
                    end: Some(DatePoint::Day(day("2026-11-24")))
                }),
                &PropertyType::Date
            ),
            "2026-11-22 → 2026-11-24"
        );
    }

    #[test]
    fn rows_give_their_title_cells_and_computed_values() {
        let at = OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a time");
        let ada = Uuid::from_u128(7);
        let text = Property {
            id: Uuid::from_u128(20),
            name: "Notes".into(),
            ty: PropertyType::Text,
        };
        let person = Property {
            id: Uuid::from_u128(21),
            name: "Owner".into(),
            ty: PropertyType::Person,
        };
        let row = Row {
            id: Uuid::from_u128(30),
            position: "a0".into(),
            number: 4,
            title: "Fix login".into(),
            created_at: at,
            created_by: Some(ada),
            last_edited_at: at,
            last_edited_by: None,
            cells: HashMap::from([
                (text.id, Value::Text("x".into())),
                (person.id, Value::People(vec![ada])),
            ]),
        };
        let of = |kind| Property {
            id: Uuid::from_u128(99),
            name: "p".into(),
            ty: PropertyType::of(kind),
        };
        assert_eq!(
            row.value(&of(PropertyKind::Title)),
            Some(Value::Text("Fix login".into()))
        );
        assert_eq!(
            row.value(&of(PropertyKind::UniqueId)),
            Some(Value::Serial(4))
        );
        assert_eq!(
            row.value(&of(PropertyKind::CreatedTime)),
            Some(Value::Time(at))
        );
        assert_eq!(
            row.value(&of(PropertyKind::CreatedBy)),
            Some(Value::People(vec![ada]))
        );
        assert_eq!(row.value(&of(PropertyKind::LastEditedBy)), None);
        assert_eq!(row.value(&text), Some(Value::Text("x".into())));
        // A cell of a property that is not there is not its value.
        assert_eq!(row.value(&of(PropertyKind::Text)), None);

        let schema = crate::databases::Schema {
            properties: vec![text.clone(), person.clone()],
        };
        let hidden = row.clone().without_people(&schema);
        assert_eq!(hidden.value(&person), None);
        assert_eq!(hidden.value(&of(PropertyKind::CreatedBy)), None);
        assert_eq!(hidden.value(&text), Some(Value::Text("x".into())));
        let mut untitled = row;
        untitled.title = String::new();
        assert_eq!(untitled.value(&of(PropertyKind::Title)), None);
    }
}
