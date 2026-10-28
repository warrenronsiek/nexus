// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary work-history-json-protocol
mod support;
use serde_json::json;
use std::process::Output;
use support::Harness;

fn cli(harness: &Harness, args: &[&str]) -> Output {
    harness.feature_cli("work", args)
}

fn success(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn work_history_is_discoverable_and_empty_lists_are_readable() {
    let mut harness = Harness::start();
    let page = harness.call("nexus_work", json!({"action":"list"}));
    assert_eq!(page["ok"], true, "{page}");
    assert_eq!(page["items"], json!([]), "{page}");
}

#[test]
fn invalid_observations_have_a_permanent_rejection_code() {
    let mut harness = Harness::start();
    let response = harness.call("nexus_work", json!({"action":"record","observations":[{
        "id":"bad", "subject":{"scope":"activity","id":"a"},
        "occurred_at":"2026-10-24T12:00:00Z", "producer":{"name":"test","version":"1","instance_id":"test"},
        "method":"caller", "body":{"event":"finished","outcome":"failed","elapsed":{"nanoseconds":"-1","method":"caller","clock_id":null}}
    }]}));
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error_code"], "invalid_input", "{response}");
}

#[test]
fn cli_supports_environment_and_occurrence_filters() {
    let harness = Harness::start();
    let page = success(cli(
        &harness,
        &[
            "list",
            "--environment",
            "beta",
            "--occurred-after",
            "2026-10-24T00:00:00Z",
            "--occurred-before",
            "2026-10-25T00:00:00Z",
        ],
    ));
    assert_eq!(page["items"], json!([]));
}

fn seeded_history() -> Harness {
    let source = Harness::start();
    success(cli(&source, &["open", "Router change", "--id", "delivery"]));
    success(cli(
        &source,
        &[
            "start",
            "Implementation",
            "--id",
            "agent",
            "--kind",
            "agent.task",
            "--work-session",
            "delivery",
        ],
    ));
    success(cli(
        &source,
        &[
            "start",
            "Run tests",
            "--id",
            "test-1",
            "--kind",
            "test",
            "--work-session",
            "delivery",
            "--parent",
            "agent",
        ],
    ));
    success(cli(
        &source,
        &[
            "finish",
            "test-1",
            "--outcome",
            "failed",
            "--elapsed-ns",
            "900719925474099312345",
            "--timing-method",
            "caller_reported",
        ],
    ));
    success(cli(
        &source,
        &[
            "start",
            "Run tests",
            "--id",
            "test-2",
            "--kind",
            "test",
            "--work-session",
            "delivery",
            "--parent",
            "agent",
            "--retry-of",
            "test-1",
        ],
    ));
    success(cli(
        &source,
        &["finish", "test-2", "--outcome", "succeeded"],
    ));
    source
}

#[test]
fn cli_records_nested_attempts_and_exact_elapsed() {
    let mut source = seeded_history();
    let children = source.call(
        "nexus_work",
        json!({"action":"timeline","work_session_id":"delivery","parent_activity_id":"agent"}),
    );
    assert_eq!(children["items"].as_array().unwrap().len(), 2, "{children}");
    let detail = source.call(
        "nexus_work",
        json!({"action":"get","subject":{"scope":"activity","id":"test-1"}}),
    );
    assert_eq!(
        detail["record"]["summary"]["elapsed"]["nanoseconds"],
        "900719925474099312345"
    );
    assert_ne!(
        detail["record"]["observations"][0]["provenance"]["agent"],
        "spoofed"
    );
}

#[test]
fn cli_activity_search_keeps_the_matching_attempt_and_tree_identity() {
    let source = seeded_history();
    let page = success(cli(
        &source,
        &["activities", "--kind", "test", "--outcome", "failed"],
    ));
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{page}");
    assert_eq!(items[0]["subject"]["id"], "test-1");
    assert_eq!(items[0]["parent_activity_id"], "agent");
    assert_eq!(items[0]["work_session_ids"], json!(["delivery"]));
}

#[test]
fn cli_transfer_preserves_parentage_exact_values_and_source_receipts() {
    let source = seeded_history();
    let bundle = source.root.join("history.jsonl");
    success(cli(&source, &["export", bundle.to_str().unwrap()]));
    let mut destination = Harness::start();
    success(cli(&destination, &["import", bundle.to_str().unwrap()]));
    success(cli(&destination, &["import", bundle.to_str().unwrap()]));
    let imported = destination.call(
        "nexus_work",
        json!({"action":"timeline","work_session_id":"delivery","parent_activity_id":"agent"}),
    );
    assert_eq!(imported["items"].as_array().unwrap().len(), 2, "{imported}");
    let detail = destination.call(
        "nexus_work",
        json!({"action":"get","subject":{"scope":"activity","id":"test-1"}}),
    );
    assert_eq!(
        detail["record"]["summary"]["elapsed"]["nanoseconds"],
        "900719925474099312345"
    );
    assert!(detail["record"]["observations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event["source"].is_object()));
}
