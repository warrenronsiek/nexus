// @feature observability-ui
// @feature runtime
// @spec docs/features/observability-ui.md
// @spec docs/features/runtime.md
// @boundary dynamic-json
mod support;

use serde_json::json;
use support::Harness;

#[test]
fn pi_dashboard_reads_round_trip_over_the_local_mcp_process() {
    let mut harness = Harness::start();
    harness.initialize_git();
    let root = harness.root.to_string_lossy().to_string();
    let prompt = harness.call(
        "nexus_user_prompt",
        json!({
            "session_id":"pi-dashboard",
            "project_root":root,
            "agent":"pi",
            "prompt":"inspect coordination"
        }),
    );
    let project_id = prompt["project_id"].as_str().unwrap();

    let projects = harness.call("nexus_projects", json!({}));
    assert_eq!(projects["ok"], true, "{projects}");
    assert!(projects["projects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|project| project["project_id"] == project_id));

    let dashboard = harness.call("nexus_dashboard", json!({"project_id":project_id}));
    assert_eq!(dashboard["ok"], true, "{dashboard}");
    assert_eq!(dashboard["project_id"], project_id);
    assert_eq!(dashboard["counts"]["active_sessions"], 1);
}
