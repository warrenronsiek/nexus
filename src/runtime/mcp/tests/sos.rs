// @feature runtime
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/sos.md
use super::*;

#[test]
fn advertises_one_typed_sos_tool_with_arbitrary_metadata_only_at_the_boundary() {
    let definitions = tool_definitions();
    let tools = definitions
        .as_array()
        .unwrap()
        .iter()
        .filter(|definition| definition["name"] == "nexus_sos")
        .collect::<Vec<_>>();
    assert_eq!(tools.len(), 1);

    let schema = &tools[0]["inputSchema"];
    let branches = schema["oneOf"].as_array().unwrap();
    assert_eq!(branches.len(), 6);
    for branch in branches {
        assert_eq!(branch["type"], "object");
        assert_eq!(branch["additionalProperties"], false);
        assert!(branch["required"]
            .as_array()
            .unwrap()
            .contains(&json!("action")));
        for caller_field in ["project_root", "session_id", "agent", "turn_id"] {
            assert!(branch["properties"].get(caller_field).is_none());
        }
    }
    assert!(schema_accepts(schema, &json!({"action":"list"})));
    assert!(schema_accepts(
        schema,
        &json!({"action":"get","issue_id":"sos-1"})
    ));
    assert!(schema_accepts(
        schema,
        &json!({"action":"dump","scope":"all"})
    ));
    assert!(schema_accepts(
        schema,
        &json!({
            "action":"report",
            "message":"Release pipeline is wedged",
            "type":"build_failure",
            "environment":"ci-linux",
            "version":"1.7.0",
            "model":"gpt-6",
            "trace":"step=publish",
            "metadata":{"job":"release","attempt":3}
        })
    ));
    assert!(schema_accepts(
        schema,
        &json!({"action":"resolve","issue_id":"sos-1","resolution":"Rolled back"})
    ));
    assert!(schema_accepts(
        schema,
        &json!({"action":"deprecate","issue_id":"sos-1","reason":"Unsupported version"})
    ));
    for rejected in [
        json!({"action":"report","message":"missing context"}),
        json!({
            "action":"report",
            "message":"bad metadata",
            "type":"failure",
            "environment":"ci",
            "version":"1",
            "model":"gpt-6",
            "trace":"trace",
            "metadata":["not", "an", "object"]
        }),
        json!({"action":"get"}),
        json!({"action":"list","unexpected":true}),
        json!({"action":"resolve","issue_id":"sos-1"}),
        json!({"action":"unknown"}),
    ] {
        assert!(!schema_accepts(schema, &rejected), "accepted {rejected}");
    }
}

#[test]
fn arguments_replace_caller_supplied_provenance_with_process_context() {
    let context = McpInvocationContext {
        session_id: "mcp-session-test".into(),
        project_root: Some("/tmp/project".into()),
        agent: "codex".into(),
    };
    let mut report = json!({
        "action":"report",
        "message":"Release pipeline is wedged",
        "type":"build_failure",
        "environment":"ci",
        "version":"1.7.0",
        "model":"gpt-6",
        "trace":"trace",
        "metadata":{},
        "project_root":"/tmp/spoofed",
        "session_id":"spoofed",
        "agent":"spoofed",
        "turn_id":"spoofed"
    });

    enrich_sos_arguments(&mut report, &context);

    assert_eq!(report["project_root"], "/tmp/project");
    assert_eq!(report["session_id"], "mcp-session-test");
    assert_eq!(report["agent"], "codex");
    assert!(report.get("turn_id").is_none());
    assert_eq!(report["model"], "gpt-6", "report model is source data");
}

#[test]
fn resource_maps_to_an_unbounded_current_project_dump() {
    let context = McpInvocationContext {
        session_id: "mcp-session-test".into(),
        project_root: Some("/tmp/project".into()),
        agent: "codex".into(),
    };
    let (method, arguments) = resource_request("nexus://sos", &context).unwrap();
    assert_eq!(method, "sos");
    assert_eq!(arguments["action"], "dump");
    assert_eq!(arguments["scope"], "project");
    assert_eq!(arguments["project_root"], "/tmp/project");
    assert_eq!(arguments["session_id"], "mcp-session-test");
}
