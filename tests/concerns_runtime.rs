// @feature code-concerns
// @feature runtime
// @spec docs/features/code-concerns.md
// @spec docs/features/runtime.md
// @boundary concern-cli-json-protocol: exercise the public command and MCP adapters
mod support;
use serde_json::{json, Value};
use std::process::{Command, Output};
use support::Harness;

fn cli(harness: &Harness, args: &[&str]) -> Output {
    harness.feature_cli("concerns", args)
}

fn success(output: Output) -> Value {
    support::assert_success("code concerns CLI", &output);
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn code_concerns_is_a_dedicated_discoverable_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["concerns", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout).unwrap();
    for operation in [
        "report", "upvote", "list", "get", "history", "areas", "resolve",
    ] {
        assert!(help.contains(operation), "missing {operation}: {help}");
    }
}

#[test]
fn cli_and_mcp_share_concern_history_without_polluting_other_registries() {
    let mut harness = Harness::start();
    let area = r#"{"area":{"repository":"repo","kind":"subsystem","key":"router","display_name":"Router"},"relation":"affected"}"#;
    let report = success(cli(
        &harness,
        &[
            "report",
            "Fallback loses context",
            "--description",
            "Router drops state on fallback",
            "--id",
            "router-issue",
            "--event-id",
            "first",
            "--area-json",
            area,
            "--work-session",
            "delivery",
            "--occurred-at",
            "2026-10-27T12:00:00Z",
        ],
    ));
    assert_eq!(report["inserted"], 1, "{report}");
    success(cli(
        &harness,
        &[
            "remedy",
            "router-issue",
            "--what",
            "Preserve dispatch state",
            "--why",
            "Fallback resets it",
            "--outcome",
            "Needs verification",
            "--event-id",
            "remedy-one",
        ],
    ));
    let detail = harness.call(
        "nexus_concerns",
        json!({"action":"get","concern_id":"router-issue"}),
    );
    assert_eq!(detail["ok"], true, "{detail}");
    assert_eq!(
        detail["concern"]["summary"]["counts"]["raw_observations"], 1,
        "{detail}"
    );
    assert_eq!(
        detail["concern"]["history"].as_array().unwrap().len(),
        2,
        "{detail}"
    );
    let papercuts = harness.call("nexus_papercuts", json!({"action":"list"}));
    assert_eq!(papercuts["papercuts"], json!([]));
    let sos = harness.call("nexus_sos", json!({"action":"list"}));
    assert_eq!(sos["issues"], json!([]));
    let dashboard = success(cli(&harness, &["areas"]));
    assert_eq!(dashboard["counts"]["raw_observations"], 1, "{dashboard}");
    assert_eq!(dashboard["areas"][0]["area"]["display_name"], "Router");
}

#[test]
fn mcp_binds_reporter_and_cli_transfer_preserves_original_sources() {
    let mut source = Harness::start();
    let observation = json!({"id":"report-one","concern_id":"concern-one",
        "observed_at":"2026-10-27T12:00:00Z","context":{},
        "body":{"event":"reported","title":"A code bug","description":"Observed unexpected behavior","encounter":{}}
    });
    let recorded = source.call(
        "nexus_concerns",
        json!({"action":"record","observations":[observation.clone()]}),
    );
    assert_eq!(recorded["ok"], true, "{recorded}");
    let replayed = source.call(
        "nexus_concerns",
        json!({"action":"record","observations":[observation]}),
    );
    assert_eq!(replayed["replayed"], 1, "{replayed}");
    let history = source.call(
        "nexus_concerns",
        json!({"action":"history","concern_id":"concern-one"}),
    );
    let actor = history["observations"][0]["provenance"].clone();
    assert!(actor["session_id"].as_str().unwrap().starts_with("mcp-"));
    let bundle = source.root.join("concerns.jsonl");
    success(cli(&source, &["export", bundle.to_str().unwrap()]));
    let destination = Harness::start();
    success(cli(&destination, &["import", bundle.to_str().unwrap()]));
    success(cli(&destination, &["import", bundle.to_str().unwrap()]));
    let imported = success(cli(&destination, &["history", "concern-one"]));
    assert_eq!(imported["observations"].as_array().unwrap().len(), 1);
    assert_eq!(imported["observations"][0]["source"]["provenance"], actor);
}

#[test]
fn agents_can_report_a_concern_without_constructing_an_observation_batch() {
    let mut harness = Harness::start();
    let report = harness.call("nexus_concerns", json!({
        "action":"report", "title":"Router loses state", "description":"Fallback omits the request state",
        "concern_id":"short-report", "encounter":{"symptom":"The fallback handler receives no context"}
    }));
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["inserted"], 1, "{report}");
    let repeated = harness.call(
        "nexus_concerns",
        json!({
            "action":"upvote", "concern_id":"short-report",
            "encounter":{"symptom":"Encountered in a separate request"}
        }),
    );
    assert_eq!(repeated["ok"], true, "{repeated}");
    let detail = harness.call(
        "nexus_concerns",
        json!({"action":"get","concern_id":"short-report"}),
    );
    assert_eq!(
        detail["concern"]["summary"]["counts"]["raw_observations"], 2,
        "{detail}"
    );
    for event in detail["concern"]["history"].as_array().unwrap() {
        assert!(event["provenance"]["session_id"]
            .as_str()
            .unwrap()
            .starts_with("mcp-"));
        assert!(event["input"]["observed_at"].as_str().is_some());
    }
}

#[test]
fn invalid_history_project_scopes_report_permanent_input_errors() {
    let mut harness = Harness::start();
    for tool in ["nexus_concerns", "nexus_work"] {
        let response = harness.call(tool, json!({"action":"list","project_id":" "}));
        assert_eq!(
            response["error_code"], "invalid_input",
            "{tool}: {response}"
        );
    }
}
