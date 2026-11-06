// @feature runtime
// @feature work-history
// @feature usage-analytics
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @spec docs/features/usage-analytics.md
// @boundary dynamic-json
mod support;

use serde_json::json;
use std::process::Command;
use support::Harness;

fn cli(harness: &Harness) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
    command
        .arg("--config")
        .arg(harness._temp.path().join("config.toml"))
        .current_dir(&harness.root);
    command
}

fn internal_hook(harness: &Harness, args: &[&str], input: &[u8]) -> std::process::Output {
    use std::io::Write;
    let mut child = cli(harness)
        .env("NEXUS_INTERNAL_PROCESS", "1")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_nothing_recorded(harness: &mut Harness, session_id: &str) {
    let sessions = harness.call("nexus_sessions", json!({"scope":"all"}));
    assert!(!sessions.to_string().contains(session_id), "{sessions}");
    let claims = harness.call("nexus_claims", json!({"scope":"all"}));
    assert!(!claims.to_string().contains("src/lib.rs"), "{claims}");
    let work = harness.call("nexus_work", json!({"action":"list","query":{}}));
    assert_eq!(work["items"], json!([]), "{work}");
    let activities = harness.call("nexus_work", json!({"action":"activities","query":{}}));
    assert_eq!(activities["items"], json!([]), "{activities}");
}

#[test]
fn nexus_internal_provider_lifecycle_hooks_are_permitted_without_recording() {
    let mut harness = Harness::start();
    let root = harness.root.to_string_lossy().to_string();
    let prompt = harness.internal_process_call(
        "nexus_user_prompt",
        json!({"session_id":"internal-reviewer","project_root":root,"agent":"claude","prompt":"Review this diff"}),
    );
    assert_eq!(prompt["content"][0]["text"], "{}", "{prompt}");
    assert_eq!(prompt["structuredContent"]["permitted"], true);
    assert_eq!(prompt["structuredContent"]["recorded"], false);
    let tool = json!({
        "session_id":"internal-reviewer",
        "project_root":root,
        "agent":"claude",
        "tool_use_id":"internal-write",
        "tool_name":"Write",
        "tool_input":{"file_path":"src/lib.rs"}
    });
    for name in ["nexus_pre_tool_use", "nexus_post_tool_use"] {
        let result = harness.internal_process_call(name, tool.clone());
        assert_eq!(result["isError"], false, "{result}");
        assert_eq!(result["structuredContent"]["permitted"], true, "{result}");
        assert_eq!(result["structuredContent"]["recorded"], false, "{result}");
    }
    assert_nothing_recorded(&mut harness, "internal-reviewer");
}

#[test]
fn nexus_internal_provider_hooks_record_nothing_and_exit_successfully() {
    let mut harness = Harness::start();
    let root = harness.root.to_string_lossy().to_string();
    let host_event = ["work", "host-event", "--host", "claude", "--advisory", "-"];
    for event in [
        json!({"session_id":"internal-review","hook_event_name":"SessionStart","cwd":root}),
        json!({"session_id":"internal-review","hook_event_name":"UserPromptSubmit","prompt_id":"turn-1","cwd":root}),
        json!({"session_id":"internal-review","hook_event_name":"Stop","prompt_id":"turn-1","cwd":root}),
    ] {
        let output = internal_hook(&harness, &host_event, &serde_json::to_vec(&event).unwrap());
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
    }
    let malformed = internal_hook(&harness, &host_event, b"{malformed");
    assert!(malformed.status.success(), "{malformed:?}");
    let ended = internal_hook(
        &harness,
        &["hook-session-end", "--agent", "claude"],
        &serde_json::to_vec(&json!({"session_id":"internal-review","cwd":root})).unwrap(),
    );
    assert!(ended.status.success(), "{ended:?}");
    assert!(ended.stdout.is_empty(), "{ended:?}");

    let health = cli(&harness).args(["work", "health"]).output().unwrap();
    let health: serde_json::Value = serde_json::from_slice(&health.stdout).unwrap();
    assert_eq!(health["queued"], 0, "{health}");
    let counters = &health["capture"];
    assert!(
        counters.is_null() || (counters["offered"] == 0 && counters["transport_rejected"] == 0),
        "{health}"
    );
    assert_nothing_recorded(&mut harness, "internal-review");
}
