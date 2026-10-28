// @feature code-concerns
// @feature runtime
// @spec docs/features/code-concerns.md
// @spec docs/features/runtime.md
use super::*;

#[test]
fn code_concerns_has_its_own_tool_and_bound_identity() {
    let definitions = tool_definitions();
    let names = definitions.as_array().unwrap();
    for name in ["nexus_papercuts", "nexus_sos", "nexus_concerns"] {
        assert!(
            names.iter().any(|tool| tool["name"] == name),
            "missing {name}"
        );
    }
    let tool = names
        .iter()
        .find(|tool| tool["name"] == "nexus_concerns")
        .unwrap();
    let schema = &tool["inputSchema"];
    for input in [
        json!({"action":"list"}),
        json!({"action":"dashboard"}),
        json!({"action":"get","concern_id":"concern-one"}),
        json!({"action":"history","concern_id":"concern-one"}),
    ] {
        assert!(schema_accepts(schema, &input), "rejects {input}");
    }
    for input in [
        json!({"action":"list","agent":"spoofed"}),
        json!({"action":"run"}),
        json!({"action":"get"}),
    ] {
        assert!(!schema_accepts(schema, &input), "accepts {input}");
    }
    let bound = bind_tool_context(
        "concerns",
        json!({"action":"list"}),
        &McpInvocationContext {
            session_id: "bound-session".into(),
            project_root: Some("/test/project".into()),
            agent: "test-agent".into(),
        },
    );
    assert_eq!(bound["context"]["session_id"], "bound-session");
    assert_eq!(bound["context"]["agent"], "test-agent");
    assert_eq!(bound["command"]["action"], "list");
}
