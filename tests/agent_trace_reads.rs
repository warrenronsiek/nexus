// @feature runtime
// @feature agent-traces
// @spec docs/features/runtime.md
// @spec docs/features/agent-traces.md
// @boundary trace-read-json
mod support;

use serde_json::{json, Value};
use support::Harness;

#[test]
fn application_trace_reads_are_available_to_cli_and_mcp() {
    let mut harness = Harness::start();
    let output = harness.feature_cli(
        "traces",
        &[
            "list",
            "--application",
            "support",
            "--environment",
            "production",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cli: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(cli["ok"], true, "{cli}");
    assert_eq!(cli["traces"], json!([]), "{cli}");

    let definitions = harness.request("tools/list", json!({}));
    let tool = definitions["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "nexus_traces")
        .expect("trace tool is discoverable");
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["inputSchema"]["type"], "object");
    let mcp = harness.call(
        "nexus_traces",
        json!({
            "action":"list",
            "filter":{"application_id":"support","environment":"production"}
        }),
    );
    assert_eq!(mcp, cli);

    let rejected = harness.request(
        "tools/call",
        json!({
            "name":"nexus_traces",
            "arguments":{"action":"append","events":[]}
        }),
    );
    assert_eq!(rejected["error"]["code"], -32602, "{rejected}");
}
