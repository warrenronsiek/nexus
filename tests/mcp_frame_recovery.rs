// @feature runtime
// @spec docs/features/runtime.md
// @boundary dynamic-json
mod support;

use serde_json::json;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use support::{Harness, HOST_HOOK_BUDGET};
use wait_timeout::ChildExt;

#[test]
fn unterminated_oversized_frame_exits_without_waiting_for_stdin_to_close() {
    let temporary = tempfile::tempdir().unwrap();
    let config = temporary.path().join("config.toml");
    std::fs::write(&config, "schema_version = 1\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .arg("--config")
        .arg(config)
        .arg("mcp")
        .current_dir(temporary.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(&vec![b'x'; 1024 * 1024 + 2]).unwrap();
    input.flush().unwrap();

    let status = child.wait_timeout(Duration::from_secs(5)).unwrap();
    if status.is_none() {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    assert!(status.is_some(), "MCP shutdown must not wait for stdin EOF");
    assert!(!status.unwrap().success());
    drop(input);
}

#[test]
fn oversized_hook_does_not_disconnect_later_hooks() {
    let mut harness = Harness::start();
    let mut oversized = harness.post_tool_arguments("codex-large-output", "large-output");
    oversized["tool_output"] = json!("x".repeat(2 * 1024 * 1024));
    harness.send(
        "tools/call",
        json!({"name":"nexus_post_tool_use","arguments":oversized}),
    );
    let arguments = harness.post_tool_arguments("codex-large-output", "after-large-output");
    let expected_id = harness.send(
        "tools/call",
        json!({"name":"nexus_post_tool_use","arguments":arguments}),
    );
    let (mut harness, response) = harness.next_response_within(HOST_HOOK_BUDGET);
    assert_eq!(response["id"], expected_id);
    assert_eq!(response["result"]["structuredContent"]["permitted"], true);
    assert_eq!(response["result"]["structuredContent"]["recorded"], true);
    assert!(harness.status()["ok"].as_bool().unwrap());
}
