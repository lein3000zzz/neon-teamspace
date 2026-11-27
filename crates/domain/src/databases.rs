//! Databases: pages whose subpages are rows, each a page with the database's properties
//! and a block body of its own.
//!
//! A database's schema is its properties, in order: exactly one title, which is each row's
//! page title, and others of the kinds [`PropertyKind`] lists. A row keeps the values of the
//! stored kinds in its cells, by property id; the title is its page's, and the computed
//! kinds come from the row: its number in the database, which a unique id shows after its
//! prefix, and who made and changed its page, and when. Each change of a value replaces
//! that property's value alone, so two people changing different properties of a row at
//! once both keep theirs, and of two changes to one property, the last wins.
//!
//! Its views lay the rows out as a table, a board or a list, each with a filter, sorts, a
//! grouping and the properties it shows. Filtering, sorting and grouping are done here,
//! over the rows the caller may see: [`query::run`]. A database holds at most
//! [`MAX_ROWS`] rows out of the trash, and a row's values at most [`MAX_ROW_BYTES`], so
//! that a query reads every row at once, with the values its filter, sorts and grouping
//! need, in bounded memory.
//!
//! Who may see and change a database is the pages' rules': its rows take their access from
//! it, as any subpage does.

pub mod convert;
pub mod filter;
mod property;
pub mod query;
mod schema;
pub mod sort;
mod value;
pub mod view;

pub use property::{
    Choice, ChoiceSpec, MAX_PREFIX, NumberFormat, Property, PropertyKind, PropertyType,
    StatusGroup, check_currency, check_name, check_options, check_prefix, default_status_options,
};
pub use schema::{Schema, default_properties};
pub use value::{DatePoint, DateValue, InvalidValue, Row, Value, check_url};
pub use view::{GroupKey, Grouping, Layout, View, ViewProperty, ViewSettings, ViewSpec};

/// The most rows a database holds out of the trash.
pub const MAX_ROWS: usize = 10_000;

/// The most bytes a row's stored values take together, as JSON text: 64 KiB, room for a
/// text of the longest, [`MAX_TEXT`] characters of four bytes each, and more.
pub const MAX_ROW_BYTES: usize = 64 * 1024;

/// The most properties a database has.
pub const MAX_PROPERTIES: usize = 100;

/// The most views a database has.
pub const MAX_VIEWS: usize = 50;

/// The longest name of a property, an option or a view, in characters.
pub const MAX_NAME: usize = 100;

/// The most options a select, multi-select or status property has.
pub const MAX_OPTIONS: usize = 200;

/// The longest text value, in characters.
pub const MAX_TEXT: usize = 10_000;

/// The longest URL value, in bytes.
pub const MAX_URL: usize = 2048;

/// The most people a person value names.
pub const MAX_PEOPLE: usize = 50;

/// How deep filter groups nest: the outer group and two within.
pub const MAX_FILTER_DEPTH: usize = 3;

/// The most conditions one filter has.
pub const MAX_CONDITIONS: usize = 50;

/// The longest text a condition compares with, in characters.
pub const MAX_FILTER_TEXT: usize = 1000;

/// The most sorts a view has.
pub const MAX_SORTS: usize = 10;

/// The most groups a view orders or hides by hand.
pub const MAX_GROUP_KEYS: usize = 500;

/// The narrowest and widest column, in pixels.
pub const MIN_WIDTH: u16 = 40;
pub const MAX_WIDTH: u16 = 2000;

/// A change to a schema that its rules refuse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    #[error("{0}")]
    Invalid(String),
    /// Another property, or option, has the name.
    #[error("{0}")]
    NameTaken(String),
    /// Past [`MAX_PROPERTIES`].
    #[error("{0}")]
    TooMany(String),
    #[error("no such property")]
    NoProperty,
}

/// A view's settings that do not fit the schema, or their bounds.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidView(pub String);

/// A row's unique id as people write it: the property's prefix, a dash and the row's
/// number, `NW-42`, or the number alone without a prefix.
pub fn unique_id(prefix: Option<&str>, number: i64) -> String {
    match prefix {
        Some(prefix) => format!("{prefix}-{number}"),
        None => number.to_string(),
    }
}

/// A unique id that names no row of the database.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidUniqueId(pub String);

/// The row number `text` names, as [`unique_id`] writes it, in a database whose unique id
/// property has `prefix`: `NW-42`, its prefix in any case, or `42` alone.
pub fn parse_unique_id(text: &str, prefix: Option<&str>) -> Result<i64, InvalidUniqueId> {
    let text = text.trim();
    let digits = match text.rsplit_once('-') {
        Some((given, number)) => {
            if !prefix.is_some_and(|prefix| prefix.eq_ignore_ascii_case(given)) {
                return Err(InvalidUniqueId(match prefix {
                    Some(prefix) => format!(
                        "{text:?} is not a unique id of this database, whose ids look like \
                         {prefix}-42"
                    ),
                    None => format!(
                        "{text:?} is not a unique id of this database, whose ids are numbers"
                    ),
                }));
            }
            number
        }
        None => text,
    };
    // Digits alone, without a sign or leading zeros.
    digits
        .parse::<i64>()
        .ok()
        .filter(|number| *number >= 1 && number.to_string() == digits)
        .ok_or_else(|| InvalidUniqueId(format!("{text:?} names no row number")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_ids_are_a_prefix_and_a_number() {
        assert_eq!(unique_id(Some("NW"), 42), "NW-42");
        assert_eq!(unique_id(None, 7), "7");
        assert_eq!(parse_unique_id("NW-42", Some("NW")), Ok(42));
        assert_eq!(parse_unique_id(" nw-42 ", Some("NW")), Ok(42));
        assert_eq!(parse_unique_id("42", Some("NW")), Ok(42));
        assert_eq!(parse_unique_id("42", None), Ok(42));
        for (bad, prefix) in [
            ("XX-42", Some("NW")),
            ("NW-42", None),
            ("NW-", Some("NW")),
            ("NW-0", Some("NW")),
            ("NW--4", Some("NW")),
            ("NW-+4", Some("NW")),
            ("NW-04", Some("NW")),
            ("forty", None),
            ("", None),
        ] {
            assert!(parse_unique_id(bad, prefix).is_err(), "{bad:?}");
        }
    }
}
