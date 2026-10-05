// @feature agent-memory
// @feature runtime
// @feature coordination
// @spec docs/features/agent-memory.md
// @spec docs/features/runtime.md
// @spec docs/features/coordination.md
// @boundary dynamic-json
#[path = "memory_runtime/database.rs"]
mod database;
mod support;

use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use support::{run_git, wait_for_path, write_executable, Harness};

#[test]
fn first_prompt_injects_memory_once_without_launching_a_provider() {
    let provider_temp = tempfile::tempdir().unwrap();
    let marker = provider_temp.path().join("provider-started");
    let provider = provider_temp.path().join("must-not-run");
    write_executable(
        &provider,
        &format!(
            "#!/bin/sh\n: > '{}'\ncat >/dev/null\nexit 9\n",
            marker.display()
        ),
    );
    let mut harness = Harness::start_with_memory_analysts(&provider, &provider, 3_600, 10);
    for content in [
        "Prefer deterministic process validation",
        "Preserve typed service boundaries",
    ] {
        let added = harness.call(
            "nexus_memory_add",
            json!({"scope":"global","content":content}),
        );
        assert_eq!(added["ok"], true, "{added}");
    }
    let arguments = json!({
        "session_id":"reader-session",
        "project_root":harness.root,
        "agent":"codex",
        "model":"gpt-test",
        "prompt":"Begin the task"
    });

    let first = harness.request(
        "tools/call",
        json!({"name":"nexus_user_prompt","arguments":arguments.clone()}),
    );
    let second = harness.request(
        "tools/call",
        json!({"name":"nexus_user_prompt","arguments":arguments}),
    );

    let first = &first["result"];
    assert_eq!(
        first["structuredContent"]["memory_context"]["item_count"],
        2
    );
    let injected = first["structuredContent"]["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(injected.contains("untrusted historical data"));
    assert!(injected.contains("Prefer deterministic process validation"));
    assert!(second["result"]["structuredContent"]
        .get("memory_context")
        .is_none());
    assert!(
        !marker.exists(),
        "lifecycle request launched a model provider"
    );
}

#[test]
fn slow_background_consolidation_keeps_lifecycle_requests_responsive() {
    let provider_temp = tempfile::tempdir().unwrap();
    let marker = provider_temp.path().join("provider-started");
    let provider = provider_temp.path().join("slow-claude");
    write_executable(
        &provider,
        &format!(
            r#"#!/usr/bin/env python3
import json
import pathlib
import re
import sys
import time
prompt = sys.stdin.read()
pathlib.Path({marker:?}).touch()
time.sleep(3)
job_id = re.search(r'"job_id":"([^"]+)"', prompt).group(1)
print(json.dumps({{"result": json.dumps({{"summaries": [{{"job_id": job_id, "summary": "Background summary"}}]}})}}))
"#,
            marker = marker.to_string_lossy()
        ),
    );
    let fallback = provider_temp.path().join("unused-codex");
    write_executable(&fallback, "#!/bin/sh\ncat >/dev/null\nexit 9\n");
    let mut harness = Harness::start_with_memory_analysts(&provider, &fallback, 1, 10);
    for content in ["First background note", "Second background note"] {
        assert_eq!(
            harness.call(
                "nexus_memory_add",
                json!({"scope":"global","content":content}),
            )["ok"],
            true
        );
    }
    wait_for_path(&marker, &mut harness.daemon);

    let started = Instant::now();
    let hook_id = harness.send(
        "tools/call",
        json!({
            "name":"nexus_user_prompt",
            "arguments":{
                "session_id":"during-consolidation",
                "project_root":harness.root,
                "agent":"codex",
                "prompt":"Keep working"
            }
        }),
    );
    let (_harness, response) = harness.next_response_within(Duration::from_secs(1));

    assert_eq!(response["id"], hook_id, "{response}");
    assert_eq!(response["result"]["structuredContent"]["permitted"], true);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn explicit_consolidation_is_enqueued_without_waiting_for_the_provider_deadline() {
    let provider_temp = tempfile::tempdir().unwrap();
    let marker = provider_temp.path().join("explicit-provider-started");
    let provider = provider_temp.path().join("slow-explicit-claude");
    write_executable(
        &provider,
        &format!(
            r#"#!/usr/bin/env python3
import json
import pathlib
import re
import sys
import time
prompt = sys.stdin.read()
pathlib.Path({marker:?}).touch()
time.sleep(2)
job_id = re.search(r'"job_id":"([^"]+)"', prompt).group(1)
print(json.dumps({{"result": json.dumps({{"summaries": [{{"job_id": job_id, "summary": "Explicit summary"}}]}})}}))
"#,
            marker = marker.to_string_lossy()
        ),
    );
    let fallback = provider_temp.path().join("unused-explicit-codex");
    write_executable(&fallback, "#!/bin/sh\ncat >/dev/null\nexit 9\n");
    let mut harness = Harness::start_with_memory_analysts(&provider, &fallback, 3_600, 120);
    for content in ["First explicit note", "Second explicit note"] {
        assert_eq!(
            harness.call(
                "nexus_memory_add",
                json!({"scope":"global","content":content}),
            )["ok"],
            true
        );
    }

    let started = Instant::now();
    let request_id = harness.send(
        "tools/call",
        json!({"name":"nexus_memory_consolidate","arguments":{}}),
    );
    let (mut harness, response) = harness.next_response_within(Duration::from_secs(1));

    assert_eq!(response["id"], request_id, "{response}");
    let report = &response["result"]["structuredContent"];
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["in_progress"], true, "{report}");
    assert!(started.elapsed() < Duration::from_secs(1));
    wait_for_path(&marker, &mut harness.daemon);
}

#[test]
fn project_memory_is_shared_by_worktrees_and_isolated_between_repositories() {
    let mut harness = Harness::start();
    initialize_repository(&harness.root);
    let sibling = harness._temp.path().join("sibling-worktree");
    run_git(
        &harness.root,
        &[
            "worktree",
            "add",
            "-b",
            "sibling",
            sibling.to_str().unwrap(),
        ],
    );
    let isolated = harness._temp.path().join("isolated-project");
    std::fs::create_dir_all(&isolated).unwrap();
    initialize_repository(&isolated);
    let config = harness._temp.path().join("config.toml");

    let added = memory_cli(
        &config,
        &harness.root,
        &["add", "--scope", "project", "Shared worktree convention"],
    );
    assert_eq!(added["ok"], true, "{added}");
    let shared = memory_cli(
        &config,
        &sibling,
        &["search", "--scope", "project", "worktree"],
    );
    assert_eq!(shared["memories"].as_array().unwrap().len(), 1, "{shared}");
    let separate = memory_cli(
        &config,
        &isolated,
        &["search", "--scope", "project", "worktree"],
    );
    assert!(
        separate["memories"].as_array().unwrap().is_empty(),
        "{separate}"
    );

    // Keep the mutable binding used so this test owns and drops both child processes.
    assert_eq!(harness.status()["status"], "running");
}

fn initialize_repository(root: &Path) {
    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "nexus@example.test"]);
    run_git(root, &["config", "user.name", "Nexus Test"]);
    std::fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "tracked.txt"]);
    run_git(root, &["commit", "-m", "initial"]);
}

fn memory_cli(config: &Path, root: &Path, arguments: &[&str]) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
    command
        .args(["--config", config.to_str().unwrap(), "memory"])
        .args(arguments)
        .current_dir(root);
    clear_git_environment(&mut command);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "memory CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn clear_git_environment(command: &mut Command) {
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(variable);
    }
}
