// @feature coordination
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
use super::*;
use serde_json::json;

#[test]
fn query_scope_is_an_enum_not_a_boolean_switch() {
    let request =
        ServiceRequest::decode("claims", json!({"project_id":"project-1","scope":"all"})).unwrap();
    let ServiceRequest::Claims(query) = request else {
        panic!("expected claims query");
    };
    assert_eq!(query.scope(), RecordScope::All);

    let error = ServiceRequest::decode("claims", json!({"active_only":false})).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn open_ended_tool_json_is_confined_to_the_tool_payload() {
    let request = ServiceRequest::decode(
        "pre_tool_use",
        json!({
            "session_id":"session-1",
            "tool_use_id":"tool-1",
            "tool_name":"future_tool",
            "tool_input":{"future":{"shape":[1,2,3]}}
        }),
    )
    .unwrap();
    let ServiceRequest::ToolHook { input, .. } = request else {
        panic!("expected tool hook");
    };
    assert_eq!(input.tool_input.as_json()["future"]["shape"][2], 3);
}

#[test]
fn memory_add_requires_an_explicit_scope_and_round_trips_over_the_daemon_wire() {
    let request = ServiceRequest::decode(
        "memory_add",
        json!({
            "scope":"project",
            "content":"Prefer typed boundaries",
            "session_id":"session-1",
            "project_root":"/tmp/project",
            "agent":"codex",
            "model":"gpt-test"
        }),
    )
    .unwrap();
    let ServiceRequest::MemoryAdd(command) = &request else {
        panic!("expected memory add command");
    };
    assert_eq!(command.scope, MemoryScopeArg::Project);
    assert_eq!(command.content, "Prefer typed boundaries");
    assert_eq!(command.context.session_id, "session-1");

    let (method, params) = request.wire_parts().unwrap();
    assert_eq!(method, "memory_add");
    assert_eq!(params["scope"], "project");

    let error = ServiceRequest::decode(
        "memory_add",
        json!({
            "content":"Ambiguous scope",
            "session_id":"session-1"
        }),
    )
    .unwrap_err();
    assert!(error.to_string().contains("scope"));
}

#[test]
fn memory_search_limit_matches_the_persistence_bound() {
    let request = ServiceRequest::decode(
        "memory_search",
        json!({"scope":"global","regex":"typed","limit":9999}),
    )
    .unwrap();
    let ServiceRequest::MemorySearch(query) = request else {
        panic!("expected memory search");
    };
    assert_eq!(query.bounded_limit(), 256);
}
