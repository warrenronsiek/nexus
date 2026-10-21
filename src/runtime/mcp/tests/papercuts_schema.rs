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
            "required":["updated_at","id"],
            "properties":{
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
            "after":{"updated_at":"2026-10-21T12:00:00Z","id":"cut-1"}
        })
    ));
}
