//! The tools' table: each one's routes of the API, their schemas and annotations, and how
//! their arguments are read.

use serde_json::json;
use utoipa::openapi::PathItem;

use super::*;
use crate::http::pages::PageAccess;
use crate::http::pages::PageKind;

/// Each tool's routes of the API, which do what it does with the same tokens: method and
/// path below `/api/v1`.
const REST: [(&str, &[(&str, &str)]); 17] = [
    ("whoami", &[("get", "/whoami")]),
    ("list_members", &[("get", "/workspaces/{id}/members")]),
    ("list_teamspaces", &[("get", "/workspaces/{id}/teamspaces")]),
    (
        "get_page_tree",
        &[
            ("get", "/teamspaces/{id}/pages"),
            ("get", "/workspaces/{id}/private-pages"),
        ],
    ),
    ("read_page", &[("get", "/pages/{id}/markdown")]),
    ("create_page", &[("post", "/workspaces/{id}/pages")]),
    ("edit_page", &[("patch", "/pages/{id}/markdown")]),
    ("move_page", &[("post", "/pages/{id}/move")]),
    ("trash_page", &[("post", "/pages/{id}/trash")]),
    ("restore_page", &[("post", "/pages/{id}/restore")]),
    ("get_database", &[("get", "/databases/{id}")]),
    ("query_database", &[("post", "/databases/{id}/query")]),
    ("create_database", &[("post", "/workspaces/{id}/databases")]),
    ("add_property", &[("post", "/databases/{id}/properties")]),
    ("create_row", &[("post", "/databases/{id}/rows")]),
    (
        "update_row",
        &[
            ("patch", "/rows/{id}"),
            ("patch", "/databases/{id}/rows/{unique_id}"),
        ],
    ),
    (
        "claim_task",
        &[
            ("post", "/rows/{id}/claim"),
            ("post", "/databases/{id}/rows/{unique_id}/claim"),
        ],
    ),
];

/// Each resource template's routes of the API, likewise.
const RESOURCES: [(&str, &[(&str, &str)]); 4] = [
    ("neon://page/{id}", &[("get", "/pages/{id}/markdown")]),
    ("neon://database/{id}", &[("get", "/databases/{id}")]),
    (
        "neon://database/{id}/view/{view}",
        &[("post", "/databases/{id}/query")],
    ),
    ("neon://me/tasks", &[("get", "/workspaces/{id}/tasks")]),
];

fn has(item: &PathItem, method: &str) -> bool {
    match method {
        "get" => item.get.is_some(),
        "post" => item.post.is_some(),
        "patch" => item.patch.is_some(),
        "put" => item.put.is_some(),
        "delete" => item.delete.is_some(),
        _ => false,
    }
}

#[test]
fn every_tool_and_resource_has_routes_of_the_api_and_they_are_there() {
    let names: Vec<&str> = TOOLS.iter().map(|spec| spec.name).collect();
    let mapped: Vec<&str> = REST.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, mapped, "each tool, in order, has its routes");
    let templates: Vec<String> = super::super::resources::templates()
        .into_iter()
        .map(|template| template.uri_template)
        .collect();
    let mapped: Vec<&str> = RESOURCES.iter().map(|(uri, _)| *uri).collect();
    assert_eq!(templates, mapped, "each resource template has its routes");

    let spec = crate::http::spec();
    for (name, routes) in REST.iter().chain(RESOURCES.iter()) {
        assert!(!routes.is_empty(), "{name} has no route");
        for (method, path) in *routes {
            let path = format!("/api/v1{path}");
            let item = spec
                .paths
                .paths
                .get(&path)
                .unwrap_or_else(|| panic!("{name}: no route {path}"));
            assert!(has(item, method), "{name}: no {method} {path}");
        }
    }
}

#[test]
fn tools_publish_their_schemas_and_what_they_change() {
    let tools = list();
    assert_eq!(tools.len(), TOOLS.len());
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), TOOLS.len(), "tool names are unique");
    for tool in &tools {
        let schema = &tool.input_schema;
        assert_eq!(schema.get("type"), Some(&json!("object")), "{}", tool.name);
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&json!(false)),
            "{} refuses unknown arguments",
            tool.name
        );
        let annotations = tool.annotations.as_ref().expect("annotations");
        let spec = find(&tool.name).expect("found");
        assert_eq!(
            annotations.read_only_hint,
            Some(!spec.writes),
            "{}",
            tool.name
        );
        assert!(tool.description.as_ref().is_some_and(|d| d.len() > 40));
    }
    // What each tool that writes may do to what is there: only add, or also overwrite or
    // remove.
    let destructive: Vec<(&str, bool)> = TOOLS
        .iter()
        .filter(|spec| spec.writes)
        .map(|spec| {
            let tool = spec.tool();
            let hint = tool
                .annotations
                .and_then(|annotations| annotations.destructive_hint);
            (spec.name, hint.expect("a destructive hint"))
        })
        .collect();
    assert_eq!(
        destructive,
        [
            ("create_page", false),
            ("edit_page", true),
            ("move_page", true),
            ("trash_page", true),
            ("restore_page", false),
            ("create_database", false),
            ("add_property", false),
            ("create_row", false),
            ("update_row", true),
            ("claim_task", true),
        ]
    );
    let required = |name: &str| -> Vec<String> {
        let tool = find(name).expect("a tool").tool();
        tool.input_schema
            .get("required")
            .and_then(|required| required.as_array())
            .map(|required| {
                required
                    .iter()
                    .filter_map(|name| name.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(required("read_page"), ["page_id"]);
    assert_eq!(required("edit_page"), ["page_id", "operations"]);
    assert_eq!(required("update_row"), ["values"]);
    assert!(required("whoami").is_empty());
    assert!(required("claim_task").is_empty());
    // The edit operations are told apart by `op`.
    let edit = find("edit_page").expect("a tool").tool();
    let text = serde_json::to_string(&*edit.input_schema).expect("serializes");
    for op in [
        "append",
        "insert_after",
        "replace_range",
        "find_replace",
        "replace_body",
    ] {
        assert!(text.contains(&format!("\"{op}\"")), "{op}: {text}");
    }
}

fn object(value: Json) -> JsonObject {
    value.as_object().cloned().expect("an object")
}

#[test]
fn arguments_are_read_as_their_schema_has_them() {
    let page = Uuid::from_u128(7);
    let read: PageArguments = arguments(object(json!({"page_id": page}))).expect("valid");
    assert_eq!(read.page_id, page);
    for (wrong, says) in [
        (json!({}), "missing field `page_id`"),
        (json!({"page_id": "not a uuid"}), "UUID"),
        (
            json!({"page_id": page, "extra": 1}),
            "unknown field `extra`",
        ),
        (json!({"page_id": 3}), "invalid type"),
    ] {
        let error = arguments::<PageArguments>(object(wrong)).expect_err("refused");
        assert_eq!(error.code, "invalid_arguments");
        assert!(error.expected);
        assert!(error.message.contains(says), "{}", error.message);
    }
    assert!(arguments::<NoArguments>(object(json!({}))).is_ok());
    assert!(arguments::<NoArguments>(object(json!({"x": 1}))).is_err());

    let edit: EditArguments = arguments(object(json!({
        "page_id": page,
        "operations": [
            {"op": "append", "markdown": "a"},
            {"op": "insert_after", "block_id": "b1", "markdown": "b"},
            {"op": "replace_range", "from": "b1", "markdown": ""},
            {"op": "find_replace", "find": "x", "replace": "y", "all": true},
            {"op": "replace_body", "markdown": "c"},
        ],
    })))
    .expect("valid");
    let edits: Vec<Edit> = edit
        .operations
        .into_iter()
        .map(Edit::try_from)
        .collect::<Result<_, _>>()
        .expect("markdown that nests nothing");
    assert_eq!(
        edits.iter().map(Edit::name).collect::<Vec<_>>(),
        [
            "append",
            "insert_after",
            "replace_range",
            "find_replace",
            "replace_body"
        ]
    );
    assert!(matches!(
        &edits[2],
        Edit::ReplaceRange { from, to, blocks } if from == "b1" && to == "b1" && blocks.is_empty()
    ));
    for wrong in [
        json!({"page_id": page, "operations": [{"op": "shout"}]}),
        json!({"page_id": page, "operations": [{"op": "append"}]}),
        json!({"page_id": page, "operations": [{"op": "append", "markdown": "a", "x": 1}]}),
        json!({"page_id": page, "operations": {"op": "append", "markdown": "a"}}),
    ] {
        assert!(arguments::<EditArguments>(object(wrong)).is_err());
    }

    let query: QueryArguments = arguments(object(json!({
        "database_id": page,
        "filter": {"and": [
            {"property": "Status", "operator": "is", "value": "Done"},
            {"or": [{"property": "Owner", "operator": "contains", "value": "me"}]},
        ]},
        "sorts": [{"property": "Due", "direction": "ascending"}],
        "limit": 10,
    })))
    .expect("valid");
    assert!(query.filter.is_some());
}

#[test]
fn arguments_that_do_not_read_name_the_field_and_what_it_takes() {
    let page = Uuid::from_u128(7);
    let refused = |given: Json| {
        arguments::<QueryArguments>(object(given))
            .expect_err("refused")
            .message
    };
    let message = arguments::<TreeArguments>(object(json!({"teamspace_id": "general"})))
        .expect_err("refused")
        .message;
    assert!(message.contains("`teamspace_id`: UUID"), "{message}");
    for (filter, says) in [
        (
            json!({"property": "Status", "operator": "is_sort_of"}),
            "unknown operator `is_sort_of`: the operators are is, is_not,",
        ),
        (
            json!({"property": "Status"}),
            "a condition names its `operator`, one of is, is_not,",
        ),
        (
            json!([{"property": "Status", "operator": "is"}]),
            "a filter is a condition",
        ),
        (
            json!({"and": [{"property": "Status", "operator": "is"}, {"operator": "is"}]}),
            "and[1]: a condition names its `property`",
        ),
        (
            json!({"and": [], "or": []}),
            "a group holds \"and\" alone, and no field `or`",
        ),
        (
            json!({"property": "Status", "operator": "is", "values": 1}),
            "unknown field `values`",
        ),
    ] {
        let message = refused(json!({"database_id": page, "filter": filter}));
        assert!(message.contains("`filter`"), "{message}");
        assert!(message.contains(says), "{message}");
    }
}

#[test]
fn errors_are_tool_errors_the_caller_reads() {
    let error = ToolError::invalid("not_found", "no such page");
    let result = error.into_result();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.structured_content,
        Some(json!({"error": {"code": "not_found", "message": "no such page"}}))
    );
    let error = ToolError::from(ApiError::internal());
    assert!(!error.expected);
    assert_eq!(error.message, "internal error");
}

fn page(id: u128, parent: Option<u128>, child_count: i64) -> TreePage {
    TreePage {
        id: Uuid::from_u128(id),
        kind: PageKind::Page,
        parent_id: parent.map(Uuid::from_u128),
        teamspace_id: None,
        owner_id: None,
        title: format!("p{id}"),
        icon: None,
        child_count,
        access: PageAccess::Edit,
    }
}

#[test]
fn trees_nest_to_their_depth() {
    // 1 > 2 > 3 > 4, and 5 at the top; 6 under a page left out of the listing.
    let pages = vec![
        page(1, None, 1),
        page(2, Some(1), 1),
        page(3, Some(2), 1),
        page(4, Some(3), 0),
        page(5, None, 0),
        page(6, Some(99), 0),
    ];
    let shown = serde_json::to_value(nest(pages, 2)).expect("serializes");
    let ids = |nodes: &Json| -> Vec<String> {
        nodes
            .as_array()
            .expect("a list")
            .iter()
            .map(|node| node["title"].as_str().expect("a title").to_owned())
            .collect()
    };
    assert_eq!(ids(&shown), ["p1", "p5", "p6"]);
    assert_eq!(ids(&shown[0]["children"]), ["p2"]);
    // Past the depth, no children, but their count.
    assert_eq!(shown[0]["children"][0].get("children"), None);
    assert_eq!(shown[0]["children"][0]["child_count"], 1);
    assert_eq!(shown[1]["children"], json!([]));
}

#[test]
fn a_rate_limit_tells_the_tool_s_caller_when_to_try_again() {
    let limited = ApiError::new(
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "too many edits of pages' content: slow down",
    )
    .with_retry_after(std::time::Duration::from_millis(1500));

    let result = ToolError::from(limited).into_result();
    let error = &result.structured_content.expect("structured content")["error"];
    assert_eq!(error["code"], "rate_limited");
    assert_eq!(error["retry_after"], 2);
    assert_eq!(
        error["message"],
        "too many edits of pages' content: slow down: try again in 2s"
    );

    let refused = ToolError::from(ApiError::forbidden("no"));
    let error = refused.into_result().structured_content.expect("content");
    assert_eq!(error["error"].get("retry_after"), None);
}
