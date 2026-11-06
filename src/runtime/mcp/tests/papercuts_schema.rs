// @feature runtime
// @feature papercuts
// @spec docs/features/runtime.md
// @spec docs/features/papercuts.md
use super::*;

#[test]
fn papercuts_list_schema_advertises_a_closed_typed_cursor() {
    let definitions = tool_definitions();
    let papercuts = definitions
        .as_array()
        .unwrap()
        .iter()
        .find(|definition| definition["name"] == "nexus_papercuts")
        .unwrap();
    let schema = &papercuts["inputSchema"];
    let list = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["properties"]["action"]["const"] == "list")
        .unwrap();

    assert_eq!(
        list["properties"]["after"],
        json!({
            "type":"object",
            "required":["upvote_count","updated_at","id"],
            "properties":{
                "upvote_count":{"type":"integer","minimum":0},
                "updated_at":{"type":"string","format":"date-time"},
                "id":{"type":"string"}
            },
            "additionalProperties":false
        })
    );
    assert!(schema_accepts(
        schema,
        &json!({
            "action":"list",
            "after":{"upvote_count":2,"updated_at":"2026-10-21T12:00:00Z","id":"cut-1"}
        })
    ));
}

#[test]
fn papercuts_get_schema_requires_a_papercut_id() {
    let definitions = tool_definitions();
    let schema = &definitions
        .as_array()
        .unwrap()
        .iter()
        .find(|definition| definition["name"] == "nexus_papercuts")
        .unwrap()["inputSchema"];
    assert!(schema_accepts(
        schema,
        &json!({"action":"get","papercut_id":"cut-1"})
    ));
    assert!(!schema_accepts(schema, &json!({"action":"get"})));
}
