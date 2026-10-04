// @feature coordination
// @feature persistence
// @feature runtime
// @feature analyst
// @feature usage-analytics
// @spec docs/features/coordination.md
// @spec docs/features/persistence.md
// @spec docs/features/runtime.md
// @spec docs/features/analyst.md
// @spec docs/features/usage-analytics.md
// @boundary dynamic-json
mod support;

use nexus::coordination::domain::{ClaimState, ConflictScope, RecordScope};
use serde_json::json;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use support::{wait_for_path, write_executable, Harness, HOST_HOOK_BUDGET};

#[test]
fn post_tool_use_returns_hook_compatible_text_and_structured_coordination_data() {
    let mut harness = Harness::start();
    let root = harness.root.to_string_lossy().to_string();
    let arguments = json!({
        "session_id":"codex-hook-output",
        "project_root":root,
        "agent":"codex",
        "tool_use_id":"tool-output-1",
        "tool_name":"Write",
        "tool_input":{"file_path":"src/lib.rs"}
    });
    harness.call("nexus_pre_tool_use", arguments.clone());

    let response = harness.request(
        "tools/call",
        json!({"name":"nexus_post_tool_use","arguments":arguments}),
    );
    let result = &response["result"];

    assert_eq!(result["content"][0]["text"], "{}");
    assert_eq!(result["structuredContent"]["permitted"], true);
    assert_eq!(result["structuredContent"]["recorded"], true);
}

#[test]
fn lifecycle_usage_is_deduplicated_and_includes_derived_scripts_and_skills() {
    let mut harness = Harness::start();
    let script = harness.root.join("tools/report.py");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, "print('report')\n").unwrap();
    let root = harness.root.to_string_lossy().to_string();
    let tool = json!({
        "session_id":"pi-usage",
        "project_root":root,
        "agent":"pi",
        "turn_id":"turn-7",
        "model":"test-model",
        "tool_use_id":"tool-usage-1",
        "tool_name":"exec_command",
        "tool_input":{"cmd":"python3 ./tools/report.py --token secret"}
    });

    harness.call("nexus_pre_tool_use", tool.clone());
    harness.call("nexus_post_tool_use", tool.clone());
    harness.call("nexus_post_tool_use", tool);
    let skill = json!({
        "session_id":"pi-usage",
        "project_root":root,
        "agent":"pi",
        "turn_id":"turn-7",
        "model":"test-model",
        "invocation_id":"skill-usage-1",
        "skill_name":"tdd",
        "evidence":"explicit_invocation",
        "actor":"user"
    });
    harness.call("nexus_skill_use", skill.clone());
    harness.call("nexus_skill_use", skill);

    let usage = harness.call("nexus_usage", json!({}));
    assert_eq!(usage["ok"], true, "{usage}");
    assert!(usage["window_started_at"].is_string());
    assert!(usage["window_ended_at"].is_string());
    assert_usage_count(&usage, "tools", "tool", "shell", 1);
    assert_usage_count(&usage, "tools", "script", "tools/report.py", 1);
    let skills = usage["skills"].as_array().unwrap();
    assert_eq!(skills.len(), 1, "{usage}");
    assert_eq!(skills[0]["name"], "tdd");
    assert_eq!(skills[0]["count"], 1);
    assert_eq!(skills[0]["sessions"], 1);
    assert_eq!(skills[0]["evidence"]["explicit_invocation"], 1);

    let database = std::fs::read(&harness.database).unwrap();
    assert_bytes_absent(&database, b"--token secret");
}

fn assert_usage_count(usage: &serde_json::Value, list: &str, kind: &str, name: &str, count: u64) {
    let found = usage[list].as_array().unwrap().iter().any(|item| {
        item["kind"] == kind && item["name"] == name && item["count"].as_u64() == Some(count)
    });
    assert!(found, "missing {kind} {name} count {count}: {usage}");
}

fn assert_bytes_absent(haystack: &[u8], needle: &[u8]) {
    assert!(
        !haystack
            .windows(needle.len())
            .any(|window| window == needle),
        "private tool arguments leaked into the analytics database"
    );
}

#[test]
fn successful_skill_manifest_reads_are_inferred_once_per_turn() {
    let mut harness = Harness::start();
    let manifest = harness.root.join(".codex/skills/code-architect/SKILL.md");
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(
        &manifest,
        "---\nname: code-architect\ndescription: Review architecture.\n---\n",
    )
    .unwrap();
    let root = harness.root.to_string_lossy().to_string();
    harness.call(
        "nexus_post_tool_use",
        json!({
            "session_id":"codex-skill-read",
            "project_root":root,
            "agent":"codex",
            "turn_id":"turn-skill",
            "tool_use_id":"read-skill-1",
            "tool_name":"read_file",
            "tool_input":{"path":manifest},
            "tool_output":{"content":"skill loaded"}
        }),
    );
    harness.call(
        "nexus_post_tool_use",
        json!({
            "session_id":"codex-skill-read",
            "project_root":root,
            "agent":"codex",
            "turn_id":"turn-skill",
            "tool_use_id":"read-skill-2",
            "tool_name":"exec_command",
            "tool_input":{"cmd":format!("sed -n '1,200p' {}", manifest.display())},
            "tool_output":{"content":"skill loaded"}
        }),
    );

    let usage = harness.call("nexus_usage", json!({}));
    let skill = usage["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "code-architect")
        .expect("skill manifest read was not observed");
    assert_eq!(skill["count"], 1, "{usage}");
    assert_eq!(skill["evidence"]["instruction_read"], 1, "{usage}");
}

#[test]
fn nexus_exec_preserves_child_exit_and_records_the_script_once() {
    let mut harness = Harness::start();
    let script = harness.root.join("tools/failing-check");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    write_executable(
        &script,
        "#!/bin/sh\nprintf 'wrapped-output'\nprintf 'wrapped-error' >&2\nexit 7\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args([
            "--config",
            harness._temp.path().join("config.toml").to_str().unwrap(),
            "exec",
            "--",
            script.to_str().unwrap(),
            "--token",
            "secret",
        ])
        .current_dir(&harness.root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "wrapped-output");
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "wrapped-error");

    let usage = harness.call("nexus_usage", json!({}));
    let scripts = usage["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "script" && item["name"] == "tools/failing-check")
        .collect::<Vec<_>>();
    assert_eq!(scripts.len(), 1, "{usage}");
    assert_eq!(scripts[0]["failed"], 1, "{usage}");
}

#[test]
fn session_end_command_hook_releases_claims_without_hook_output() {
    let mut harness = Harness::start();
    let root = harness.root.to_string_lossy().to_string();
    harness.hook(
        "nexus_pre_tool_use",
        "codex-session-end",
        "tool-session-end",
        "Write",
        json!({"file_path":"src/lib.rs"}),
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args([
            "--config",
            harness._temp.path().join("config.toml").to_str().unwrap(),
        ])
        .args(["hook-session-end", "--agent", "codex"])
        .current_dir(&harness.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        json!({"session_id":"codex-session-end","cwd":root})
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let mut store = harness.store();
    let claims = store.list_claims(None, RecordScope::All).unwrap();
    assert!(claims.iter().any(|claim| {
        claim.session_id == "codex-session-end" && claim.state == ClaimState::Released
    }));
}

#[test]
fn mcp_lifecycle_classifies_conflicts_and_persists_projections() {
    let mut harness = Harness::start();

    let initialized = harness.request("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"integration-test","version":"1"}}));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "nexus");
    let tools = harness.request("tools/list", json!({}));
    assert!(tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "nexus_pre_tool_use"));

    let first = harness.hook(
        "nexus_pre_tool_use",
        "codex-1",
        "tool-1",
        "Edit",
        json!({"file_path":"src/lib.rs","line_start":10,"line_end":20}),
    );
    assert_eq!(first["permitted"], true);
    assert_eq!(first["recorded"], true);
    assert!(first["advisories"].as_array().unwrap().is_empty());

    let overlap = harness.hook(
        "nexus_pre_tool_use",
        "claude-1",
        "tool-2",
        "Edit",
        json!({"file_path":"src/lib.rs","line_start":15,"line_end":18}),
    );
    assert_eq!(
        overlap["permitted"], true,
        "even critical overlap must never block"
    );
    assert_eq!(overlap["advisories"][0]["severity"], "critical");
    assert_eq!(overlap["advisories"][0]["kind"], "hunk_overlap");
    assert_eq!(overlap["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert!(overlap["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("informational only"));
    assert!(overlap.get("decision").is_none());
    assert!(overlap.get("permissionDecision").is_none());

    let distinct = harness.hook(
        "nexus_pre_tool_use",
        "claude-2",
        "tool-3",
        "Edit",
        json!({"file_path":"src/lib.rs","line_start":50,"line_end":60}),
    );
    assert_eq!(distinct["permitted"], true);
    assert_eq!(distinct["advisories"][0]["severity"], "info");
    assert_eq!(
        distinct["advisories"][0]["kind"],
        "same_file_distinct_hunks"
    );

    let unknown_range = harness.hook(
        "nexus_pre_tool_use",
        "claude-3",
        "tool-4",
        "Write",
        json!({"file_path":"src/lib.rs","content":"replacement"}),
    );
    assert_eq!(unknown_range["permitted"], true);
    assert_eq!(unknown_range["advisories"][0]["severity"], "warning");
    assert_eq!(unknown_range["advisories"][0]["kind"], "path_overlap");

    let destructive = harness.hook(
        "nexus_pre_tool_use",
        "claude-4",
        "tool-5",
        "Bash",
        json!({"command":"rm -f src/lib.rs"}),
    );
    assert_eq!(
        destructive["permitted"], true,
        "destructive overlap is advisory too"
    );
    assert_eq!(destructive["advisories"][0]["severity"], "critical");
    assert_eq!(destructive["advisories"][0]["kind"], "destructive_overlap");

    let post = harness.hook(
        "nexus_post_tool_use",
        "claude-1",
        "tool-2",
        "Edit",
        json!({"file_path":"src/lib.rs"}),
    );
    assert_eq!(post["permitted"], true);

    let open_conflicts = harness.call("nexus_conflicts", json!({"scope":"open"}));
    let conflict_id = open_conflicts["conflicts"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let resolved = harness.call(
        "nexus_resolve",
        json!({"conflict_id":conflict_id,"resolution":"accept_overlap"}),
    );
    assert_eq!(resolved["ok"], true);
    assert_eq!(resolved["status"], "resolved");

    let mut store = harness.store();
    let counts = store.counts().unwrap();
    assert_eq!(counts.active_sessions, 5);
    assert_eq!(counts.active_claims, 5);
    assert!(counts.open_conflicts >= 4);
    let events = store.list_events(None, 1_000).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "advisory_ignored")
            .count(),
        1
    );
    let claims = store.list_claims(None, RecordScope::All).unwrap();
    assert_eq!(
        claims
            .iter()
            .filter(|claim| {
                claim.session_id == "claude-1"
                    && claim.tool_use_id == "tool-2"
                    && claim.state == ClaimState::Modified
            })
            .count(),
        1
    );
}

#[test]
fn apply_patch_shell_unknown_and_release_paths_behave_advisorially() {
    let mut harness = Harness::start();
    let patch_a = "*** Begin Patch\n*** Update File: src/main.rs\n@@ -4,2 +10,5 @@\n-old\n+new\n*** End Patch";
    let first = harness.hook(
        "nexus_pre_tool_use",
        "codex-patch",
        "patch-1",
        "apply_patch",
        json!({"patch":patch_a}),
    );
    assert!(first["advisories"].as_array().unwrap().is_empty());

    let patch_b =
        "*** Begin Patch\n*** Update File: src/main.rs\n@@ -12,2 +12,2 @@\n-a\n+b\n*** End Patch";
    let second = harness.hook(
        "nexus_pre_tool_use",
        "claude-patch",
        "patch-2",
        "apply_patch",
        json!({"patch":patch_b}),
    );
    assert_eq!(second["permitted"], true);
    assert_eq!(second["advisories"][0]["kind"], "hunk_overlap");

    let unknown = harness.hook(
        "nexus_pre_tool_use",
        "codex-unknown",
        "unknown-1",
        "web_search",
        json!({"query":"safe"}),
    );
    assert_eq!(unknown["permitted"], true);
    assert!(unknown["advisories"].as_array().unwrap().is_empty());

    let shell_write = harness.hook(
        "nexus_pre_tool_use",
        "codex-shell",
        "shell-1",
        "exec_command",
        json!({"cmd":"touch notes.txt"}),
    );
    assert_eq!(shell_write["recorded"], true);

    let root = harness.root.to_string_lossy().to_string();
    let stopped = harness.call(
        "nexus_session_stop",
        json!({"session_id":"codex-patch","project_root":root,"agent":"codex"}),
    );
    assert_eq!(stopped["permitted"], true);

    let after_release = harness.hook(
        "nexus_pre_tool_use",
        "fresh",
        "patch-3",
        "Edit",
        json!({"file_path":"src/main.rs","line_start":10,"line_end":14}),
    );
    // claude-patch remains active, so one conflict remains; the stopped Codex claim is absent.
    assert_eq!(after_release["advisories"].as_array().unwrap().len(), 1);
    assert_eq!(
        after_release["advisories"][0]["other_session_id"],
        "claude-patch"
    );

    let mut store = harness.store();
    let claims = store.list_claims(None, RecordScope::All).unwrap();
    assert_eq!(
        claims
            .iter()
            .filter(|claim| {
                claim.session_id == "codex-patch" && claim.state == ClaimState::Released
            })
            .count(),
        1
    );
    assert_eq!(
        claims
            .iter()
            .filter(|claim| claim.path.ends_with("notes.txt"))
            .count(),
        1
    );
    assert_eq!(
        claims
            .iter()
            .filter(|claim| claim.session_id == "codex-unknown")
            .count(),
        0
    );
}

#[test]
fn malformed_hook_and_unavailable_daemon_never_deny_execution() {
    let mut harness = Harness::start();
    let malformed = harness.call("nexus_pre_tool_use", json!({"broken":true}));
    assert_eq!(malformed["permitted"], true);
    assert_eq!(malformed["recorded"], false);
    assert!(malformed["diagnostic"]
        .as_str()
        .unwrap()
        .contains("could not record"));

    harness.daemon.kill().unwrap();
    harness.daemon.wait().unwrap();
    // Make auto-restart fail by removing the config file's state directory after the MCP
    // process has loaded it and replacing it with an ordinary file.
    let state = harness.database.parent().unwrap().to_path_buf();
    std::fs::remove_file(state.join("nexus.sock")).ok();
    std::fs::remove_dir_all(&state).unwrap();
    std::fs::write(&state, "not a directory").unwrap();

    let unavailable = harness.call(
        "nexus_pre_tool_use",
        json!({
            "session_id":"s","project_root":harness.root,"agent":"codex",
            "tool_use_id":"t","tool_name":"Write","tool_input":{"file_path":"a.rs"}
        }),
    );
    assert_eq!(unavailable["permitted"], true);
    assert_eq!(unavailable["recorded"], false);
    assert!(unavailable["diagnostic"]
        .as_str()
        .unwrap()
        .contains("unavailable"));
}

#[test]
fn git_reconciliation_observes_edits_that_bypass_hooks() {
    let mut harness = Harness::start();
    harness.initialize_git();
    harness.observe_dirty_file("observer");
}

#[test]
fn git_reconciliation_stops_refreshing_an_inactive_session() {
    let mut harness = Harness::start_with_claim_ttl(1);
    harness.initialize_git();
    harness.observe_dirty_file("inactive-observer");

    std::thread::sleep(Duration::from_millis(2_200));
    let mut store = harness.store();
    assert_eq!(store.counts().unwrap().active_claims, 0);
    let event_count = store.counts().unwrap().events;
    drop(store);

    std::thread::sleep(Duration::from_millis(1_200));
    assert_eq!(harness.store().counts().unwrap().events, event_count);
}

#[test]
fn git_reconciliation_does_not_repeat_unchanged_observations() {
    let mut harness = Harness::start();
    harness.initialize_git();
    harness.observe_dirty_file("steady-observer");

    std::thread::sleep(Duration::from_millis(100));
    let event_count = harness.store().counts().unwrap().events;
    std::thread::sleep(Duration::from_millis(1_200));
    assert_eq!(harness.store().counts().unwrap().events, event_count);
}

#[test]
fn configured_analyst_is_called_through_mcp_and_recorded() {
    let analyst_temp = tempfile::tempdir().unwrap();
    let script = analyst_temp.path().join("fake-claude");
    write_executable(
        &script,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"result\":\"Use a handoff or accept the overlap.\"}'\n",
    );

    let mut harness = Harness::start_with_analyst(Some(&script), 120);
    harness.hook(
        "nexus_pre_tool_use",
        "codex-analysis",
        "analysis-1",
        "Write",
        json!({"file_path":"src/shared.rs"}),
    );
    let overlap = harness.hook(
        "nexus_pre_tool_use",
        "claude-analysis",
        "analysis-2",
        "Write",
        json!({"file_path":"src/shared.rs"}),
    );
    let conflict_id = overlap["advisories"][0]["conflict_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let analyzed = harness.call("nexus_analyze", json!({"conflict_id":conflict_id}));
    assert_eq!(analyzed["ok"], true);
    assert_eq!(analyzed["analysis"]["provider"], "claude");
    assert_eq!(
        analyzed["analysis"]["text"],
        "Use a handoff or accept the overlap."
    );
    let mut store = harness.store();
    assert_eq!(
        store
            .list_events(None, 1_000)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "conflict_analyzed")
            .count(),
        1
    );
}

#[test]
fn lifecycle_hook_fails_open_within_the_host_budget_when_the_daemon_hangs() {
    let mut harness = Harness::start();
    harness.daemon.kill().unwrap();
    harness.daemon.wait().unwrap();
    let socket = harness.database.with_file_name("nexus.sock");
    std::fs::remove_file(&socket).ok();
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::thread::spawn(move || {
        let mut unanswered = Vec::new();
        for connection in listener.incoming() {
            unanswered.push(connection);
        }
    });

    let arguments = harness.post_tool_arguments("codex-hung-daemon", "t1");
    let started = Instant::now();
    let id = harness.send(
        "tools/call",
        json!({"name":"nexus_post_tool_use","arguments":arguments}),
    );
    let (_harness, response) = harness.next_response_within(HOST_HOOK_BUDGET);

    assert_eq!(response["id"], id);
    assert!(started.elapsed() < HOST_HOOK_BUDGET);
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(response["result"]["content"][0]["text"], "{}");
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["permitted"], true);
    assert_eq!(structured["recorded"], false);
    assert!(structured["diagnostic"]
        .as_str()
        .unwrap()
        .contains("did not respond"));
}

#[test]
fn slow_explicit_tool_does_not_delay_lifecycle_hooks_on_the_same_connection() {
    let analyst_temp = tempfile::tempdir().unwrap();
    let script = analyst_temp.path().join("slow-claude");
    let started = std::path::PathBuf::from(format!("{}.started", script.display()));
    write_executable(
        &script,
        "#!/bin/sh\ncat >/dev/null\n: > \"${0}.started\"\nsleep 6\nprintf '%s' '{\"result\":\"late\"}'\n",
    );
    let mut harness = Harness::start_with_analyst(Some(&script), 120);
    for (session, tool_id) in [("codex-slow-a", "slow-1"), ("claude-slow-b", "slow-2")] {
        harness.hook(
            "nexus_pre_tool_use",
            session,
            tool_id,
            "Write",
            json!({"file_path":"src/shared.rs"}),
        );
    }
    let conflict_id = harness
        .store()
        .list_conflicts(None, ConflictScope::Open)
        .unwrap()[0]
        .id
        .clone();

    harness.send(
        "tools/call",
        json!({"name":"nexus_analyze","arguments":{"conflict_id":conflict_id}}),
    );
    wait_for_path(&started, &mut harness.daemon);
    let arguments = harness.post_tool_arguments("codex-slow-a", "after-analyze");
    let hook_id = harness.send(
        "tools/call",
        json!({"name":"nexus_post_tool_use","arguments":arguments}),
    );
    let (_harness, response) = harness.next_response_within(HOST_HOOK_BUDGET);

    assert_eq!(response["id"], hook_id, "{response}");
    assert_eq!(response["result"]["structuredContent"]["recorded"], true);
}
