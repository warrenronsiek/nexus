// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary isolated-nested-command-capture
mod support;

use serde_json::{json, Value};
use std::process::Command;
use support::Harness;

fn nested_command(intended: &Harness, decoy: &Harness) -> Command {
    let private_home = intended._temp.path().join("home");
    std::fs::create_dir_all(&private_home).unwrap();
    let script = intended.root.join("child-script.sh");
    std::fs::write(&script, "#!/bin/sh\nset -eu\n\"$NESTED_TEST_NEXUS_BIN\" exec --name 'Nested inner' -- /usr/bin/true\n").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &private_home)
        .env("XDG_CONFIG_HOME", private_home.join("config"))
        .env("XDG_DATA_HOME", private_home.join("data"))
        .env("NEXUS_STATE_DIR", private_home.join("state"))
        .env("NEXUS_CONFIG", decoy._temp.path().join("config.toml"))
        .env("NEXUS_WORK_OUTBOX_DIR", private_home.join("work-outbox"))
        .env("NEXUS_HOST_SESSION_ID", "nested-host")
        .env("NESTED_TEST_NEXUS_BIN", env!("CARGO_BIN_EXE_nexus"))
        .arg("--config")
        .arg(intended._temp.path().join("config.toml"))
        .args([
            "exec",
            "--name",
            "Nested outer",
            "--work-session",
            "nested-flow",
            "--",
            "/bin/sh",
        ])
        .arg(script)
        .current_dir(&intended.root);
    command
}

fn named<'a>(page: &'a Value, title: &str) -> &'a Value {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == title)
        .unwrap_or_else(|| {
            panic!("{title} was not captured in the explicitly selected private store: {page}")
        })
}

#[test]
fn nested_exec_retains_the_explicit_configuration_and_parent_chain() {
    let mut intended = Harness::start();
    let decoy = Harness::start();
    let opened = intended.call("nexus_work", json!({"action":"record","observations":[{
        "id":"nested-open","subject":{"scope":"session","id":"nested-flow"},
        "occurred_at":"2026-10-24T12:00:00Z",
        "producer":{"name":"fixture","version":"1","instance_id":"nested-test"},
        "method":"caller_reported","body":{"event":"session_opened","title":"Nested config fixture","context":{}}
    }]}));
    assert_eq!(opened["ok"], true, "{opened}");
    let output = nested_command(&intended, &decoy).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let activities = intended.call("nexus_work", json!({"action":"activities","query":{}}));
    let outer = named(&activities, "Nested outer");
    let inner = named(&activities, "Nested inner");
    assert_eq!(
        inner["parent_activity_id"], outer["subject"]["id"],
        "{activities}"
    );
    assert_eq!(
        inner["work_session_ids"],
        json!(["nested-flow"]),
        "{activities}"
    );
    assert_eq!(outer["outcome"], "succeeded", "{activities}");
    assert_eq!(inner["outcome"], "succeeded", "{activities}");
}
