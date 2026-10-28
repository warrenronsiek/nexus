// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary synthetic-caller-context-json
mod support;
use serde_json::json;
use std::process::Command;
use std::time::{Duration, Instant};
use support::Harness;

#[path = "work_capture/git.rs"]
mod git_capture;

#[test]
fn automatic_shell_capture_supports_the_system_bash_with_unset_checks() {
    let harness = Harness::start();
    let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/work-capture.sh");
    let output = Command::new("/bin/bash")
        .args([
            "-u",
            "-c",
            "source \"$1\"; nexus_work_exec test 'Named check' sh -c 'printf executed; exit 7'",
            "test",
        ])
        .arg(helper)
        .env("NEXUS_WORK_BINARY", env!("CARGO_BIN_EXE_nexus"))
        .env("NEXUS_CONFIG", harness._temp.path().join("config.toml"))
        .env_remove("NEXUS_HOST_SESSION_ID")
        .current_dir(&harness.root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert_eq!(output.stdout, b"executed");
}

#[test]
fn automatic_shell_capture_runs_once_when_nexus_configuration_is_invalid() {
    let temporary = tempfile::tempdir().unwrap();
    let invalid = temporary.path().join("invalid.toml");
    std::fs::write(&invalid, "invalid = [").unwrap();
    let marker = temporary.path().join("executions");
    let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/work-capture.sh");
    let output = Command::new("bash").args(["-c",
        "source \"$1\"; nexus_work_exec test 'Named check' sh -c 'printf x >> \"$1\"; exit 7' sh \"$2\"","test"])
        .arg(helper).arg(&marker).env("NEXUS_WORK_BINARY",env!("CARGO_BIN_EXE_nexus"))
        .env("NEXUS_CONFIG",invalid).env("HOME",temporary.path()).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(marker).unwrap(), b"x");
}

#[test]
fn inline_interpreter_programs_never_become_script_names() {
    let mut harness = Harness::start();
    let marker = "private_inline_program_sentinel";
    let program = format!("print('{marker}')");
    let output = cli(&harness)
        .args(["exec", "--", "python3", "-c", &program])
        .output()
        .unwrap();
    assert!(output.status.success());
    let work = harness.call("nexus_work", json!({"action":"activities","query":{}}));
    let usage = harness.call("nexus_usage", json!({}));
    assert!(
        !work.to_string().contains(marker),
        "inline source retained in work history"
    );
    assert!(
        !usage.to_string().contains(marker),
        "inline source retained in legacy usage"
    );
}

#[test]
fn standalone_capture_opens_a_root_and_retains_a_caller_environment() {
    let mut harness = Harness::start();
    let output = cli(&harness)
        .env("NEXUS_WORK_ENVIRONMENT", "staging")
        .args(["exec", "--name", "Verify staging", "--", "true"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let page = harness.call(
        "nexus_work",
        json!({"action":"list","query":{"environment":"staging"}}),
    );
    let root = page["items"]
        .as_array()
        .unwrap()
        .first()
        .unwrap_or_else(|| panic!("missing work root: {page}"));
    assert_eq!(root["context"]["environment"], "staging");
}

#[test]
fn wrapped_commit_passes_one_attempt_identity_to_post_commit() {
    let mut harness = Harness::start();
    harness.initialize_git();
    let hooks = harness.root.join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    let hook = hooks.join("post-commit");
    let hook_error = hooks.join("post-commit.stderr");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\n'{}' --config '{}' work git-hook post-commit >/dev/null 2>'{}' || true\n",
            env!("CARGO_BIN_EXE_nexus"),
            harness._temp.path().join("config.toml").display(),
            hook_error.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    support::run_git(
        &harness.root,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
    std::fs::write(harness.root.join("tracked.txt"), "changed\n").unwrap();
    support::run_git(&harness.root, &["add", "tracked.txt"]);
    let output = cli(&harness)
        .args([
            "exec",
            "--work-session",
            "delivery",
            "--name",
            "Commit candidate",
            "--",
            "git",
            "commit",
            "-m",
            "change",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    wait_for_work_delivery(&harness);
    let timeline = harness.call(
        "nexus_work",
        json!({"action":"timeline","work_session_id":"delivery"}),
    );
    let attempt = timeline["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "Commit candidate")
        .unwrap();
    let id = attempt["subject"]["id"].as_str().unwrap();
    let detail = harness.call(
        "nexus_work",
        json!({"action":"get","subject":{"scope":"activity","id":id}}),
    );
    let links = detail["record"]["links"].as_array().unwrap();
    assert!(
        links
            .iter()
            .any(|link| link["relation"] == "produced" && link["target"]["kind"] == "revision"),
        "{detail}; post-commit stderr: {}",
        std::fs::read_to_string(hook_error).unwrap_or_default()
    );
}

#[test]
fn exec_retains_exit_and_stdio_while_recording_named_child_timing() {
    let mut harness = Harness::start();
    let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args([
            "--config",
            harness._temp.path().join("config.toml").to_str().unwrap(),
            "exec",
            "--kind",
            "test",
            "--name",
            "failing check",
            "--work-session",
            "flow",
            "--parent",
            "owner",
            "--",
            "sh",
            "-c",
            "printf 'child-output'; printf 'child-error' >&2; exit 7",
        ])
        .current_dir(&harness.root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"child-output");
    assert_eq!(output.stderr, b"child-error");
    let page = harness.call(
        "nexus_work",
        json!({"action":"timeline","work_session_id":"flow","parent_activity_id":"owner"}),
    );
    let activity = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "failing check")
        .unwrap_or_else(|| panic!("named test activity: {page}"));
    assert_eq!(activity["parent_activity_id"], "owner");
    assert_eq!(activity["work_session_ids"], json!(["flow"]));
    assert_eq!(activity["outcome"], "failed");
    assert_eq!(activity["elapsed"]["method"], "rust_instant_elapsed");
    assert!(
        activity["elapsed"]["nanoseconds"]
            .as_str()
            .unwrap()
            .parse::<u128>()
            .unwrap()
            > 0
    );
}

fn cli(harness: &Harness) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
    command
        .arg("--config")
        .arg(harness._temp.path().join("config.toml"))
        .current_dir(&harness.root);
    command
}

#[test]
fn explicit_handoff_reaches_measured_descendants_without_checkout_inference() {
    let mut harness = Harness::start();
    let handoff = nexus::runtime::work_capture::host_handoff(
        "codex",
        "host-source",
        Some("request-source"),
        Some("model-source"),
    );
    let output = cli(&harness)
        .env(
            "NEXUS_WORK_CONTEXT",
            serde_json::to_string(&handoff).unwrap(),
        )
        .env_remove("NEXUS_HOST_SESSION_ID")
        .env_remove("NEXUS_AGENT")
        .env_remove("NEXUS_WORK_SESSION_ID")
        .env_remove("NEXUS_PARENT_ACTIVITY_ID")
        .args([
            "exec",
            "--kind",
            "test",
            "--name",
            "Handoff child",
            "--",
            "sh",
            "-c",
            "printf '%s' \"$NEXUS_WORK_SESSION_ID\"",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        handoff.work_session_id
    );
    let page = harness.call(
        "nexus_work",
        json!({"action":"activities","query":{"host_session_id":"host-source"}}),
    );
    let item = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "Handoff child")
        .unwrap();
    assert_eq!(
        item["parent_activity_id"],
        handoff.parent_activity_id.unwrap()
    );
    assert_eq!(
        item["context"]["labels"]["nexus.accounting.role"],
        "execution"
    );
}

#[test]
fn native_callbacks_measure_across_processes_and_context_resolves_the_active_shared_root() {
    let mut harness = Harness::start();
    let event = |name| json!({"session_id":"native-host","turn_id":"native-turn","hook_event_name":name,"work_session_id":"logical-work","session_title":"Measured native session","cwd":harness.root});
    for name in ["SessionStart", "UserPromptSubmit"] {
        capture_native(&harness, "codex", event(name));
    }
    let output = cli(&harness)
        .args([
            "work",
            "context",
            "--agent",
            "codex",
            "--host-session",
            "native-host",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let context: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(context["work_session_id"], "logical-work");
    assert_eq!(context["turn_id"], "native-turn");
    capture_native(&harness, "codex", event("Stop"));
    let page = harness.call(
        "nexus_work",
        json!({"action":"activities","query":{"host_session_id":"native-host","kind":"request"}}),
    );
    let request = page["items"].as_array().unwrap().first().unwrap();
    assert_eq!(
        request["elapsed"]["method"], "os_monotonic_callback_interval",
        "{request}"
    );
    assert!(
        request["elapsed"]["nanoseconds"]
            .as_str()
            .unwrap()
            .parse::<u128>()
            .unwrap()
            > 0
    );
    assert!(
        !request["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap == "clock_domain_mismatch"),
        "{request}"
    );
}

fn capture_native(harness: &Harness, host: &str, value: serde_json::Value) {
    capture_native_receipt(harness, host, value);
    wait_for_work_delivery(harness);
}

fn capture_native_receipt(
    harness: &Harness,
    host: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    let source = tempfile::NamedTempFile::new_in(harness._temp.path()).unwrap();
    std::fs::write(source.path(), serde_json::to_vec(&value).unwrap()).unwrap();
    let output = cli(harness)
        .args(["work", "host-event", "--host", host])
        .arg(source.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["capture"]["status"], "durable", "{receipt}");
    assert_eq!(receipt["capture"]["lost"], 0, "{receipt}");
    assert_eq!(receipt["capture"]["rejected"], 0, "{receipt}");
    receipt
}

fn wait_for_work_delivery(harness: &Harness) {
    // Native capture acknowledges local durability; projection assertions need delivery.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = cli(harness).args(["work", "flush"]).output().unwrap();
        if output.status.success() {
            let health: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(health["rejected"], 0, "{health}");
            assert_eq!(health["capture"]["lost"], 0, "{health}");
            if health["queued"] == 0 {
                return;
            }
        }
        assert!(Instant::now() < deadline, "pending delivery: {output:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct PausedDaemon(u32);
impl PausedDaemon {
    fn new(id: u32) -> Self {
        assert_eq!(unsafe { libc::kill(id as i32, libc::SIGSTOP) }, 0);
        Self(id)
    }
}
impl Drop for PausedDaemon {
    fn drop(&mut self) {
        let _ = unsafe { libc::kill(self.0 as i32, libc::SIGCONT) };
    }
}

#[test]
fn successive_authoritative_titles_can_return_to_an_earlier_name() {
    let mut harness = Harness::start();
    for title in ["First name", "Second name"] {
        capture_native(
            &harness,
            "codex",
            json!({"session_id":"title-host","hook_event_name":"SessionTitle","session_title":title,"cwd":harness.root}),
        );
    }
    let paused = PausedDaemon::new(harness.daemon.id());
    let receipt = capture_native_receipt(
        &harness,
        "codex",
        json!({"session_id":"title-host","hook_event_name":"SessionTitle","session_title":"First name","cwd":harness.root}),
    );
    assert!(
        receipt["health"]["queued"].as_u64().unwrap() > 0,
        "{receipt}"
    );
    let outbox = harness.database.parent().unwrap().join("work-outbox");
    assert!(std::fs::read_dir(outbox).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.extension().is_some_and(|ext| ext == "json")
            && String::from_utf8(std::fs::read(path).unwrap())
                .unwrap()
                .contains("First name")
    }));
    drop(paused);
    wait_for_work_delivery(&harness);
    let page = harness.call(
        "nexus_work",
        json!({"action":"list","query":{"host_session_id":"title-host"}}),
    );
    assert_eq!(page["items"][0]["title"], "First name", "{page}");
    let events = harness.call(
        "nexus_work",
        json!({"action":"events","subject":page["items"][0]["subject"]}),
    );
    let titles: Vec<_> = events["observations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["body"]["name"] == "Host session title updated")
        .collect();
    assert_eq!(titles.len(), 3, "{events}");
    let ids: std::collections::HashSet<_> = titles.iter().map(|event| &event["id"]).collect();
    assert_eq!(ids.len(), 3, "{events}");
}

#[test]
fn sourced_shell_helper_preserves_the_handoff_host_identity() {
    let harness = Harness::start();
    let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/work-capture.sh");
    let handoff = nexus::runtime::work_capture::host_handoff(
        "codex",
        "actual-host",
        Some("actual-turn"),
        None,
    );
    let output=Command::new("/bin/bash").args(["-u","-c","source \"$1\"; nexus_work_exec test 'Handoff check' sh -c 'printf %s \"$NEXUS_HOST_SESSION_ID\"'","test"])
        .arg(helper).env("NEXUS_WORK_BINARY",env!("CARGO_BIN_EXE_nexus"))
        .env("NEXUS_CONFIG",harness._temp.path().join("config.toml"))
        .env("NEXUS_WORK_CONTEXT",serde_json::to_string(&handoff).unwrap())
        .env_remove("NEXUS_HOST_SESSION_ID").env_remove("NEXUS_AGENT")
        .current_dir(&harness.root).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "actual-host");
}

#[test]
fn measured_commands_qualify_distinct_executor_instances() {
    let mut harness = Harness::start();
    for name in ["First command", "Second command"] {
        let output = cli(&harness)
            .env("NEXUS_AGENT", "same-runner")
            .env("NEXUS_HOST_SESSION_ID", "same-host")
            .args([
                "exec",
                "--work-session",
                "work",
                "--kind",
                "test",
                "--name",
                name,
                "--",
                "true",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let page = harness.call(
        "nexus_work",
        json!({"action":"activities","query":{"host_session_id":"same-host","kind":"test"}}),
    );
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{page}");
    assert_ne!(
        items[0]["executor"], items[1]["executor"],
        "command instances are separate workers: {page}"
    );
    assert!(
        items
            .iter()
            .all(|item| item["parent_activity_id"].is_null()),
        "an unobserved host activity cannot be the inferred command parent: {page}"
    );
}

#[test]
fn command_and_native_callbacks_share_one_canonical_host_root_declaration() {
    let mut harness = Harness::start();
    let output = cli(&harness)
        .env("NEXUS_AGENT", "codex")
        .env("NEXUS_HOST_SESSION_ID", "same-source")
        .env("NEXUS_TURN_ID", "first-turn")
        .args(["exec", "--name", "First command", "--", "true"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let command=nexus::runtime::work_capture::host_event("codex",&serde_json::to_vec(&json!({"session_id":"same-source","hook_event_name":"SessionStart","cwd":harness.root.canonicalize().unwrap()})).unwrap()).unwrap();
    let nexus::work_history::WorkAction::Record { observations } = command.command else {
        panic!("record")
    };
    let expected = observations
        .iter()
        .find(|item| {
            matches!(
                item.body,
                nexus::work_history::WorkEvent::SessionOpened { .. }
            )
        })
        .unwrap();
    let detail = harness.call(
        "nexus_work",
        json!({"action":"get","subject":expected.subject}),
    );
    let actual = detail["record"]["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["body"]["event"] == "session_opened")
        .unwrap();
    assert_eq!(
        actual["body"],
        serde_json::to_value(&expected.body).unwrap()
    );
}

#[test]
fn malformed_advisory_host_callbacks_are_counted_without_blocking_the_host() {
    let harness = Harness::start();
    let source = tempfile::NamedTempFile::new_in(harness._temp.path()).unwrap();
    std::fs::write(source.path(), b"{malformed").unwrap();
    let output = cli(&harness)
        .args(["work", "host-event", "--host", "codex", "--advisory"])
        .arg(source.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    let health = cli(&harness).args(["work", "health"]).output().unwrap();
    let health: serde_json::Value = serde_json::from_slice(&health.stdout).unwrap();
    assert_eq!(health["capture"]["transport_rejected"], 1, "{health}");
}

#[test]
fn an_explicit_model_overrides_the_retained_host_context() {
    let harness = Harness::start();
    capture_native(
        &harness,
        "codex",
        json!({"session_id":"model-host","hook_event_name":"SessionStart","model":"observed-model","session_title":"Model source","cwd":harness.root}),
    );
    let output = cli(&harness)
        .args([
            "work",
            "context",
            "--agent",
            "codex",
            "--host-session",
            "model-host",
            "--model",
            "selected-model",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let handoff: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(handoff["model"], "selected-model");
}
