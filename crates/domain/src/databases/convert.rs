//! Changing a property's type: what becomes of each row's value.
//!
//! | From | To | Each value becomes |
//! |---|---|---|
//! | any | the same kind | itself: only the settings change, such as a number's format |
//! | select, multi-select, status | select, multi-select, status | the same options, by the same ids; a status takes each option's group, or to do |
//! | any other | select, multi-select, status | an option named after its text, made for it; a multi-select splits the text at commas |
//! | any | text | its text, as an export shows it |
//! | any | number | its text read as a number; a checked checkbox is 1 |
//! | any | checkbox | checked for a number other than 0, or a text such as `yes`, `true`, `x` or `1`; unchecked for 0, `no`, `false` or `0` |
//! | any | URL, date | its text read as a URL, or as a day or moment |
//! | any but a person | person | nothing |
//! | any | unique id, created or edited time or by | nothing: the new property computes its values |
//! | unique id, created or edited time or by | any | nothing: they were never kept |
//!
//! A value that becomes nothing is lost, and so is all but the first of several becoming
//! one, as a multi-select's options becoming a select's. A change that loses values is
//! made only when the caller confirms it.

use std::collections::HashMap;

use uuid::Uuid;

use super::property::{Choice, Property, PropertyKind, PropertyType, StatusGroup, check_name};
use super::value::{DatePoint, DateValue, Value, check_url};
use super::{MAX_OPTIONS, MAX_TEXT};

/// What changing a property's type does to its rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    /// The property's new type: as asked, with its options worked out.
    pub ty: PropertyType,
    /// Each row whose value changes, and its new value. Rows left out keep theirs.
    pub values: Vec<(Uuid, Option<Value>)>,
    /// How many rows lose all or part of their value.
    pub lost: usize,
}

/// What becomes of the values of `property`, each row's in `values`, when it becomes of type
/// `target`. `names` are the members' display names, which people become as text; `ids`
/// makes the ids of new options.
pub fn convert(
    property: &Property,
    target: PropertyType,
    values: &[(Uuid, Value)],
    names: &HashMap<Uuid, String>,
    ids: &mut dyn FnMut() -> Uuid,
) -> Conversion {
    let from = property.kind();
    let to = target.kind();
    if from == to {
        // Only its settings change; options change on their own.
        let ty = match target {
            PropertyType::Select { .. }
            | PropertyType::MultiSelect { .. }
            | PropertyType::Status { .. } => property.ty.clone(),
            other => other,
        };
        return Conversion {
            ty,
            values: Vec::new(),
            lost: 0,
        };
    }
    if !from.is_stored() {
        return Conversion {
            ty: target,
            values: Vec::new(),
            lost: 0,
        };
    }
    if !to.is_stored() {
        return Conversion {
            ty: target,
            values: values.iter().map(|(row, _)| (*row, None)).collect(),
            lost: values.len(),
        };
    }
    if to.has_options() {
        return to_options(property, to, values, names, ids);
    }
    let mut converted = Vec::with_capacity(values.len());
    let mut lost = 0;
    for (row, value) in values {
        let text = value.display(&property.ty, names);
        let new = match to {
            PropertyKind::Text => {
                let text: String = text.chars().take(MAX_TEXT).collect();
                (!text.trim().is_empty()).then_some(Value::Text(text))
            }
            PropertyKind::Url => check_url(&text).ok().map(Value::Text),
            PropertyKind::Number => match value {
                Value::Checked => Some(Value::Number(1.0)),
                _ => parse_number(&text).map(Value::Number),
            },
            PropertyKind::Checkbox => {
                let checked = match value {
                    Value::Number(number) => Some(*number != 0.0),
                    _ => truth(&text),
                };
                match checked {
                    Some(true) => Some(Value::Checked),
                    Some(false) => {
                        // Unchecked is no value, and nothing is lost.
                        converted.push((*row, None));
                        continue;
                    }
                    None => None,
                }
            }
            PropertyKind::Date => {
                DatePoint::parse(&text).map(|start| Value::Date(DateValue { start, end: None }))
            }
            _ => None,
        };
        if new.is_none() {
            lost += 1;
        }
        converted.push((*row, new));
    }
    Conversion {
        ty: target,
        values: converted,
        lost,
    }
}

/// Values becoming options, of a select, multi-select or status.
fn to_options(
    property: &Property,
    to: PropertyKind,
    values: &[(Uuid, Value)],
    names: &HashMap<Uuid, String>,
    ids: &mut dyn FnMut() -> Uuid,
) -> Conversion {
    let group = |option: &Choice| match to {
        PropertyKind::Status => Some(option.group().unwrap_or(StatusGroup::ToDo)),
        _ => None,
    };
    let carried = property.kind().has_options();
    let mut options: Vec<Choice> = if carried {
        property
            .ty
            .options()
            .iter()
            .map(|option| Choice {
                group: group(option),
                ..option.clone()
            })
            .collect()
    } else {
        Vec::new()
    };
    let single = to != PropertyKind::MultiSelect;
    let mut converted = Vec::with_capacity(values.len());
    let mut lost = 0;
    for (row, value) in values {
        let (chosen, partly_lost) = if carried {
            let ids: Vec<Uuid> = match value {
                Value::Choice(id) => vec![*id],
                Value::Choices(ids) => ids.clone(),
                _ => Vec::new(),
            };
            (ids, false)
        } else {
            let pieces = pieces(value, &property.ty, names, !single);
            let mut chosen = Vec::with_capacity(pieces.len());
            let mut dropped = false;
            for piece in pieces {
                match option_named(&mut options, &piece, to, ids) {
                    Some(id) => {
                        if !chosen.contains(&id) {
                            chosen.push(id);
                        }
                    }
                    None => dropped = true,
                }
            }
            (chosen, dropped)
        };
        let new = match (single, chosen.as_slice()) {
            (_, []) => None,
            (true, [first, ..]) => Some(Value::Choice(*first)),
            (false, _) => Some(Value::Choices(chosen.clone())),
        };
        if new.is_none() || partly_lost || (single && chosen.len() > 1) {
            lost += 1;
        }
        converted.push((*row, new));
    }
    let ty = match to {
        PropertyKind::Select => PropertyType::Select { options },
        PropertyKind::MultiSelect => PropertyType::MultiSelect { options },
        _ => PropertyType::Status { options },
    };
    Conversion {
        ty,
        values: converted,
        lost,
    }
}

/// The texts a value becomes options of: its people's names, or its text, split at commas
/// for a multi-select.
fn pieces(
    value: &Value,
    ty: &PropertyType,
    names: &HashMap<Uuid, String>,
    split: bool,
) -> Vec<String> {
    let texts: Vec<String> = match value {
        Value::People(ids) => ids.iter().filter_map(|id| names.get(id).cloned()).collect(),
        other => vec![other.display(ty, names)],
    };
    texts
        .into_iter()
        .flat_map(|text| {
            if split {
                text.split(',').map(str::to_owned).collect()
            } else {
                vec![text]
            }
        })
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect()
}

/// The option named `name`, ignoring case, made when there is none and room for one.
fn option_named(
    options: &mut Vec<Choice>,
    name: &str,
    to: PropertyKind,
    ids: &mut dyn FnMut() -> Uuid,
) -> Option<Uuid> {
    let name = check_name(name).ok()?;
    if let Some(option) = options
        .iter()
        .find(|option| option.name.to_lowercase() == name.to_lowercase())
    {
        return Some(option.id);
    }
    if options.len() >= MAX_OPTIONS {
        return None;
    }
    let option = Choice {
        id: ids(),
        name,
        hue: None,
        group: (to == PropertyKind::Status).then_some(StatusGroup::ToDo),
    };
    let id = option.id;
    options.push(option);
    Some(id)
}

/// A text read as a number: digits with an optional sign and decimals, and commas or
/// spaces between thousands.
fn parse_number(text: &str) -> Option<f64> {
    let cleaned: String = text
        .trim()
        .chars()
        .filter(|c| *c != ',' && !c.is_whitespace())
        .collect();
    cleaned
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
}

/// A text read as a checkbox: checked, unchecked, or neither.
fn truth(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "yes" | "y" | "true" | "x" | "1" | "checked" | "on" | "✓" | "✔" => Some(true),
        "no" | "n" | "false" | "0" | "unchecked" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databases::property::NumberFormat;

    fn counter() -> impl FnMut() -> Uuid {
        let mut next = 500u128;
        move || {
            next += 1;
            Uuid::from_u128(next)
        }
    }

    fn property(ty: PropertyType) -> Property {
        Property {
            id: Uuid::from_u128(1),
            name: "P".into(),
            ty,
        }
    }

    fn rows(values: Vec<Value>) -> Vec<(Uuid, Value)> {
        values
            .into_iter()
            .enumerate()
            .map(|(i, value)| (Uuid::from_u128(100 + i as u128), value))
            .collect()
    }

    fn choice(id: u128, name: &str) -> Choice {
        Choice {
            id: Uuid::from_u128(id),
            name: name.into(),
            hue: Some(25),
            group: None,
        }
    }

    fn run(from: PropertyType, to: PropertyType, values: Vec<Value>) -> Conversion {
        let names = HashMap::from([
            (Uuid::from_u128(7), "Ada".to_owned()),
            (Uuid::from_u128(8), "Bea".to_owned()),
        ]);
        convert(&property(from), to, &rows(values), &names, &mut counter())
    }

    fn new_values(conversion: &Conversion) -> Vec<Option<Value>> {
        conversion
            .values
            .iter()
            .map(|(_, value)| value.clone())
            .collect()
    }

    fn text(text: &str) -> Value {
        Value::Text(text.into())
    }

    #[test]
    fn the_same_kind_changes_only_its_settings() {
        let percent = PropertyType::Number {
            format: NumberFormat::Percent,
            currency: None,
        };
        let done = run(
            PropertyType::of(PropertyKind::Number),
            percent.clone(),
            vec![Value::Number(0.5)],
        );
        assert_eq!(
            done,
            Conversion {
                ty: percent,
                values: vec![],
                lost: 0
            }
        );
        let select = PropertyType::Select {
            options: vec![choice(9, "Kept")],
        };
        let kept = run(
            select.clone(),
            PropertyType::of(PropertyKind::Select),
            vec![],
        );
        assert_eq!(kept.ty, select);
    }

    #[test]
    fn anything_becomes_text() {
        let multi = PropertyType::MultiSelect {
            options: vec![choice(9, "Bug"), choice(10, "UI")],
        };
        let done = run(
            multi,
            PropertyType::Text,
            vec![Value::Choices(vec![
                Uuid::from_u128(10),
                Uuid::from_u128(9),
            ])],
        );
        assert_eq!(new_values(&done), [Some(text("UI, Bug"))]);
        assert_eq!(done.lost, 0);
        let people = run(
            PropertyType::Person,
            PropertyType::Text,
            vec![Value::People(vec![Uuid::from_u128(7), Uuid::from_u128(8)])],
        );
        assert_eq!(new_values(&people), [Some(text("Ada, Bea"))]);
        let numbers = run(
            PropertyType::of(PropertyKind::Number),
            PropertyType::Text,
            vec![Value::Number(2.0), Value::Number(2.5)],
        );
        assert_eq!(new_values(&numbers), [Some(text("2")), Some(text("2.5"))]);
        let checks = run(
            PropertyType::Checkbox,
            PropertyType::Text,
            vec![Value::Checked],
        );
        assert_eq!(new_values(&checks), [Some(text("Yes"))]);
    }

    #[test]
    fn texts_become_numbers_checkboxes_dates_and_urls_when_they_read_as_them() {
        let number = run(
            PropertyType::Text,
            PropertyType::of(PropertyKind::Number),
            vec![text(" 1,234.5 "), text("-3"), text("twelve")],
        );
        assert_eq!(
            new_values(&number),
            [Some(Value::Number(1234.5)), Some(Value::Number(-3.0)), None]
        );
        assert_eq!(number.lost, 1);
        let check = run(
            PropertyType::Text,
            PropertyType::Checkbox,
            vec![text("Yes"), text("no"), text("maybe"), text("X")],
        );
        assert_eq!(
            new_values(&check),
            [Some(Value::Checked), None, None, Some(Value::Checked)]
        );
        assert_eq!(check.lost, 1);
        let from_number = run(
            PropertyType::of(PropertyKind::Number),
            PropertyType::Checkbox,
            vec![Value::Number(0.0), Value::Number(2.0)],
        );
        assert_eq!(new_values(&from_number), [None, Some(Value::Checked)]);
        assert_eq!(from_number.lost, 0);
        let to_number = run(
            PropertyType::Checkbox,
            PropertyType::of(PropertyKind::Number),
            vec![Value::Checked],
        );
        assert_eq!(new_values(&to_number), [Some(Value::Number(1.0))]);
        let date = run(
            PropertyType::Text,
            PropertyType::Date,
            vec![text("2026-11-22"), text("next tuesday")],
        );
        assert_eq!(date.lost, 1);
        assert_eq!(
            new_values(&date)[0].as_ref().map(Value::to_json),
            Some(serde_json::json!({"start": "2026-11-22"}))
        );
        let url = run(
            PropertyType::Text,
            PropertyType::Url,
            vec![text("https://example.com"), text("not a url")],
        );
        assert_eq!(new_values(&url), [Some(text("https://example.com")), None]);
        assert_eq!(url.lost, 1);
    }

    #[test]
    fn options_carry_over_between_option_kinds() {
        let select = PropertyType::Select {
            options: vec![choice(9, "Bug"), choice(10, "UI")],
        };
        let status = run(
            select.clone(),
            PropertyType::of(PropertyKind::Status),
            vec![Value::Choice(Uuid::from_u128(10))],
        );
        assert_eq!(status.ty.options().len(), 2);
        assert!(
            status
                .ty
                .options()
                .iter()
                .all(|o| o.group() == Some(StatusGroup::ToDo) && o.hue == Some(25))
        );
        assert_eq!(
            new_values(&status),
            [Some(Value::Choice(Uuid::from_u128(10)))]
        );
        assert_eq!(status.lost, 0);
        let multi = run(
            select,
            PropertyType::of(PropertyKind::MultiSelect),
            vec![Value::Choice(Uuid::from_u128(9))],
        );
        assert_eq!(
            new_values(&multi),
            [Some(Value::Choices(vec![Uuid::from_u128(9)]))]
        );
        assert_eq!(multi.ty.options()[0].group, None);
        let back = run(
            multi.ty,
            PropertyType::of(PropertyKind::Select),
            vec![
                Value::Choices(vec![Uuid::from_u128(10), Uuid::from_u128(9)]),
                Value::Choices(vec![Uuid::from_u128(9)]),
            ],
        );
        assert_eq!(
            new_values(&back),
            [
                Some(Value::Choice(Uuid::from_u128(10))),
                Some(Value::Choice(Uuid::from_u128(9)))
            ]
        );
        // The first row keeps one of its two options.
        assert_eq!(back.lost, 1);
    }

    #[test]
    fn other_values_become_options_named_after_them() {
        let select = run(
            PropertyType::Text,
            PropertyType::of(PropertyKind::Select),
            vec![
                text("Bug"),
                text(" bug "),
                text("Feature"),
                text(&"x".repeat(200)),
            ],
        );
        let names: Vec<&str> = select
            .ty
            .options()
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(names, ["Bug", "Feature"]);
        assert_eq!(
            new_values(&select),
            [
                Some(Value::Choice(Uuid::from_u128(501))),
                Some(Value::Choice(Uuid::from_u128(501))),
                Some(Value::Choice(Uuid::from_u128(502))),
                None
            ]
        );
        assert_eq!(select.lost, 1);
        let multi = run(
            PropertyType::Text,
            PropertyType::of(PropertyKind::MultiSelect),
            vec![text("ui, bug, UI")],
        );
        assert_eq!(
            new_values(&multi),
            [Some(Value::Choices(vec![
                Uuid::from_u128(501),
                Uuid::from_u128(502)
            ]))]
        );
        let status = run(
            PropertyType::Person,
            PropertyType::of(PropertyKind::Status),
            vec![Value::People(vec![Uuid::from_u128(8), Uuid::from_u128(7)])],
        );
        assert_eq!(status.ty.options()[0].name, "Bea");
        assert_eq!(status.ty.options()[0].group(), Some(StatusGroup::ToDo));
        assert_eq!(status.lost, 1);
    }

    #[test]
    fn people_come_only_from_people_and_computed_kinds_keep_nothing() {
        let person = run(PropertyType::Text, PropertyType::Person, vec![text("Ada")]);
        assert_eq!(new_values(&person), [None]);
        assert_eq!(person.lost, 1);
        let computed = run(
            PropertyType::Text,
            PropertyType::CreatedTime,
            vec![text("a"), text("b")],
        );
        assert_eq!(computed.lost, 2);
        assert_eq!(new_values(&computed), [None, None]);
        let id = run(
            PropertyType::Text,
            PropertyType::UniqueId { prefix: None },
            vec![text("a")],
        );
        assert_eq!(id.lost, 1);
        let from_computed = run(PropertyType::LastEditedBy, PropertyType::Text, vec![]);
        assert_eq!(
            from_computed,
            Conversion {
                ty: PropertyType::Text,
                values: vec![],
                lost: 0
            }
        );
    }
}
