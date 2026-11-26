// @feature runtime
// @feature agent-traces
// @spec docs/features/runtime.md
// @spec docs/features/agent-traces.md
// @boundary trace-read-json
mod support;

use serde_json::{json, Value};
use support::Harness;

#[test]
fn trace_query_accepts_only_one_tagged_version_selector() {
    for selector in [
        json!({"kind":"exact","value":"3"}),
        json!({"kind":"unversioned"}),
    ] {
        let query = json!({"query":{"action":"calls","filter":{"version":selector}}});
        let decoded = nexus::coordination::api::ServiceRequest::decode("traces", query).unwrap();
        let (_, encoded) = decoded.wire_parts().unwrap();
        assert_eq!(encoded["query"]["filter"]["version"], selector);
    }

    for invalid in [
        json!({"version":"3","unversioned":true}),
        json!({"version":{"kind":"exact"}}),
        json!({"version":{"kind":"unversioned","value":"3"}}),
        json!({"version":{"kind":"exact","value":""}}),
    ] {
        let query = json!({"query":{"action":"calls","filter":invalid}});
        assert!(nexus::coordination::api::ServiceRequest::decode("traces", query).is_err());
    }

    let omitted = json!({"query":{"action":"calls"}});
    let decoded = nexus::coordination::api::ServiceRequest::decode("traces", omitted).unwrap();
    let (_, encoded) = decoded.wire_parts().unwrap();
    assert!(encoded["query"]["filter"].get("version").is_none());
}

#[test]
fn trace_schema_distinguishes_producer_versions_from_read_selectors() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["traces", "schema"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let schema: Value = serde_json::from_slice(&output.stdout).unwrap();
    let producer_version = &schema["batch"]["$defs"]["TraceEvent"]["properties"]["version"];
    assert_eq!(producer_version["type"], json!(["string", "null"]));
    let selectors = schema["query"]["$defs"]["TraceVersionSelector"]["oneOf"]
        .as_array()
        .expect("query selector exposes tagged variants");
    assert!(selectors.iter().any(|variant| {
        variant["properties"]["kind"]["const"] == "exact"
            && variant["properties"]["value"]["type"] == "string"
    }));
    assert!(selectors.iter().any(|variant| {
        variant["properties"]["kind"]["const"] == "unversioned"
            && variant["additionalProperties"] == false
            && variant["properties"].get("value").is_none()
    }));
}

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
