//! Every operator of every kind, against rows with and without a value, and the rules for
//! resolving filters.

use std::collections::HashMap;

use serde_json::json;
use time::{Date, Month, OffsetDateTime, UtcOffset};

use super::*;
use crate::databases::property::{Choice, Property, PropertyType};
use crate::databases::value::DateValue;

const TITLE: u128 = 1;
const TEXT: u128 = 2;
const URL: u128 = 3;
const NUMBER: u128 = 4;
const SELECT: u128 = 5;
const STATUS: u128 = 6;
const MULTI: u128 = 7;
const DATE: u128 = 8;
const PERSON: u128 = 9;
const CHECKBOX: u128 = 10;
const UNIQUE: u128 = 11;
const CREATED_TIME: u128 = 12;
const CREATED_BY: u128 = 13;
const EDITED_TIME: u128 = 14;
const EDITED_BY: u128 = 15;

const BUG: u128 = 100;
const FEATURE: u128 = 101;
const TODO: u128 = 102;
const DOING: u128 = 103;
const DONE: u128 = 104;
const ME: u128 = 200;
const BEA: u128 = 201;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn choice(n: u128, name: &str, group: Option<StatusGroup>) -> Choice {
    Choice {
        id: id(n),
        name: name.into(),
        hue: None,
        group,
    }
}

fn schema() -> Schema {
    let options = vec![choice(BUG, "Bug", None), choice(FEATURE, "Feature", None)];
    let property = |n, name: &str, ty| Property {
        id: id(n),
        name: name.into(),
        ty,
    };
    Schema {
        properties: vec![
            property(TITLE, "Name", PropertyType::Title),
            property(TEXT, "Notes", PropertyType::Text),
            property(URL, "Link", PropertyType::Url),
            property(NUMBER, "Points", PropertyType::of(PropertyKind::Number)),
            property(
                SELECT,
                "Type",
                PropertyType::Select {
                    options: options.clone(),
                },
            ),
            property(
                STATUS,
                "Status",
                PropertyType::Status {
                    options: vec![
                        choice(TODO, "Not started", Some(StatusGroup::ToDo)),
                        choice(DOING, "In progress", Some(StatusGroup::InProgress)),
                        choice(DONE, "Done", Some(StatusGroup::Done)),
                    ],
                },
            ),
            property(MULTI, "Tags", PropertyType::MultiSelect { options }),
            property(DATE, "Due", PropertyType::Date),
            property(PERSON, "Owner", PropertyType::Person),
            property(CHECKBOX, "Shipped", PropertyType::Checkbox),
            property(
                UNIQUE,
                "ID",
                PropertyType::UniqueId {
                    prefix: Some("NW".into()),
                },
            ),
            property(CREATED_TIME, "Created", PropertyType::CreatedTime),
            property(CREATED_BY, "Creator", PropertyType::CreatedBy),
            property(EDITED_TIME, "Edited", PropertyType::LastEditedTime),
            property(EDITED_BY, "Editor", PropertyType::LastEditedBy),
        ],
    }
}

fn day(y: i32, m: Month, d: u8) -> Date {
    Date::from_calendar_date(y, m, d).expect("a day")
}

/// Today is 2026-11-22, at UTC+2.
fn context() -> Context {
    Context {
        me: id(ME),
        offset: UtcOffset::from_hms(2, 0, 0).expect("an offset"),
        today: day(2026, Month::November, 22),
    }
}

/// 2026-11-21 23:30 UTC: 2026-11-22 at UTC+2.
fn late_evening() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_795_303_800).expect("a time")
}

/// A row with every property set.
fn full() -> Row {
    let cells = HashMap::from([
        (id(TEXT), Value::Text("Fix the Login page".into())),
        (id(URL), Value::Text("https://example.com/Login".into())),
        (id(NUMBER), Value::Number(5.0)),
        (id(SELECT), Value::Choice(id(BUG))),
        (id(STATUS), Value::Choice(id(DOING))),
        (id(MULTI), Value::Choices(vec![id(BUG)])),
        (
            id(DATE),
            Value::Date(DateValue {
                start: DatePoint::Day(day(2026, Month::November, 20)),
                end: Some(DatePoint::Day(day(2026, Month::November, 30))),
            }),
        ),
        (id(PERSON), Value::People(vec![id(ME), id(BEA)])),
        (id(CHECKBOX), Value::Checked),
    ]);
    Row {
        id: id(1000),
        position: "a0".into(),
        number: 42,
        title: "Login bug".into(),
        created_at: late_evening(),
        created_by: Some(id(ME)),
        last_edited_at: late_evening(),
        last_edited_by: Some(id(BEA)),
        cells,
    }
}

/// A row with nothing set but what every row has.
fn empty() -> Row {
    Row {
        title: String::new(),
        created_by: None,
        last_edited_by: None,
        cells: HashMap::new(),
        ..full()
    }
}

/// Whether the full row and the empty one pass a condition.
fn passes(property: u128, operator: Operator, value: serde_json::Value) -> (bool, bool) {
    let schema = schema();
    let spec = FilterSpec::Condition {
        property: id(property).to_string(),
        operator,
        value,
    };
    let filter = spec
        .resolve(&schema, Fit::Strict)
        .unwrap_or_else(|error| panic!("{operator} on {property}: {error}"))
        .expect("a filter");
    (
        filter.matches(&schema, &full(), &context()),
        filter.matches(&schema, &empty(), &context()),
    )
}

fn refused(property: u128, operator: Operator, value: serde_json::Value) {
    let spec = FilterSpec::Condition {
        property: id(property).to_string(),
        operator,
        value: value.clone(),
    };
    assert!(
        spec.resolve(&schema(), Fit::Strict).is_err(),
        "{operator} on {property} with {value} is refused"
    );
}

use Operator::*;

#[test]
fn every_kind_takes_the_empty_checks_but_the_checkbox() {
    for property in [
        TITLE, TEXT, URL, NUMBER, SELECT, STATUS, MULTI, DATE, PERSON, CREATED_BY, EDITED_BY,
    ] {
        assert_eq!(
            passes(property, IsEmpty, json!(null)),
            (false, true),
            "{property}"
        );
        assert_eq!(
            passes(property, IsNotEmpty, json!(null)),
            (true, false),
            "{property}"
        );
    }
    // Computed values that every row has are never empty.
    for property in [UNIQUE, CREATED_TIME, EDITED_TIME] {
        assert_eq!(
            passes(property, IsEmpty, json!(null)),
            (false, false),
            "{property}"
        );
        assert_eq!(
            passes(property, IsNotEmpty, json!(null)),
            (true, true),
            "{property}"
        );
    }
    refused(CHECKBOX, IsEmpty, json!(null));
}

#[test]
fn texts_compare_ignoring_case() {
    for property in [TITLE, TEXT, URL] {
        let value = match property {
            TITLE => "login bug",
            TEXT => "fix the login page",
            _ => "https://example.com/login",
        };
        assert_eq!(
            passes(property, Is, json!(value)),
            (true, false),
            "{property}"
        );
        assert_eq!(
            passes(property, IsNot, json!(value)),
            (false, true),
            "{property}"
        );
        assert_eq!(
            passes(property, IsNot, json!("other")),
            (true, true),
            "{property}"
        );
        assert_eq!(
            passes(property, Contains, json!("LOGIN")),
            (true, false),
            "{property}"
        );
        assert_eq!(
            passes(property, Contains, json!("logout")),
            (false, false),
            "{property}"
        );
        assert_eq!(
            passes(property, DoesNotContain, json!("login")),
            (false, true),
            "{property}"
        );
        assert_eq!(
            passes(property, DoesNotContain, json!("logout")),
            (true, true),
            "{property}"
        );
        let start = &value[..3];
        let end = &value[value.len() - 3..];
        assert_eq!(
            passes(property, StartsWith, json!(start)),
            (true, false),
            "{property}"
        );
        assert_eq!(
            passes(property, StartsWith, json!(end)),
            (false, false),
            "{property}"
        );
        assert_eq!(
            passes(property, EndsWith, json!(end)),
            (true, false),
            "{property}"
        );
        assert_eq!(
            passes(property, EndsWith, json!(start)),
            (false, false),
            "{property}"
        );
        refused(property, Is, json!(""));
        refused(property, Contains, json!(3));
        refused(property, Contains, json!("x".repeat(MAX_FILTER_TEXT + 1)));
        refused(property, Equals, json!("x"));
        refused(property, Before, json!("2026-11-22"));
    }
}

#[test]
fn numbers_and_unique_ids_compare_as_numbers() {
    // Points is 5; the row's number is 42.
    for (property, n) in [(NUMBER, 5.0), (UNIQUE, 42.0)] {
        assert_eq!(
            passes(property, Equals, json!(n)),
            (true, property == UNIQUE),
            "{property}"
        );
        assert_eq!(
            passes(property, NotEquals, json!(n)),
            (false, property == NUMBER),
            "{property}"
        );
        assert!(passes(property, GreaterThan, json!(n - 1.0)).0);
        assert!(!passes(property, GreaterThan, json!(n)).0);
        assert!(passes(property, GreaterThanOrEqual, json!(n)).0);
        assert!(passes(property, LessThan, json!(n + 0.5)).0);
        assert!(!passes(property, LessThan, json!(n)).0);
        assert!(passes(property, LessThanOrEqual, json!(n)).0);
        refused(property, Equals, json!("5"));
        refused(property, Is, json!(5));
    }
    // Without a value, only the negative comparison holds.
    for operator in [
        GreaterThan,
        GreaterThanOrEqual,
        LessThan,
        LessThanOrEqual,
        Equals,
    ] {
        assert!(!passes(NUMBER, operator, json!(0)).1, "{operator}");
    }
}

#[test]
fn selects_and_statuses_compare_options_by_id_or_name() {
    assert_eq!(passes(SELECT, Is, json!("bug")), (true, false));
    assert_eq!(
        passes(SELECT, Is, json!(id(FEATURE).to_string())),
        (false, false)
    );
    assert_eq!(passes(SELECT, IsNot, json!("Bug")), (false, true));
    assert_eq!(passes(SELECT, IsNot, json!("Feature")), (true, true));
    refused(SELECT, Is, json!("Chore"));
    refused(SELECT, Contains, json!("Bug"));
    refused(SELECT, InGroup, json!("done"));

    assert_eq!(passes(STATUS, Is, json!("In progress")), (true, false));
    assert_eq!(passes(STATUS, IsNot, json!("Done")), (true, true));
    assert_eq!(passes(STATUS, InGroup, json!("in_progress")), (true, false));
    assert_eq!(passes(STATUS, InGroup, json!("done")), (false, false));
    refused(STATUS, InGroup, json!("blocked"));
    refused(STATUS, Is, json!("in_progress"));
}

#[test]
fn multi_selects_contain_options() {
    assert_eq!(passes(MULTI, Contains, json!("Bug")), (true, false));
    assert_eq!(passes(MULTI, Contains, json!("Feature")), (false, false));
    assert_eq!(passes(MULTI, DoesNotContain, json!("Bug")), (false, true));
    assert_eq!(
        passes(MULTI, DoesNotContain, json!("Feature")),
        (true, true)
    );
    refused(MULTI, Is, json!("Bug"));
    refused(MULTI, Contains, json!("Chore"));
}

#[test]
fn dates_compare_by_the_day_of_their_start() {
    // Due starts on the 20th; the row was made and edited on the 22nd at UTC+2.
    let cases = [
        (DATE, "2026-11-20"),
        (CREATED_TIME, "2026-11-22"),
        (EDITED_TIME, "2026-11-22"),
    ];
    for (property, on) in cases {
        // The empty row has the same computed times as the full one.
        let both = |full: bool| (full, full && property != DATE);
        assert_eq!(passes(property, Is, json!(on)), both(true), "{property}");
        assert_eq!(
            passes(property, Before, json!(on)),
            both(false),
            "{property}"
        );
        assert_eq!(
            passes(property, OnOrBefore, json!(on)),
            both(true),
            "{property}"
        );
        assert_eq!(
            passes(property, After, json!(on)),
            both(false),
            "{property}"
        );
        assert_eq!(
            passes(property, OnOrAfter, json!(on)),
            both(true),
            "{property}"
        );
        assert_eq!(
            passes(property, Within, json!("past_week")),
            both(true),
            "{property}"
        );
    }
    assert_eq!(passes(DATE, Before, json!("today")), (true, false));
    assert_eq!(passes(DATE, Is, json!("today")), (false, false));
    assert!(passes(CREATED_TIME, Is, json!("today")).0);
    assert!(!passes(CREATED_TIME, Is, json!("yesterday")).0);
    assert!(passes(DATE, After, json!("one_week_ago")).0);
    assert!(passes(DATE, Before, json!("tomorrow")).0);
    assert!(passes(DATE, Before, json!("one_week_from_now")).0);
    assert!(passes(DATE, After, json!("one_month_ago")).0);
    assert!(passes(DATE, Before, json!("one_month_from_now")).0);
    assert!(passes(DATE, Within, json!("past_month")).0);
    assert!(passes(DATE, Within, json!("past_year")).0);
    assert!(!passes(DATE, Within, json!("next_week")).0);
    assert!(!passes(DATE, Within, json!("next_month")).0);
    assert!(!passes(DATE, Within, json!("next_year")).0);
    assert!(passes(CREATED_TIME, Within, json!("next_week")).0);
    refused(DATE, Is, json!("22/11/2026"));
    refused(DATE, Within, json!("today"));
    refused(DATE, Is, json!("past_week"));
    refused(DATE, Contains, json!("2026"));
}

#[test]
fn relative_days_and_ranges_count_from_today() {
    let today = day(2026, Month::March, 31);
    assert_eq!(RelativeDay::Today.on(today), today);
    assert_eq!(RelativeDay::Tomorrow.on(today), day(2026, Month::April, 1));
    assert_eq!(
        RelativeDay::Yesterday.on(today),
        day(2026, Month::March, 30)
    );
    assert_eq!(
        RelativeDay::OneWeekAgo.on(today),
        day(2026, Month::March, 24)
    );
    assert_eq!(
        RelativeDay::OneWeekFromNow.on(today),
        day(2026, Month::April, 7)
    );
    // Months end early: the 31st a month back is the last of February.
    assert_eq!(
        RelativeDay::OneMonthAgo.on(today),
        day(2026, Month::February, 28)
    );
    assert_eq!(
        RelativeDay::OneMonthFromNow.on(today),
        day(2026, Month::April, 30)
    );
    assert_eq!(
        RelativeRange::PastYear.days(today),
        (day(2025, Month::March, 31), today)
    );
    assert_eq!(
        RelativeRange::NextYear.days(day(2028, Month::February, 29)),
        (
            day(2028, Month::February, 29),
            day(2029, Month::February, 28)
        )
    );
    assert_eq!(
        RelativeRange::NextMonth.days(day(2026, Month::December, 15)),
        (
            day(2026, Month::December, 15),
            day(2027, Month::January, 15)
        )
    );
    assert_eq!(
        add_months(day(2026, Month::January, 31), -13),
        day(2024, Month::December, 31)
    );
}

#[test]
fn people_contain_members_or_me() {
    for property in [PERSON, CREATED_BY] {
        assert_eq!(
            passes(property, Contains, json!("me")),
            (true, false),
            "{property}"
        );
        assert_eq!(
            passes(property, DoesNotContain, json!("me")),
            (false, true),
            "{property}"
        );
    }
    assert_eq!(passes(EDITED_BY, Contains, json!("me")), (false, false));
    assert_eq!(
        passes(EDITED_BY, Contains, json!(id(BEA).to_string())),
        (true, false)
    );
    assert_eq!(
        passes(PERSON, Contains, json!(id(BEA).to_string())),
        (true, false)
    );
    assert_eq!(
        passes(PERSON, DoesNotContain, json!(id(999).to_string())),
        (true, true)
    );
    refused(PERSON, Contains, json!("Bea"));
    refused(PERSON, Is, json!("me"));
    refused(CREATED_BY, Contains, json!(null));
}

#[test]
fn checkboxes_are_checked_or_not() {
    assert_eq!(passes(CHECKBOX, Is, json!(true)), (true, false));
    assert_eq!(passes(CHECKBOX, Is, json!(false)), (false, true));
    refused(CHECKBOX, Is, json!("yes"));
    refused(CHECKBOX, IsNot, json!(true));
}

#[test]
fn every_kind_lists_its_operators() {
    for kind in PropertyKind::ALL {
        let listed = operators(*kind);
        assert!(!listed.is_empty(), "{kind}");
        if *kind != PropertyKind::Checkbox {
            assert!(
                listed.contains(&IsEmpty) && listed.contains(&IsNotEmpty),
                "{kind}"
            );
        }
    }
    let used: std::collections::HashSet<Operator> = PropertyKind::ALL
        .iter()
        .flat_map(|kind| operators(*kind).iter().copied())
        .collect();
    assert_eq!(
        used.len(),
        Operator::ALL.len(),
        "every operator is for some kind"
    );
}

#[test]
fn groups_combine_conditions() {
    let schema = schema();
    let spec: FilterSpec = serde_json::from_value(json!({"or": [
        {"property": "Points", "operator": "greater_than", "value": 10},
        {"and": [
            {"property": "Type", "operator": "is", "value": "Bug"},
            {"property": "Owner", "operator": "contains", "value": "me"}
        ]}
    ]}))
    .expect("a spec");
    let filter = spec
        .resolve(&schema, Fit::Strict)
        .expect("valid")
        .expect("a filter");
    assert!(filter.matches(&schema, &full(), &context()));
    assert!(!filter.matches(&schema, &empty(), &context()));
    let mut other = full();
    other.cells.insert(id(SELECT), Value::Choice(id(FEATURE)));
    assert!(!filter.matches(&schema, &other, &context()));
    other.cells.insert(id(NUMBER), Value::Number(11.0));
    assert!(filter.matches(&schema, &other, &context()));
}

#[test]
fn filters_keep_to_their_bounds() {
    let schema = schema();
    let condition = || json!({"property": "Name", "operator": "is_empty"});
    let nested = json!({"and": [{"or": [{"and": [condition()]}]}]});
    assert!(
        serde_json::from_value::<FilterSpec>(nested)
            .expect("a spec")
            .resolve(&schema, Fit::Strict)
            .is_ok()
    );
    let too_deep = json!({"and": [{"or": [{"and": [{"or": [condition()]}]}]}]});
    assert!(
        serde_json::from_value::<FilterSpec>(too_deep)
            .expect("a spec")
            .resolve(&schema, Fit::Strict)
            .is_err()
    );
    let many = json!({"and": (0..=MAX_CONDITIONS).map(|_| condition()).collect::<Vec<_>>()});
    assert!(
        serde_json::from_value::<FilterSpec>(many)
            .expect("a spec")
            .resolve(&schema, Fit::Strict)
            .is_err()
    );
    let empty = FilterSpec::And {
        and: vec![FilterSpec::Or { or: vec![] }],
    };
    assert_eq!(empty.resolve(&schema, Fit::Strict), Ok(None));
    let unknown = FilterSpec::Condition {
        property: "Nope".into(),
        operator: IsEmpty,
        value: json!(null),
    };
    assert!(unknown.resolve(&schema, Fit::Strict).is_err());
    assert_eq!(unknown.resolve(&schema, Fit::Lenient), Ok(None));
}

#[test]
fn a_text_condition_keeps_the_case_it_was_given() {
    let spec = FilterSpec::Condition {
        property: "Notes".into(),
        operator: Contains,
        value: json!("Login"),
    };
    let filter = spec
        .resolve(&schema(), Fit::Strict)
        .expect("valid")
        .expect("a filter");
    let FilterSpec::Condition { value, .. } = filter.to_spec() else {
        panic!("a condition");
    };
    assert_eq!(value, json!("Login"));
}

#[test]
fn a_filter_reads_back_from_its_spec() {
    let schema = schema();
    for (property, operator, value) in [
        (TEXT, Contains, json!("Login")),
        (NUMBER, GreaterThan, json!(2.5)),
        (SELECT, Is, json!("Bug")),
        (STATUS, InGroup, json!("done")),
        (DATE, OnOrAfter, json!("2026-11-01")),
        (DATE, Before, json!("one_week_from_now")),
        (CREATED_TIME, Within, json!("past_month")),
        (PERSON, Contains, json!("me")),
        (PERSON, Contains, json!(id(BEA).to_string())),
        (CHECKBOX, Is, json!(false)),
        (TITLE, IsNotEmpty, json!(null)),
    ] {
        let spec = FilterSpec::Condition {
            property: id(property).to_string(),
            operator,
            value,
        };
        let filter = spec
            .resolve(&schema, Fit::Strict)
            .expect("valid")
            .expect("a filter");
        let kept = serde_json::to_value(filter.to_spec()).expect("serializes");
        let back: FilterSpec = serde_json::from_value(kept).expect("reads back");
        assert_eq!(
            back.resolve(&schema, Fit::Strict).expect("valid"),
            Some(filter),
            "{operator} on {property}"
        );
    }
}

#[test]
fn a_condition_on_a_deleted_property_filters_nothing() {
    let mut schema = schema();
    let filter = FilterSpec::Condition {
        property: "Points".into(),
        operator: GreaterThan,
        value: json!(100),
    }
    .resolve(&schema, Fit::Strict)
    .expect("valid")
    .expect("a filter");
    assert!(!filter.matches(&schema, &full(), &context()));
    schema
        .properties
        .retain(|property| property.id != id(NUMBER));
    assert!(filter.matches(&schema, &full(), &context()));
}
