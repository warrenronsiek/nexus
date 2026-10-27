// @feature runtime
// @spec docs/features/runtime.md
// @boundary native-hook-event-json
mod support;

use nexus::agents::integration::{hook_configuration, Host};
use serde_json::{json, Value};
use std::path::Path;
use support::{Harness, HOST_HOOK_BUDGET};

#[test]
fn generated_completion_hooks_record_large_results_without_forwarding_output() {
    for host in [Host::Codex, Host::Claude] {
        let mut harness = Harness::start();
        let event = json!({
            "session_id":format!("{}-image-result", host.name()),
            "cwd":harness.root,
            "turn_id":"turn", "prompt_id":"turn", "model":"fixture-model",
            "tool_use_id":"image-call", "tool_name":"view_image",
            "tool_input":{"path":"frame.png"},
            "tool_response":[{"type":"image","mimeType":"image/png",
                "data":"private-image-fixture".repeat(100_000)}]
        });
        let configuration = hook_configuration(host, Path::new("/opt/nexus"), None);
        let hook = &configuration["hooks"]["PostToolUse"][0]["hooks"][0];
        let arguments = render_hook_input(&hook["input"], &event);
        let params = json!({"name":hook["tool"],"arguments":arguments});
        assert!(
            serde_json::to_vec(&params).unwrap().len() < 1024 * 1024,
            "a large tool result must not overflow the completion hook transport"
        );
        let expected_id = harness.send("tools/call", params);
        let (mut harness, response) = harness.next_response_within(HOST_HOOK_BUDGET);
        assert_eq!(response["id"], expected_id);
        assert_eq!(response["result"]["structuredContent"]["permitted"], true);
        assert_eq!(response["result"]["structuredContent"]["recorded"], true);
        let events = harness.call("nexus_events", json!({"limit":16}));
        let completion = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["kind"] == "tool_succeeded")
            .unwrap();
        assert_eq!(completion["payload"]["tool_name"], "view_image");
        assert!(!completion.to_string().contains("private-image-fixture"));
        assert_eq!(harness.status()["ok"], true);
    }
}

/// Native MCP hooks substitute whole event fields while retaining literal configuration values.
fn render_hook_input(template: &Value, event: &Value) -> Value {
    Value::Object(
        template
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| {
                let field = value
                    .as_str()
                    .and_then(|text| text.strip_prefix("${"))
                    .and_then(|text| text.strip_suffix('}'));
                (
                    key.clone(),
                    field.map_or_else(|| value.clone(), |field| event[field].clone()),
                )
            })
            .collect(),
    )
}

#[test]
fn generated_work_hooks_cover_supported_response_and_child_sources() {
    for host in [Host::Codex, Host::Claude] {
        let configuration = hook_configuration(host, Path::new("/opt/nexus"), None);
        for event in ["SessionStart", "Stop", "SubagentStart", "SubagentStop"] {
            let groups = configuration["hooks"][event]
                .as_array()
                .unwrap_or_else(|| panic!("missing {host:?} {event}"));
            assert!(groups
                .iter()
                .flat_map(|group| group["hooks"].as_array().unwrap())
                .any(|hook| hook["command"]
                    .as_str()
                    .is_some_and(|command| command.contains("work host-event"))));
        }
        let terminal = if host == Host::Codex {
            "Interrupt"
        } else {
            "StopFailure"
        };
        assert!(configuration["hooks"][terminal].is_array());
    }
    let claude = hook_configuration(Host::Claude, Path::new("/opt/nexus"), None);
    assert!(
        claude["hooks"]["Interrupt"].is_null(),
        "do not install unsupported events"
    );
}
