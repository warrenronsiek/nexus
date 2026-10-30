// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary fixture-host-json: exact native identities exercise the installed command adapter without provider traffic.
use super::{cli, Harness};
use serde_json::json;
use std::process::Command;

#[test]
fn claude_session_start_exports_exact_identity_for_later_shell_commands() {
    let harness = Harness::start();
    let env_file = harness.root.join("host.env");
    std::fs::write(&env_file, "export KEEP_ME='preserved'\n").unwrap();
    let input = harness.root.join("start.json");
    let identity = "native'$(touch injected)";
    std::fs::write(
        &input,
        json!({"hook_event_name":"SessionStart","session_id":identity,"cwd":harness.root})
            .to_string(),
    )
    .unwrap();
    let output = cli(&harness)
        .args(["work", "host-event", "--host", "claude"])
        .arg(&input)
        .env("CLAUDE_ENV_FILE", &env_file)
        .env_remove("NEXUS_HOST_SESSION_ID")
        .env_remove("NEXUS_AGENT")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["environment_handoff"], "exported");
    let shell = Command::new("bash").args(["-c", "source \"$1\"; printf '%s\\n' \"$KEEP_ME\" \"$NEXUS_AGENT\" \"$NEXUS_HOST_SESSION_ID\"", "verify"]).arg(&env_file)
        .env_remove("NEXUS_AGENT").env_remove("NEXUS_HOST_SESSION_ID").current_dir(&harness.root).output().unwrap();
    assert!(shell.status.success());
    assert_eq!(
        String::from_utf8(shell.stdout).unwrap(),
        format!("preserved\nclaude\n{identity}\n")
    );
    assert!(!harness.root.join("injected").exists());
}

#[test]
fn claude_environment_export_refuses_symlinks_without_disabling_capture() {
    let mut harness = Harness::start();
    let target = harness.root.join("untouched.env");
    std::fs::write(&target, "original\n").unwrap();
    let link = harness.root.join("host.env");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let input = harness.root.join("start.json");
    std::fs::write(
        &input,
        json!({"hook_event_name":"SessionStart","session_id":"native","cwd":harness.root})
            .to_string(),
    )
    .unwrap();
    let output = cli(&harness)
        .args(["work", "host-event", "--host", "claude"])
        .arg(&input)
        .env("CLAUDE_ENV_FILE", &link)
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["environment_handoff"], "unavailable");
    assert_eq!(std::fs::read_to_string(target).unwrap(), "original\n");
    let roots = harness.call(
        "nexus_work",
        json!({"action":"dashboard","session_origin":"host"}),
    );
    assert_eq!(roots["sessions"].as_array().unwrap().len(), 1);
}
