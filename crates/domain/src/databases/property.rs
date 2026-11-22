//! A database's properties: their types, the options of those that have some, and the rules
//! for names, options, number formats and unique id prefixes.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MAX_NAME, MAX_OPTIONS, SchemaError};
use crate::hue::MAX_HUE;

text_enum! {
    /// What a property holds, without its settings.
    PropertyKind {
        /// The row's title: its page's title. Every database has exactly one.
        Title => "title",
        Text => "text",
        Number => "number",
        /// One of its options.
        Select => "select",
        /// Any of its options.
        MultiSelect => "multi_select",
        /// One of its options, each in a group: to do, in progress or done.
        Status => "status",
        /// A day or a moment, with an optional end.
        Date => "date",
        /// Members of the workspace.
        Person => "person",
        Checkbox => "checkbox",
        Url => "url",
        /// The row's number in its database, with the property's prefix: `NW-42`.
        UniqueId => "unique_id",
        /// When the row was made, by its page.
        CreatedTime => "created_time",
        /// Who made the row.
        CreatedBy => "created_by",
        /// When the row's page last changed: its title, properties or content.
        LastEditedTime => "last_edited_time",
        /// Who changed it last.
        LastEditedBy => "last_edited_by",
    }
}

impl PropertyKind {
    /// Its values are worked out from the row rather than written: its number, and who
    /// made and changed its page, and when.
    pub fn is_computed(self) -> bool {
        matches!(
            self,
            Self::UniqueId
                | Self::CreatedTime
                | Self::CreatedBy
                | Self::LastEditedTime
                | Self::LastEditedBy
        )
    }

    /// Its values are kept in the row's cells: every kind but the title, which is the
    /// page's, and the computed ones.
    pub fn is_stored(self) -> bool {
        self != Self::Title && !self.is_computed()
    }

    /// It has options to choose from.
    pub fn has_options(self) -> bool {
        matches!(self, Self::Select | Self::MultiSelect | Self::Status)
    }

    /// Its values name members of the workspace, whom guests may not know.
    pub fn names_people(self) -> bool {
        matches!(self, Self::Person | Self::CreatedBy | Self::LastEditedBy)
    }

    /// A board, or any view, may group its rows by it.
    pub fn groups(self) -> bool {
        matches!(self, Self::Select | Self::Status | Self::Person)
    }
}

text_enum! {
    /// How a number shows. Values are any finite number whatever the format.
    NumberFormat {
        /// As it is: `1234.5`.
        Plain => "plain",
        /// Rounded to a whole number: `1235`.
        Integer => "integer",
        /// A hundred times the value, with `%`: `0.25` shows `25%`.
        Percent => "percent",
        /// With two decimals and the property's currency code: `12.50 EUR`.
        Currency => "currency",
    }
}

text_enum! {
    /// The group of a status option.
    StatusGroup {
        ToDo => "to_do",
        InProgress => "in_progress",
        Done => "done",
    }
}

/// An option of a select, multi-select or status property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// Values hold it by its id, so renaming it keeps them.
    pub id: Uuid,
    /// Up to [`MAX_NAME`] characters, unique in its property, ignoring case.
    pub name: String,
    /// 0 to 359; none draws it in grey.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<u16>,
    /// A status option's group; none for other options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<StatusGroup>,
}

impl Choice {
    pub fn group(&self) -> Option<StatusGroup> {
        self.group
    }
}

/// What a property holds, with its settings: the stored form of a property's type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PropertyType {
    Title,
    Text,
    Number {
        format: NumberFormat,
        /// The ISO 4217 code of the currency format, such as `EUR`; none for the others.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        currency: Option<String>,
    },
    Select {
        options: Vec<Choice>,
    },
    MultiSelect {
        options: Vec<Choice>,
    },
    Status {
        options: Vec<Choice>,
    },
    Date,
    Person,
    Checkbox,
    Url,
    UniqueId {
        /// What the number follows, as in `NW-42`; none for numbers alone.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    CreatedTime,
    CreatedBy,
    LastEditedTime,
    LastEditedBy,
}

impl PropertyType {
    pub fn kind(&self) -> PropertyKind {
        match self {
            Self::Title => PropertyKind::Title,
            Self::Text => PropertyKind::Text,
            Self::Number { .. } => PropertyKind::Number,
            Self::Select { .. } => PropertyKind::Select,
            Self::MultiSelect { .. } => PropertyKind::MultiSelect,
            Self::Status { .. } => PropertyKind::Status,
            Self::Date => PropertyKind::Date,
            Self::Person => PropertyKind::Person,
            Self::Checkbox => PropertyKind::Checkbox,
            Self::Url => PropertyKind::Url,
            Self::UniqueId { .. } => PropertyKind::UniqueId,
            Self::CreatedTime => PropertyKind::CreatedTime,
            Self::CreatedBy => PropertyKind::CreatedBy,
            Self::LastEditedTime => PropertyKind::LastEditedTime,
            Self::LastEditedBy => PropertyKind::LastEditedBy,
        }
    }

    /// A type of `kind` with its default settings: plain numbers, no options, no prefix.
    pub fn of(kind: PropertyKind) -> Self {
        match kind {
            PropertyKind::Title => Self::Title,
            PropertyKind::Text => Self::Text,
            PropertyKind::Number => Self::Number {
                format: NumberFormat::Plain,
                currency: None,
            },
            PropertyKind::Select => Self::Select {
                options: Vec::new(),
            },
            PropertyKind::MultiSelect => Self::MultiSelect {
                options: Vec::new(),
            },
            PropertyKind::Status => Self::Status {
                options: Vec::new(),
            },
            PropertyKind::Date => Self::Date,
            PropertyKind::Person => Self::Person,
            PropertyKind::Checkbox => Self::Checkbox,
            PropertyKind::Url => Self::Url,
            PropertyKind::UniqueId => Self::UniqueId { prefix: None },
            PropertyKind::CreatedTime => Self::CreatedTime,
            PropertyKind::CreatedBy => Self::CreatedBy,
            PropertyKind::LastEditedTime => Self::LastEditedTime,
            PropertyKind::LastEditedBy => Self::LastEditedBy,
        }
    }

    /// Its options, for a select, multi-select or status property; empty for the others.
    pub fn options(&self) -> &[Choice] {
        match self {
            Self::Select { options } | Self::MultiSelect { options } | Self::Status { options } => {
                options
            }
            _ => &[],
        }
    }

    /// Its options, to change, when it has some.
    pub fn options_mut(&mut self) -> Option<&mut Vec<Choice>> {
        match self {
            Self::Select { options } | Self::MultiSelect { options } | Self::Status { options } => {
                Some(options)
            }
            _ => None,
        }
    }

    /// The option whose id, or else whose name ignoring case, is `key`.
    pub fn option(&self, key: &str) -> Option<&Choice> {
        let options = self.options();
        Uuid::try_parse(key)
            .ok()
            .and_then(|id| options.iter().find(|option| option.id == id))
            .or_else(|| {
                let key = key.trim();
                options
                    .iter()
                    .find(|option| option.name.to_lowercase() == key.to_lowercase())
            })
    }

    /// Where an option is in its property's order, for sorting.
    pub fn option_index(&self, id: Uuid) -> Option<usize> {
        self.options().iter().position(|option| option.id == id)
    }

    /// The prefix of a unique id property.
    pub fn prefix(&self) -> Option<&str> {
        match self {
            Self::UniqueId { prefix } => prefix.as_deref(),
            _ => None,
        }
    }

    /// The same type with its settings checked and made canonical: a currency code only
    /// with the currency format, upper case, and a prefix in upper case. Options are
    /// checked by [`check_options`] as they change.
    pub fn checked(self) -> Result<Self, SchemaError> {
        match self {
            Self::Number { format, currency } => {
                let currency = match (format, currency) {
                    (NumberFormat::Currency, Some(code)) => Some(check_currency(&code)?),
                    (NumberFormat::Currency, None) => {
                        return Err(SchemaError::Invalid(
                            "the currency format names a currency: an ISO 4217 code such as EUR"
                                .to_owned(),
                        ));
                    }
                    (_, Some(_)) => {
                        return Err(SchemaError::Invalid(
                            "only the currency format names a currency".to_owned(),
                        ));
                    }
                    (_, None) => None,
                };
                Ok(Self::Number { format, currency })
            }
            Self::UniqueId { prefix } => Ok(Self::UniqueId {
                prefix: prefix.map(|prefix| check_prefix(&prefix)).transpose()?,
            }),
            other => Ok(other),
        }
    }
}

/// A property of a database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub id: Uuid,
    /// Up to [`MAX_NAME`] characters, unique in its database, ignoring case.
    pub name: String,
    pub ty: PropertyType,
}

impl Property {
    pub fn kind(&self) -> PropertyKind {
        self.ty.kind()
    }
}

/// A property's or a view's name, trimmed: 1 to [`MAX_NAME`] characters, none of them
/// control characters.
pub fn check_name(raw: &str) -> Result<String, SchemaError> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        Err(SchemaError::Invalid(format!(
            "a name has 1 to {MAX_NAME} characters, and no line breaks or other control characters"
        )))
    } else {
        Ok(name.to_owned())
    }
}

/// The longest unique id prefix, in characters.
pub const MAX_PREFIX: usize = 10;

/// A unique id prefix in upper case: 1 to [`MAX_PREFIX`] ASCII letters and digits, starting
/// with a letter.
pub fn check_prefix(raw: &str) -> Result<String, SchemaError> {
    let prefix = raw.trim().to_ascii_uppercase();
    let valid = (1..=MAX_PREFIX).contains(&prefix.len())
        && prefix.starts_with(|c: char| c.is_ascii_alphabetic())
        && prefix.chars().all(|c| c.is_ascii_alphanumeric());
    if valid {
        Ok(prefix)
    } else {
        Err(SchemaError::Invalid(format!(
            "a unique id prefix has 1 to {MAX_PREFIX} letters and digits, starting with a letter"
        )))
    }
}

/// A currency code in upper case: three ASCII letters, as ISO 4217 has them.
pub fn check_currency(raw: &str) -> Result<String, SchemaError> {
    let code = raw.trim().to_ascii_uppercase();
    if code.len() == 3 && code.chars().all(|c| c.is_ascii_alphabetic()) {
        Ok(code)
    } else {
        Err(SchemaError::Invalid(
            "a currency is an ISO 4217 code of three letters, such as EUR".to_owned(),
        ))
    }
}

/// An option as a change of a property's options asks for it: an existing one by its id,
/// or a new one without.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceSpec {
    pub id: Option<Uuid>,
    pub name: String,
    pub hue: Option<u16>,
    pub group: Option<StatusGroup>,
}

/// The options of a property of `kind` after a change that lists all of them, in their
/// new order: those named by id keep it, renamed and recoloured as asked, and those
/// without get one from `ids`. Every name is checked and kept apart from the others,
/// ignoring case. A status option names its group; other options name none. Options of
/// `current` the list leaves out are deleted: the caller clears them from the rows.
pub fn check_options(
    kind: PropertyKind,
    current: &[Choice],
    specs: Vec<ChoiceSpec>,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<Vec<Choice>, SchemaError> {
    if !kind.has_options() {
        return Err(SchemaError::Invalid(format!(
            "a {kind} property has no options"
        )));
    }
    if specs.len() > MAX_OPTIONS {
        return Err(SchemaError::Invalid(format!(
            "a property has at most {MAX_OPTIONS} options"
        )));
    }
    let mut names = HashSet::new();
    let mut seen = HashSet::new();
    let mut options = Vec::with_capacity(specs.len());
    for spec in specs {
        let name = check_name(&spec.name)?;
        if !names.insert(name.to_lowercase()) {
            return Err(SchemaError::NameTaken(format!(
                "two options are named {name:?}: each option's name is its own"
            )));
        }
        if let Some(hue) = spec.hue
            && hue > MAX_HUE
        {
            return Err(SchemaError::Invalid(format!("a hue is 0 to {MAX_HUE}")));
        }
        let group = match (kind, spec.group) {
            (PropertyKind::Status, Some(group)) => Some(group),
            (PropertyKind::Status, None) => {
                return Err(SchemaError::Invalid(
                    "a status option is in a group: to_do, in_progress or done".to_owned(),
                ));
            }
            (_, Some(_)) => {
                return Err(SchemaError::Invalid(
                    "only status options are in groups".to_owned(),
                ));
            }
            (_, None) => None,
        };
        let id = match spec.id {
            Some(id) if current.iter().any(|option| option.id == id) => id,
            Some(_) => {
                return Err(SchemaError::Invalid(
                    "an option's id is one of the property's: leave it out for a new option"
                        .to_owned(),
                ));
            }
            None => ids(),
        };
        if !seen.insert(id) {
            return Err(SchemaError::Invalid("an option is listed twice".to_owned()));
        }
        options.push(Choice {
            id,
            name,
            hue: spec.hue,
            group,
        });
    }
    Ok(options)
}

/// Grey, blue and green: the hues of the default status options.
const BLUE: u16 = 250;
const GREEN: u16 = 145;

/// The options a status property starts with: Not started, to do; In progress; Done.
pub fn default_status_options(ids: &mut dyn FnMut() -> Uuid) -> Vec<Choice> {
    [
        ("Not started", None, StatusGroup::ToDo),
        ("In progress", Some(BLUE), StatusGroup::InProgress),
        ("Done", Some(GREEN), StatusGroup::Done),
    ]
    .into_iter()
    .map(|(name, hue, group)| Choice {
        id: ids(),
        name: name.to_owned(),
        hue,
        group: Some(group),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> impl FnMut() -> Uuid {
        let mut next = 100u128;
        move || {
            next += 1;
            Uuid::from_u128(next)
        }
    }

    fn spec(id: Option<u128>, name: &str, group: Option<StatusGroup>) -> ChoiceSpec {
        ChoiceSpec {
            id: id.map(Uuid::from_u128),
            name: name.to_owned(),
            hue: None,
            group,
        }
    }

    #[test]
    fn every_kind_round_trips_and_is_classified() {
        for kind in PropertyKind::ALL {
            assert_eq!(PropertyKind::parse(kind.as_str()), Some(*kind));
            assert_eq!(PropertyType::of(*kind).kind(), *kind);
            // Exactly one of: the title, stored, computed.
            let classes = [
                *kind == PropertyKind::Title,
                kind.is_stored(),
                kind.is_computed(),
            ];
            assert_eq!(classes.iter().filter(|c| **c).count(), 1, "{kind}");
        }
        assert_eq!(PropertyKind::ALL.len(), 15);
        let computed: Vec<_> = PropertyKind::ALL
            .iter()
            .filter(|kind| kind.is_computed())
            .map(|kind| kind.as_str())
            .collect();
        assert_eq!(
            computed,
            [
                "unique_id",
                "created_time",
                "created_by",
                "last_edited_time",
                "last_edited_by"
            ]
        );
        let grouping: Vec<_> = PropertyKind::ALL
            .iter()
            .filter(|kind| kind.groups())
            .map(|kind| kind.as_str())
            .collect();
        assert_eq!(grouping, ["select", "status", "person"]);
        let people: Vec<_> = PropertyKind::ALL
            .iter()
            .filter(|kind| kind.names_people())
            .map(|kind| kind.as_str())
            .collect();
        assert_eq!(people, ["person", "created_by", "last_edited_by"]);
        for format in NumberFormat::ALL {
            assert_eq!(NumberFormat::parse(format.as_str()), Some(*format));
        }
        for group in StatusGroup::ALL {
            assert_eq!(StatusGroup::parse(group.as_str()), Some(*group));
        }
    }

    #[test]
    fn types_keep_their_settings_as_stored() {
        let status = PropertyType::Status {
            options: default_status_options(&mut counter()),
        };
        let json = serde_json::to_value(&status).expect("serializes");
        assert_eq!(json["type"], "status");
        assert_eq!(json["options"][0]["name"], "Not started");
        assert_eq!(json["options"][0]["group"], "to_do");
        assert!(json["options"][0].get("hue").is_none());
        assert_eq!(json["options"][1]["hue"], 250);
        assert_eq!(
            serde_json::from_value::<PropertyType>(json).expect("reads back"),
            status
        );
        let number = serde_json::json!({"type": "number", "format": "currency", "currency": "EUR"});
        let read: PropertyType = serde_json::from_value(number.clone()).expect("reads");
        assert_eq!(serde_json::to_value(&read).expect("serializes"), number);
        for bad in [
            serde_json::json!({"type": "number", "format": "roman"}),
            serde_json::json!({"type": "nope"}),
            serde_json::json!({"type": "unique_id", "prefix": "NW", "extra": 1}),
        ] {
            assert!(serde_json::from_value::<PropertyType>(bad).is_err());
        }
    }

    #[test]
    fn number_formats_and_prefixes_are_checked() {
        let currency = |code: Option<&str>, format| PropertyType::Number {
            format,
            currency: code.map(str::to_owned),
        };
        assert_eq!(
            currency(Some(" eur "), NumberFormat::Currency).checked(),
            Ok(currency(Some("EUR"), NumberFormat::Currency))
        );
        assert!(currency(None, NumberFormat::Currency).checked().is_err());
        assert!(
            currency(Some("EUR"), NumberFormat::Plain)
                .checked()
                .is_err()
        );
        assert!(
            currency(Some("EURO"), NumberFormat::Currency)
                .checked()
                .is_err()
        );
        assert!(
            currency(Some("E1R"), NumberFormat::Currency)
                .checked()
                .is_err()
        );
        assert_eq!(
            currency(None, NumberFormat::Percent).checked(),
            Ok(currency(None, NumberFormat::Percent))
        );

        assert_eq!(check_prefix(" nw "), Ok("NW".to_owned()));
        assert_eq!(check_prefix("Task2"), Ok("TASK2".to_owned()));
        for bad in ["", "2NW", "N-W", "NÉ", &"N".repeat(MAX_PREFIX + 1)] {
            assert!(check_prefix(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            PropertyType::UniqueId {
                prefix: Some("nw".into())
            }
            .checked(),
            Ok(PropertyType::UniqueId {
                prefix: Some("NW".into())
            })
        );
        assert_eq!(
            PropertyType::UniqueId { prefix: None }.checked(),
            Ok(PropertyType::UniqueId { prefix: None })
        );
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(check_name("  Status "), Ok("Status".to_owned()));
        assert_eq!(check_name(&"é".repeat(MAX_NAME)), Ok("é".repeat(MAX_NAME)));
        for bad in ["", "   ", "a\nb", &"x".repeat(MAX_NAME + 1)] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn options_keep_their_ids_and_new_ones_get_some() {
        let current = vec![
            Choice {
                id: Uuid::from_u128(1),
                name: "Bug".into(),
                hue: Some(25),
                group: None,
            },
            Choice {
                id: Uuid::from_u128(2),
                name: "Feature".into(),
                hue: None,
                group: None,
            },
        ];
        let changed = check_options(
            PropertyKind::Select,
            &current,
            vec![
                spec(Some(2), "Feature request", None),
                spec(None, " Chore ", None),
            ],
            &mut counter(),
        )
        .expect("valid");
        assert_eq!(
            changed
                .iter()
                .map(|option| (option.id.as_u128(), option.name.as_str()))
                .collect::<Vec<_>>(),
            [(2, "Feature request"), (101, "Chore")]
        );

        let refused = |kind, specs: Vec<ChoiceSpec>| {
            check_options(kind, &current, specs, &mut counter()).expect_err("refused")
        };
        assert!(matches!(
            refused(
                PropertyKind::Select,
                vec![spec(None, "Bug", None), spec(None, "bug", None)]
            ),
            SchemaError::NameTaken(_)
        ));
        refused(PropertyKind::Select, vec![spec(Some(9), "Unknown", None)]);
        refused(
            PropertyKind::Select,
            vec![spec(Some(1), "A", None), spec(Some(1), "B", None)],
        );
        refused(
            PropertyKind::Select,
            vec![spec(None, "Done", Some(StatusGroup::Done))],
        );
        refused(PropertyKind::Status, vec![spec(None, "Done", None)]);
        refused(PropertyKind::Text, vec![spec(None, "A", None)]);
        refused(PropertyKind::Select, vec![spec(None, "", None)]);
        refused(
            PropertyKind::Select,
            vec![ChoiceSpec {
                hue: Some(360),
                ..spec(None, "A", None)
            }],
        );
        refused(
            PropertyKind::MultiSelect,
            (0..=MAX_OPTIONS)
                .map(|i| spec(None, &format!("o{i}"), None))
                .collect(),
        );
        let status = check_options(
            PropertyKind::Status,
            &[],
            vec![spec(None, "Review", Some(StatusGroup::InProgress))],
            &mut counter(),
        )
        .expect("valid");
        assert_eq!(status[0].group(), Some(StatusGroup::InProgress));
    }

    #[test]
    fn options_are_found_by_id_or_name() {
        let ty = PropertyType::Status {
            options: default_status_options(&mut counter()),
        };
        assert_eq!(
            ty.option("in progress").map(|o| o.id),
            Some(Uuid::from_u128(102))
        );
        assert_eq!(
            ty.option(&Uuid::from_u128(103).to_string())
                .map(|o| o.name.as_str()),
            Some("Done")
        );
        assert_eq!(ty.option("Blocked"), None);
        assert_eq!(ty.option_index(Uuid::from_u128(103)), Some(2));
        assert!(PropertyType::Text.options().is_empty());
        assert_eq!(PropertyType::Text.option("x"), None);
        assert_eq!(
            PropertyType::UniqueId {
                prefix: Some("NW".into())
            }
            .prefix(),
            Some("NW")
        );
    }
}
