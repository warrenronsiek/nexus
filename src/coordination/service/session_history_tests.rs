// @feature coordination
// @spec docs/features/coordination.md
// @boundary service-wire-fixture: JSON requests and responses verify the public coordination wire contract.
use super::tests::service_with;
use super::*;
use crate::config::Config;
use serde_json::{json, Value};

const PROJECT: &str = "timeline-project";

fn service() -> NexusService {
    service_with(Config::default())
}

fn request(method: &str, params: Value) -> ServiceRequest {
    ServiceRequest::decode(method, params).unwrap()
}

fn write(service: &NexusService, session: &str, agent: &str, tool: &str) -> Value {
    service
        .handle_remote(
            PROJECT,
            request(
                "pre_tool_use",
                json!({
                    "session_id": session,
                    "agent": agent,
                    "project_root": "/client/checkout",
                    "tool_use_id": tool,
                    "tool_name": "Write",
                    "tool_input": {"file_path": "/client/checkout/src/shared.rs"}
                }),
            ),
        )
        .to_json()
}

fn read(service: &NexusService, method: &str, params: Value) -> Value {
    service
        .handle_remote(PROJECT, request(method, params))
        .to_json()
}

fn timeline(service: &NexusService, session: &str) -> Value {
    read(service, "session_timeline", json!({"session_id": session}))
}

fn entry_summary(timeline: &Value) -> Vec<(String, String, String)> {
    timeline["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["type"].as_str().unwrap().to_owned(),
                entry["direction"].as_str().unwrap().to_owned(),
                entry["counterpart"]["session_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect()
}

fn summary(items: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    items
        .iter()
        .map(|(kind, direction, counterpart)| {
            (
                (*kind).to_owned(),
                (*direction).to_owned(),
                (*counterpart).to_owned(),
            )
        })
        .collect()
}

#[test]
fn advisories_name_the_session_holding_the_overlapping_claim() {
    let service = service();
    write(&service, "3f2a9c1b-first-session", "codex", "tool-one");

    let response = write(&service, "second-session", "claude", "tool-two");

    assert_eq!(
        response["advisories"][0]["message"],
        "codex (session 3f2a9c1b) has an active claim on src/shared.rs",
        "{response}"
    );
    assert_eq!(response["advisories"][0]["kind"], "path_overlap");
    assert_eq!(response["advisories"][0]["severity"], "warning");
}

#[test]
fn session_timeline_reports_received_and_sent_coordination_with_named_counterparts() {
    let service = three_way_overlap();

    let before = timeline(&service, "two");
    assert_eq!(before["ok"], true, "{before}");
    assert_eq!(before["session"]["session_id"], "two");
    assert_eq!(before["session"]["agent"], "claude");
    assert_eq!(before["truncated"], false);
    assert_eq!(
        entry_summary(&before),
        summary(&[
            ("conflict_detected", "received", "one"),
            ("advisory", "received", "one"),
            ("conflict_detected", "sent", "three"),
            ("advisory", "sent", "three"),
        ])
    );
    let delivered = &before["entries"][1];
    let mut keys: Vec<_> = delivered.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "at",
            "conflict_id",
            "counterpart",
            "direction",
            "id",
            "message",
            "path",
            "severity",
            "status",
            "type"
        ]
    );
    assert_eq!(
        delivered["counterpart"],
        json!({"session_id":"one","agent":"codex","display_name":null})
    );
    assert_eq!(delivered["path"], "src/shared.rs");
    assert_eq!(delivered["status"], "open");
    assert_eq!(
        delivered["message"],
        "codex (session one) has an active claim on src/shared.rs"
    );
}

fn three_way_overlap() -> NexusService {
    let service = service();
    write(&service, "one", "codex", "tool-one");
    write(&service, "two", "claude", "tool-two");
    write(&service, "three", "pi", "tool-three");
    service
}

fn statuses(service: &NexusService) -> Vec<String> {
    read(service, "conflicts", json!({"scope": "all"}))["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|conflict| conflict["status"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn release_and_stop_end_conflicts_and_add_resolution_entries() {
    let service = three_way_overlap();

    let released = read(&service, "release", json!({"session_id": "two"}));
    assert_eq!(released["ok"], true, "{released}");
    let after = timeline(&service, "two");
    let mut resolutions: Vec<_> = after["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["type"] == "conflict_resolved")
        .map(|entry| {
            (
                entry["status"].as_str().unwrap().to_owned(),
                entry["counterpart"]["session_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect();
    resolutions.sort();
    assert_eq!(
        resolutions,
        [
            ("ended".to_owned(), "one".to_owned()),
            ("ended".to_owned(), "three".to_owned())
        ]
    );

    let stopped = service.handle_remote(
        PROJECT,
        request(
            "session_stop",
            json!({"session_id": "one", "agent": "codex"}),
        ),
    );
    assert_eq!(stopped.to_json()["permitted"], true);
    assert_eq!(statuses(&service), ["ended", "ended", "ended"]);

    let dashboard = read(&service, "dashboard", json!({}));
    assert_eq!(dashboard["counts"]["open_conflicts"], 0);
    assert_eq!(dashboard["counts"]["recent_conflicts"], 3);
    assert_eq!(dashboard["counts"]["recent_advisories"], 3);
}

#[test]
fn session_index_lists_project_sessions_with_their_coordination_counts() {
    let service = service();
    write(&service, "one", "codex", "tool-one");
    write(&service, "two", "claude", "tool-two");
    service.handle_remote(
        "other-project",
        request(
            "user_prompt",
            json!({"session_id":"elsewhere","agent":"codex","prompt":"unrelated"}),
        ),
    );

    let index = read(&service, "session_index", json!({}));

    assert_eq!(index["ok"], true, "{index}");
    assert_eq!(index["truncated"], false);
    let sessions: Vec<_> = index["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| {
            (
                session["session_id"].as_str().unwrap().to_owned(),
                session["conflicts"].as_i64().unwrap(),
                session["advisories"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(sessions, [("one".into(), 1, 0), ("two".into(), 1, 1)]);
    for key in [
        "agent",
        "display_name",
        "task_summary",
        "status",
        "last_seen_at",
        "last_activity_at",
    ] {
        assert!(
            index["sessions"][0].get(key).is_some(),
            "missing {key}: {index}"
        );
    }
}

#[test]
fn session_history_reads_require_a_project_and_a_bounded_session_identity() {
    let service = service();
    let unscoped = service
        .handle(request("session_timeline", json!({"session_id": "one"})))
        .to_json();
    assert_eq!(unscoped["ok"], false);
    assert!(unscoped["error"]
        .as_str()
        .unwrap()
        .contains("select a project"));
    let index = service
        .handle(request("session_index", json!({})))
        .to_json();
    assert_eq!(index["ok"], false);

    for invalid in [json!(""), json!("x".repeat(513)), json!("line\nbreak")] {
        assert!(ServiceRequest::decode(
            "session_timeline",
            json!({"project_id": PROJECT, "session_id": invalid})
        )
        .is_err());
    }
    let absent = read(
        &service,
        "session_timeline",
        json!({"session_id": "absent"}),
    );
    assert_eq!(
        absent,
        json!({"ok": true, "session": null, "entries": [], "truncated": false})
    );
}
