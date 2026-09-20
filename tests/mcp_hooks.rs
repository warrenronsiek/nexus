// @feature coordination
// @feature persistence
// @feature runtime
// @feature analyst
// @spec docs/features/coordination.md
// @spec docs/features/persistence.md
// @spec docs/features/runtime.md
// @spec docs/features/analyst.md
// @boundary dynamic-json
use nexus::coordination::domain::{ClaimState, RecordScope};
use nexus::persistence::Store;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct Harness {
    _temp: TempDir,
    root: PathBuf,
    database: PathBuf,
    daemon: Child,
    mcp: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl Harness {
    fn start() -> Self {
        Self::start_with_analyst(None)
    }

    fn start_with_analyst(analyst_command: Option<&std::path::Path>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let state = temp.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let database = state.join("nexus.db");
        let socket = state.join("nexus.sock");
        let lock = state.join("nexus.lock");
        let config = temp.path().join("config.toml");
        let analyst_command = analyst_command
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "claude".into());
        std::fs::write(
            &config,
            format!(
                r#"
schema_version = 1

[coordination]
reconcile_seconds = 1

[storage]
database_path = '{}'

[runtime]
socket_path = '{}'
lock_path = '{}'

[analyst]
enabled = {}
provider = "claude"

[analyst.claude]
command = '{}'
"#,
                database.display(),
                socket.display(),
                lock.display(),
                analyst_command != "claude",
                analyst_command
            ),
        )
        .unwrap();

        let executable = env!("CARGO_BIN_EXE_nexus");
        let daemon = Command::new(executable)
            .args(["--config", config.to_str().unwrap(), "daemon"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !socket.exists() {
            assert!(Instant::now() < deadline, "daemon socket did not appear");
            std::thread::sleep(Duration::from_millis(20));
        }

        let mut mcp = Command::new(executable)
            .args(["--config", config.to_str().unwrap(), "mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = mcp.stdin.take().unwrap();
        let output = BufReader::new(mcp.stdout.take().unwrap());
        Self {
            _temp: temp,
            root,
            database,
            daemon,
            mcp,
            input,
            output,
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        response
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":arguments}))["result"]
            ["structuredContent"]
            .clone()
    }

    fn hook(
        &mut self,
        name: &str,
        session: &str,
        tool_id: &str,
        tool_name: &str,
        tool_input: Value,
    ) -> Value {
        let root = self.root.to_string_lossy().to_string();
        self.call(
            name,
            json!({
                "session_id":session,
                "project_root":root,
                "agent":if session.starts_with("codex") { "codex" } else { "claude" },
                "tool_use_id":tool_id,
                "tool_name":tool_name,
                "tool_input":tool_input
            }),
        )
    }

    fn store(&self) -> Store {
        Store::open(&self.database).unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.mcp.kill();
        let _ = self.mcp.wait();
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
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
    run_git(&harness.root, &["init"]);
    run_git(
        &harness.root,
        &["config", "user.email", "nexus@example.test"],
    );
    run_git(&harness.root, &["config", "user.name", "Nexus Test"]);
    std::fs::write(harness.root.join("tracked.txt"), "initial\n").unwrap();
    run_git(&harness.root, &["add", "tracked.txt"]);
    run_git(&harness.root, &["commit", "-m", "initial"]);

    let root = harness.root.to_string_lossy().to_string();
    let prompt = harness.call(
        "nexus_user_prompt",
        json!({"session_id":"observer","project_root":root,"agent":"codex","prompt":"Edit the tracked file"}),
    );
    assert_eq!(prompt["recorded"], true);
    std::fs::write(harness.root.join("tracked.txt"), "changed outside hooks\n").unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut store = harness.store();
        let observed = store
            .list_claims(None, RecordScope::All)
            .unwrap()
            .into_iter()
            .any(|claim| {
                claim.session_id == "observer"
                    && claim.tool_use_id.starts_with("git-reconcile:")
                    && claim.path.ends_with("tracked.txt")
            });
        if observed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Git reconciliation did not create an advisory claim"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn configured_analyst_is_called_through_mcp_and_recorded() {
    let analyst_temp = tempfile::tempdir().unwrap();
    let script = analyst_temp.path().join("fake-claude");
    std::fs::write(
        &script,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"result\":\"Use a handoff or accept the overlap.\"}'\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();

    let mut harness = Harness::start_with_analyst(Some(&script));
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

fn run_git(root: &std::path::Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
}
