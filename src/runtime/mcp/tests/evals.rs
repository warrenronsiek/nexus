// @feature runtime
// @feature evals
// @spec docs/features/runtime.md
// @spec docs/features/evals.md
use super::*;

#[test]
fn eval_history_tool_exposes_recording_and_queries_without_execution() {
    let definitions = tool_definitions();
    let tool = definitions
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "nexus_evals")
        .expect("eval history must be discoverable by callers and the TUI");
    let schema = &tool["inputSchema"];
    for input in [
        json!({"action":"list"}),
        json!({"action":"dashboard"}),
        json!({"action":"dashboard","group_id":"chosen-group"}),
        json!({"action":"get","id":"run-1"}),
        json!({"action":"why","id":"case-1"}),
    ] {
        assert!(schema_accepts(schema, &input), "rejects {input}");
    }
    for input in [
        json!({"action":"run"}),
        json!({"action":"grade"}),
        json!({"action":"get"}),
        json!({"action":"list","agent":"spoofed"}),
    ] {
        assert!(!schema_accepts(schema, &input), "accepts {input}");
    }
}
