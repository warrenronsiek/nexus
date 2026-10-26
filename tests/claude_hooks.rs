// @feature coordination
// @feature runtime
// @spec docs/features/coordination.md
// @spec docs/features/runtime.md
// @boundary host-hook-json
mod support;
use serde_json::json;
use support::Harness;

#[test]
fn claude_json_string_tool_inputs_still_deliver_overlap_context() {
    let mut harness = Harness::start();
    let payload = json!({"file_path":"shared.txt","content":"fixture"});
    harness.hook(
        "nexus_pre_tool_use",
        "peer",
        "first",
        "Write",
        payload.clone(),
    );
    // Claude's native MCP hook templates stringify object-valued event fields.
    let overlap = harness.hook(
        "nexus_pre_tool_use",
        "claude",
        "second",
        "Write",
        json!(payload.to_string()),
    );
    assert_eq!(
        overlap["advisories"].as_array().unwrap().len(),
        1,
        "{overlap}"
    );
    assert!(overlap["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("peer"));
    assert_eq!(overlap["permitted"], true);
}
