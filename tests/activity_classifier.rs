// @feature usage-analytics
// @feature runtime
// @spec docs/features/usage-analytics.md
// @spec docs/features/runtime.md
// @boundary isolated-provider-fixture
mod support;
use serde_json::json;
use std::time::{Duration, Instant};
use support::executable::write_executable;

#[test]
fn companion_classifies_opaque_work_asynchronously_and_reuses_labels_without_retaining_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let command = temp.path().join("classifier");
    let prompt = temp.path().join("prompt");
    let release = temp.path().join("release");
    let invocations = temp.path().join("invocations");
    write_executable(
        &command,
        &format!(
            r#"#!/bin/sh
cat > '{}'
printf '1\n' >> '{}'
while [ ! -f '{}' ]; do sleep 0.02; done
printf '%s' '{{"category_id":"external.http","name":"HTTP requests","definition":"Interact with HTTP services","description":"Query an HTTP service"}}'
"#,
            prompt.display(),
            invocations.display(),
            release.display()
        ),
    );
    let mut harness = support::start_classifier_harness(&command);
    let result=harness.hook("nexus_post_tool_use","codex-classifier","opaque-one","Bash",json!({"command":"python -c 'import requests; requests.get(\"https://private.example/PRIVATE_CANARY_123456789012345\")'"}));
    assert_eq!(result["recorded"], true, "{result}");
    std::fs::write(&release, "ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let summary = harness.call("nexus_analytics", json!({"action":"summary"}));
        if summary["classified_actions"] == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "classification did not finish: {summary}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let redacted = std::fs::read_to_string(&prompt).unwrap();
    assert!(redacted.contains("requests.get"), "{redacted}");
    assert!(!redacted.contains("private.example"));
    assert!(!redacted.contains("PRIVATE_CANARY"));
    let details = harness.call(
        "nexus_analytics",
        json!({"action":"examples","category_id":"external.http"}),
    );
    assert_eq!(details["examples"][0]["evidence"], "model");
    harness.hook("nexus_post_tool_use","codex-classifier","opaque-two","Bash",json!({"command":"python -c 'import requests; requests.get(\"https://different.example/OTHER_CANARY_123456789012345\")'"}));
    let summary = harness.call("nexus_analytics", json!({"action":"summary"}));
    assert_eq!(summary["classified_actions"], 2, "{summary}");
    assert_eq!(
        std::fs::read_to_string(&invocations)
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(
        !String::from_utf8_lossy(&std::fs::read(&harness.database).unwrap())
            .contains("PRIVATE_CANARY")
    );
}

#[test]
fn analytics_is_available_through_the_mcp_boundary() {
    let mut harness = support::Harness::start();
    let result = harness.call("nexus_analytics", json!({"action":"summary"}));
    assert_eq!(result["ok"], true, "{result}");
}

#[test]
fn analytics_distinguishes_supplied_scripts_from_scripts_created_during_the_session() {
    let mut harness = support::Harness::start();
    std::fs::create_dir_all(harness.root.join("scripts")).unwrap();
    std::fs::write(
        harness.root.join("scripts/supplied.py"),
        "print('supplied')",
    )
    .unwrap();
    for (index, name, origin) in [
        (0, "supplied.py", "repository"),
        (1, "generated.py", "agent_authored"),
    ] {
        if index == 1 {
            std::fs::write(
                harness.root.join("scripts/generated.py"),
                "print('generated')",
            )
            .unwrap();
        }
        let args = json!({"session_id":"script-origin","agent":"codex","project_root":harness.root,"tool_use_id":format!("script-{index}"),"tool_name":"Bash","tool_input":{"command":format!("python scripts/{name}")}});
        harness.call("nexus_pre_tool_use", args.clone());
        harness.call("nexus_post_tool_use", args);
        let catalog = harness.call("nexus_analytics", json!({"action":"catalog"}));
        let resource = catalog["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|resource| resource["name"] == format!("scripts/{name}"))
            .unwrap();
        assert_eq!(resource["origin"], origin, "{catalog}");
    }
}

#[test]
fn opaque_activity_is_not_dropped_when_inventory_classification_is_backlogged() {
    let temp = tempfile::tempdir().unwrap();
    let command = temp.path().join("classifier");
    let started = temp.path().join("started");
    let ordering = temp.path().join("ordering");
    let release = temp.path().join("release");
    write_executable(
        &command,
        &format!(
            r#"#!/bin/sh
input=$(cat)
case "$input" in *requests.get*) printf 'opaque\n' >> '{}' ;; *) printf 'inventory\n' >> '{}' ;; esac
touch '{}'
while [ ! -f '{}' ]; do sleep 0.02; done
printf '%s' '{{"category_id":"external.http","name":"HTTP requests","definition":"Interact with HTTP services","description":"Query an HTTP service"}}'
"#,
            ordering.display(),
            ordering.display(),
            started.display(),
            release.display()
        ),
    );
    let mut harness = support::start_classifier_harness(&command);
    let result=harness.call("nexus_analytics",json!({"action":"inventory","session_id":"codex-priority","agent":"codex","project_root":harness.root,"scope":"host_tools","resources":tool_inventory(24)}));
    assert_eq!(result["ok"], true, "{result}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    harness.hook(
        "nexus_post_tool_use",
        "codex-priority",
        "opaque",
        "Bash",
        json!({"command":"python -c 'import requests; requests.get(endpoint)'"}),
    );
    std::thread::sleep(Duration::from_secs(1));
    std::fs::write(release, "ready").unwrap();
    wait_for_classification(&mut harness);
    let order = std::fs::read_to_string(ordering).unwrap();
    assert_eq!(order.lines().nth(1), Some("opaque"), "{order}");
}
fn tool_inventory(count: usize) -> Vec<serde_json::Value> {
    (0..count).map(|index|json!({"name":format!("tool-{index}"),"kind":"tool","origin":"host_tool","exposure":"exposed","description":"A capability"})).collect()
}
fn wait_for_classification(harness: &mut support::Harness) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let result = harness.call("nexus_analytics", json!({"action":"summary"}));
        if result["classified_actions"] == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "classification did not finish: {result}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
