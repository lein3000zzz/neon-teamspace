//! What an incoming webhook takes: a small JSON format of its own, version 1, read strictly.
//!
//! The body is one object, with `version`, `1`, and `action`, which says what to do with
//! the webhook's target:
//!
//! | `action` | Target | Fields |
//! |---|---|---|
//! | `create_row` | a database | `values`: an object of values by property name or id, as the API's rows take them |
//! | `update_row` | a database | `unique_id`, such as `NW-42` or `42`, and `values`, at least one |
//! | `append` | a page | `markdown`: what to add at the end of the page, at least one block, nested at most [`MAX_DEPTH`] levels deep |
//! | `comment` | a page | `markdown`: a comment, its inline content alone, on the whole page or, with `block_id`, on that block's text |
//!
//! Any other field, a missing one, another version or an action the target does not take is
//! refused, saying what is wrong.

use neon_blocks::Block;
use neon_blocks::markdown::{MAX_DEPTH, try_from_markdown};
use neon_domain::comments::check_comment;
use neon_domain::webhooks::PAYLOAD_VERSION;
use serde::Deserialize;
use serde_json::{Map, Value as Json};

/// The kind of page an incoming webhook works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// Its rows are made and changed.
    Database,
    /// Blocks are added at its end.
    Page,
}

impl TargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Page => "page",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "database" => Some(Self::Database),
            "page" => Some(Self::Page),
            _ => None,
        }
    }
}

/// What a request asks for, read and checked against its target.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Order {
    CreateRow {
        values: Map<String, Json>,
    },
    UpdateRow {
        unique_id: String,
        values: Map<String, Json>,
    },
    Append {
        blocks: Vec<Block>,
    },
    Comment {
        markdown: String,
        block_id: Option<String>,
    },
}

impl Order {
    /// Its action's name, as the payload and the log have it.
    pub(crate) fn action(&self) -> &'static str {
        match self {
            Self::CreateRow { .. } => "create_row",
            Self::UpdateRow { .. } => "update_row",
            Self::Append { .. } => "append",
            Self::Comment { .. } => "comment",
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Payload {
    CreateRow {
        version: u32,
        values: Map<String, Json>,
    },
    UpdateRow {
        version: u32,
        unique_id: String,
        values: Map<String, Json>,
    },
    Append {
        version: u32,
        markdown: String,
    },
    Comment {
        version: u32,
        markdown: String,
        #[serde(default)]
        block_id: Option<String>,
    },
}

/// The fields an action takes beside `action`, in the order its message names them.
fn fields_of(action: &str) -> Option<&'static [&'static str]> {
    match action {
        "create_row" => Some(&["version", "values"]),
        "update_row" => Some(&["version", "unique_id", "values"]),
        "append" => Some(&["version", "markdown"]),
        "comment" => Some(&["version", "markdown", "block_id"]),
        _ => None,
    }
}

/// The action an incoming request asks for, or what is wrong with it.
pub(crate) fn read(body: &[u8], target: TargetKind) -> Result<Order, String> {
    let json: Json =
        serde_json::from_slice(body).map_err(|error| format!("the body is not JSON: {error}"))?;
    let Some(object) = json.as_object() else {
        return Err("the body is a JSON object".to_owned());
    };
    if let Some(action) = object.get("action").and_then(Json::as_str)
        && let Some(fields) = fields_of(action)
        && let Some(unknown) = object
            .keys()
            .find(|key| *key != "action" && !fields.contains(&key.as_str()))
    {
        let (last, rest) = fields.split_last().unwrap_or((&"", &[]));
        return Err(format!(
            "unknown field `{unknown}`: {action} takes action, {} and {last}",
            rest.join(", ")
        ));
    }
    let payload: Payload = serde_json::from_value(json).map_err(|error| error.to_string())?;
    let (Payload::CreateRow { version, .. }
    | Payload::UpdateRow { version, .. }
    | Payload::Append { version, .. }
    | Payload::Comment { version, .. }) = &payload;
    if *version != PAYLOAD_VERSION {
        return Err(format!(
            "unknown version {version}: this server takes version {PAYLOAD_VERSION}"
        ));
    }
    let order = match payload {
        Payload::CreateRow { values, .. } => Order::CreateRow { values },
        Payload::UpdateRow {
            unique_id, values, ..
        } => {
            if unique_id.trim().is_empty() {
                return Err("`unique_id` names a row, such as NW-42".to_owned());
            }
            if values.is_empty() {
                return Err("`values` names at least one property to change".to_owned());
            }
            Order::UpdateRow { unique_id, values }
        }
        Payload::Append { markdown, .. } => {
            let blocks = try_from_markdown(&markdown)
                .map_err(|_| format!("`markdown` nests blocks more than {MAX_DEPTH} levels deep"))?
                .blocks;
            if blocks.is_empty() {
                return Err("`markdown` holds nothing to add".to_owned());
            }
            Order::Append { blocks }
        }
        Payload::Comment {
            markdown, block_id, ..
        } => {
            check_comment(&markdown).map_err(|error| format!("`markdown`: {error}"))?;
            if block_id.as_deref().is_some_and(|id| id.trim().is_empty()) {
                return Err("`block_id` names a block of the page, or is left out".to_owned());
            }
            Order::Comment { markdown, block_id }
        }
    };
    match (target, &order) {
        (TargetKind::Database, Order::CreateRow { .. } | Order::UpdateRow { .. })
        | (TargetKind::Page, Order::Append { .. } | Order::Comment { .. }) => Ok(order),
        (TargetKind::Database, Order::Append { .. } | Order::Comment { .. }) => Err(
            "this webhook's target is a database: it takes create_row and update_row".to_owned(),
        ),
        (TargetKind::Page, _) => {
            Err("this webhook's target is a page: it takes append and comment".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn read_json(json: Json, target: TargetKind) -> Result<Order, String> {
        read(json.to_string().as_bytes(), target)
    }

    #[test]
    fn each_action_reads_for_its_target() {
        let made = read_json(
            json!({"version": 1, "action": "create_row", "values": {"Name": "Deploy"}}),
            TargetKind::Database,
        );
        assert!(matches!(made, Ok(Order::CreateRow { ref values }) if values["Name"] == "Deploy"));
        let changed = read_json(
            json!({"version": 1, "action": "update_row", "unique_id": "NW-42", "values": {"Status": "Done"}}),
            TargetKind::Database,
        );
        assert!(
            matches!(changed, Ok(Order::UpdateRow { ref unique_id, .. }) if unique_id == "NW-42")
        );
        let appended = read_json(
            json!({"version": 1, "action": "append", "markdown": "## Deployed\n\n- web\n- server"}),
            TargetKind::Page,
        )
        .expect("an append");
        assert_eq!(appended.action(), "append");
        assert!(matches!(appended, Order::Append { ref blocks } if blocks.len() == 3));
        let commented = read_json(
            json!({"version": 1, "action": "comment", "markdown": "Deployed **v2**"}),
            TargetKind::Page,
        )
        .expect("a comment");
        assert_eq!(commented.action(), "comment");
        assert!(
            matches!(commented, Order::Comment { ref markdown, block_id: None } if markdown == "Deployed **v2**")
        );
        let anchored = read_json(
            json!({"version": 1, "action": "comment", "markdown": "Here", "block_id": "b1"}),
            TargetKind::Page,
        );
        assert!(
            matches!(anchored, Ok(Order::Comment { block_id: Some(ref id), .. }) if id == "b1")
        );
    }

    #[test]
    fn comments_are_for_pages_and_hold_some_text() {
        let refused = |json: Json, target| read_json(json, target).expect_err("refused");
        assert_eq!(
            refused(
                json!({"version": 1, "action": "comment", "markdown": "x"}),
                TargetKind::Database
            ),
            "this webhook's target is a database: it takes create_row and update_row"
        );
        assert_eq!(
            refused(
                json!({"version": 1, "action": "comment", "markdown": "  "}),
                TargetKind::Page
            ),
            "`markdown`: a comment holds some text"
        );
        assert_eq!(
            refused(
                json!({"version": 1, "action": "comment", "markdown": "x", "block_id": " "}),
                TargetKind::Page
            ),
            "`block_id` names a block of the page, or is left out"
        );
        assert!(
            refused(
                json!({"version": 1, "action": "comment", "markdown": "x", "pin": true}),
                TargetKind::Page
            )
            .starts_with(
                "unknown field `pin`: comment takes action, version, markdown and block_id"
            )
        );
    }

    #[test]
    fn anything_else_is_refused_saying_why() {
        let refused = |json: Json, target| read_json(json, target).expect_err("refused");
        let db = TargetKind::Database;

        assert!(
            read(b"not json", db)
                .expect_err("refused")
                .starts_with("the body is not JSON")
        );
        assert_eq!(refused(json!([1]), db), "the body is a JSON object");
        assert_eq!(
            refused(json!({"action": "create_row", "values": {}}), db),
            "missing field `version`"
        );
        assert_eq!(
            refused(
                json!({"version": 2, "action": "create_row", "values": {}}),
                db
            ),
            "unknown version 2: this server takes version 1"
        );
        assert!(
            refused(
                json!({"version": "1", "action": "create_row", "values": {}}),
                db
            )
            .contains("invalid type")
        );
        assert!(
            refused(
                json!({"version": 1, "action": "delete_row", "values": {}}),
                db
            )
            .contains("unknown variant `delete_row`")
        );
        assert_eq!(
            refused(
                json!({"version": 1, "action": "create_row", "values": {}, "extra": true}),
                db
            ),
            "unknown field `extra`: create_row takes action, version and values"
        );
        assert_eq!(
            refused(
                json!({"version": 1, "action": "append", "markdown": "x", "extra": true}),
                TargetKind::Page
            ),
            "unknown field `extra`: append takes action, version and markdown"
        );
        assert_eq!(
            refused(
                json!({"version": 1, "action": "update_row", "unique_id": "1", "x": 1}),
                db
            ),
            "unknown field `x`: update_row takes action, version, unique_id and values"
        );
        assert!(refused(json!({"version": 1, "action": "create_row"}), db).contains("values"));
        assert!(refused(json!({"version": 1, "values": {}}), db).contains("action"));
        assert_eq!(
            refused(
                json!({"version": 1, "action": "update_row", "unique_id": "NW-1", "values": {}}),
                db
            ),
            "`values` names at least one property to change"
        );
        assert_eq!(
            refused(
                json!({"version": 1, "action": "update_row", "unique_id": " ", "values": {"a": 1}}),
                db
            ),
            "`unique_id` names a row, such as NW-42"
        );
        assert_eq!(
            refused(
                json!({"version": 1, "action": "append", "markdown": "  \n"}),
                TargetKind::Page
            ),
            "`markdown` holds nothing to add"
        );
        assert!(
            refused(
                json!({"version": 1, "action": "append", "markdown": "x"}),
                db
            )
            .contains("takes create_row and update_row")
        );
        assert_eq!(
            refused(
                json!({"version": 1, "action": "append", "markdown": "> ".repeat(MAX_DEPTH + 1)}),
                TargetKind::Page
            ),
            "`markdown` nests blocks more than 32 levels deep"
        );
        assert!(
            read_json(
                json!({"version": 1, "action": "append", "markdown": format!("{}x", "> ".repeat(MAX_DEPTH))}),
                TargetKind::Page
            )
            .is_ok()
        );
        assert!(
            refused(
                json!({"version": 1, "action": "create_row", "values": {}}),
                TargetKind::Page
            )
            .contains("takes append")
        );
    }

    #[test]
    fn target_kinds_round_trip_through_their_text() {
        for kind in [TargetKind::Database, TargetKind::Page] {
            assert_eq!(TargetKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(TargetKind::parse("row"), None);
    }
}
