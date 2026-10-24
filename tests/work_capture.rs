// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
mod support;
use serde_json::json;
use std::process::Command;
use support::Harness;

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
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\n'{}' --config '{}' work git-hook post-commit >/dev/null 2>&1 || true\n",
            env!("CARGO_BIN_EXE_nexus"),
            harness._temp.path().join("config.toml").display()
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
        "{detail}"
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
