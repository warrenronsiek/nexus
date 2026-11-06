// @feature coordination
// @feature observability-ui
// @spec docs/features/coordination.md
// @spec docs/features/observability-ui.md
// @boundary dynamic-json: tests inspect the public JSON projects response.
use super::tests::service_with;
use super::*;
use crate::config::Config;
use crate::coordination::api::ServiceRequest;
use serde_json::{json, Value};

fn prompt(session: &str) -> ServiceRequest {
    ServiceRequest::decode(
        "user_prompt",
        json!({"session_id":session,"agent":"codex","project_root":"/client/checkout","prompt":"work"}),
    )
    .unwrap()
}

fn memory_note() -> ServiceRequest {
    ServiceRequest::decode(
        "memory_add",
        json!({"scope":"project","content":"A durable note","session_id":"caller","agent":"codex","model":"gpt-test"}),
    )
    .unwrap()
}

fn projects(service: &NexusService) -> Vec<Value> {
    service.handle(ServiceRequest::Projects).to_json()["projects"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn remote_activity_lists_the_project_under_its_client_reported_name() {
    let service = service_with(Config::default());
    let binding = ProjectBinding::remote("0123456789abcdef01234567", Some("example-app".into()));
    service.handle_in(binding, prompt("named"));

    let listed = projects(&service);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["project_id"], "0123456789abcdef01234567");
    assert_eq!(listed[0]["display_name"], "example-app");
}

#[test]
fn unnamed_activity_keeps_the_earlier_reported_name() {
    let service = service_with(Config::default());
    service.handle_in(
        ProjectBinding::remote("bound", Some("nexus".into())),
        prompt("new-client"),
    );
    service.handle_remote("bound", prompt("old-client"));

    assert_eq!(projects(&service)[0]["display_name"], "nexus");
}

#[test]
fn caller_owned_records_register_their_project_without_host_sessions() {
    let service = service_with(Config::default());
    let response = service.handle_in(ProjectBinding::remote("notes-only", None), memory_note());
    assert_eq!(response.to_json()["ok"], true);

    let listed = projects(&service);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["project_id"], "notes-only");
    assert_eq!(listed[0]["display_name"], Value::Null);
}

#[test]
fn browser_inspection_does_not_make_a_project_look_active() {
    let service = service_with(Config::default());
    let inspect = ProjectBinding::Inspect {
        project_id: Some("browsed".into()),
    };
    let status = ServiceRequest::decode("memory_status", json!({})).unwrap();
    service.handle_in(inspect, status);

    assert!(projects(&service).is_empty());
}

#[test]
fn invalid_client_names_are_omitted_rather_than_stored() {
    let service = service_with(Config::default());
    let binding = ProjectBinding::remote("bound", Some("bad\nname".into()));
    service.handle_in(binding, prompt("session"));

    assert_eq!(projects(&service)[0]["display_name"], Value::Null);
}
