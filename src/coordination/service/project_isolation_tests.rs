// @feature papercuts
// @spec docs/features/papercuts.md
// @feature coordination
// @feature persistence
// @feature agent-memory
// @spec docs/features/coordination.md
// @spec docs/features/persistence.md
// @spec docs/features/agent-memory.md
use super::*;
use crate::config::Config;
use crate::coordination::client_evidence::ClientFilesystemEvidence;
use crate::coordination::domain::{
    Claim, ClaimState, Operation, PathIntent, RecordScope, SessionRecord, SessionStatus,
};
use serde_json::json;

fn service() -> NexusService {
    super::tests::service_with(Config::default())
}

fn remote_edit(project_root: &str) -> ServiceRequest {
    ServiceRequest::decode(
        "pre_tool_use",
        json!({
            "session_id":"shared-session",
            "agent":"codex",
            "project_root":project_root,
            "tool_use_id":"shared-tool",
            "tool_name":"Edit",
            "tool_input":{"file_path":format!("{project_root}/src/lib.rs")}
        }),
    )
    .unwrap()
}

fn claims_for(service: &NexusService, project_id: &str) -> Vec<Claim> {
    service
        .store
        .lock()
        .unwrap()
        .list_claims(Some(project_id), RecordScope::All)
        .unwrap()
}

#[test]
fn completing_a_shared_raw_session_id_is_scoped_to_its_remote_project() {
    let service = service();
    service.handle_remote("project-a", remote_edit("/compute/a/repo"));
    service.handle_remote("project-b", remote_edit("/compute/b/repo"));
    let completion = ServiceRequest::decode(
        "post_tool_use",
        json!({
            "session_id":"shared-session",
            "agent":"codex",
            "project_root":"/compute/a/repo",
            "tool_use_id":"shared-tool",
            "tool_name":"Edit",
            "tool_input":{"file_path":"/compute/a/repo/src/lib.rs"},
            "tool_output":"done"
        }),
    )
    .unwrap();
    service.handle_remote("project-a", completion);

    assert_eq!(
        claims_for(&service, "project-a")[0].state,
        ClaimState::Modified
    );
    assert_eq!(
        claims_for(&service, "project-b")[0].state,
        ClaimState::Claimed
    );
}

#[test]
fn stopping_a_shared_raw_session_id_is_scoped_to_its_remote_project() {
    let service = service();
    service.handle_remote("project-a", remote_edit("/compute/a/repo"));
    service.handle_remote("project-b", remote_edit("/compute/b/repo"));

    let stop = ServiceRequest::decode(
        "session_stop",
        json!({
            "session_id":"shared-session",
            "agent":"codex",
            "project_root":"/compute/a/repo"
        }),
    )
    .unwrap();
    service.handle_remote("project-a", stop);

    let sessions = service
        .store
        .lock()
        .unwrap()
        .list_sessions(RecordScope::All, 120)
        .unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        session_status(&sessions, "project-a"),
        SessionStatus::Stopped
    );
    assert_eq!(
        session_status(&sessions, "project-b"),
        SessionStatus::Active
    );
    assert_eq!(
        claims_for(&service, "project-a")[0].state,
        ClaimState::Released
    );
    assert_eq!(
        claims_for(&service, "project-b")[0].state,
        ClaimState::Claimed
    );
}

fn session_status(sessions: &[SessionRecord], project_id: &str) -> SessionStatus {
    sessions
        .iter()
        .find(|session| session.project_id == project_id)
        .unwrap()
        .status
}

#[test]
fn releasing_a_shared_raw_session_id_is_scoped_to_its_remote_project() {
    let service = service();
    service.handle_remote("project-a", remote_edit("/compute/a/repo"));
    service.handle_remote("project-b", remote_edit("/compute/b/repo"));

    let response = service
        .handle_remote(
            "project-a",
            ServiceRequest::Release(ReleaseCommand {
                session_id: "shared-session".into(),
                path: None,
            }),
        )
        .to_json();

    assert_eq!(response["released"], 1, "{response}");
    assert_eq!(
        claims_for(&service, "project-a")[0].state,
        ClaimState::Released
    );
    assert_eq!(
        claims_for(&service, "project-b")[0].state,
        ClaimState::Claimed
    );
}

#[test]
fn stale_reconciliation_cannot_reactivate_a_stopped_session_or_its_claims() {
    let service = service();
    service.handle_remote("project-a", remote_edit("/compute/a/repo"));
    service.handle_remote(
        "project-a",
        ServiceRequest::decode(
            "session_stop",
            json!({
                "session_id":"shared-session",
                "agent":"codex",
                "project_root":"/compute/a/repo"
            }),
        )
        .unwrap(),
    );

    service
        .reconcile_remote_evidence(
            "project-a",
            &ClientFilesystemEvidence::reconciliation(
                "shared-session",
                "codex",
                vec![PathIntent {
                    path: "src/stale.rs".into(),
                    operation: Operation::Write,
                    line_start: None,
                    line_end: None,
                }],
                false,
            ),
            true,
        )
        .unwrap();

    let mut store = service.store.lock().unwrap();
    let sessions = store.list_sessions(RecordScope::All, 120).unwrap();
    assert_eq!(
        session_status(&sessions, "project-a"),
        SessionStatus::Stopped
    );
    assert!(store
        .list_claims(Some("project-a"), RecordScope::Active)
        .unwrap()
        .is_empty());
}

#[test]
fn remote_reads_default_to_the_envelope_project() {
    let service = service();
    seed_remote_read_projects(&service);
    assert_default_reads_are_project_scoped(&service);
    assert_dashboard_and_usage_are_project_scoped(&service);
    assert_local_and_explicit_reads_remain_available(&service);
}

fn seed_remote_read_projects(service: &NexusService) {
    for (project, root) in [
        ("project-a", "/compute/a/repo"),
        ("project-b", "/compute/b/repo"),
    ] {
        service.handle_remote(project, remote_edit(root));
        service.handle_remote(
            project,
            ServiceRequest::decode(
                "pre_tool_use",
                json!({
                    "session_id":format!("{project}-other"), "agent":"claude",
                    "project_root":root, "tool_use_id":"other-tool", "tool_name":"Edit",
                    "tool_input":{"file_path":format!("{root}/src/lib.rs")}
                }),
            )
            .unwrap(),
        );
    }
}

fn assert_default_reads_are_project_scoped(service: &NexusService) {
    for (method, params, field) in [
        ("sessions", json!({"scope":"all"}), "sessions"),
        ("claims", json!({"scope":"all"}), "claims"),
        ("events", json!({"limit":100}), "events"),
        ("conflicts", json!({"scope":"all"}), "conflicts"),
    ] {
        let response = service
            .handle_remote("project-a", ServiceRequest::decode(method, params).unwrap())
            .to_json();
        let records = response[field].as_array().unwrap();
        assert!(!records.is_empty(), "{response}");
        assert!(
            records
                .iter()
                .all(|record| record["project_id"] == "project-a"),
            "{response}"
        );
    }
}

fn assert_dashboard_and_usage_are_project_scoped(service: &NexusService) {
    let dashboard = service
        .handle_remote(
            "project-a",
            ServiceRequest::decode("dashboard", json!({})).unwrap(),
        )
        .to_json();
    assert_eq!(dashboard["project_id"], "project-a");
    assert!(dashboard["sessions"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|session| session["project_id"] == "project-a"));

    let usage = service
        .handle_remote(
            "project-a",
            ServiceRequest::decode("usage", json!({})).unwrap(),
        )
        .to_json();
    assert_eq!(usage["project_id"], "project-a");
    assert_eq!(usage["tools"][0]["count"], 2);
}

fn assert_local_and_explicit_reads_remain_available(service: &NexusService) {
    let local = service
        .handle(ServiceRequest::decode("sessions", json!({"scope":"all"})).unwrap())
        .to_json();
    assert_eq!(local["sessions"].as_array().unwrap().len(), 4);

    let explicit = service
        .handle_remote(
            "project-a",
            ServiceRequest::decode("claims", json!({"project_id":"project-b","scope":"all"}))
                .unwrap(),
        )
        .to_json();
    assert!(explicit["claims"]
        .as_array()
        .unwrap()
        .iter()
        .all(|claim| claim["project_id"] == "project-b"));

    let explicit_sessions = service
        .handle_remote(
            "project-a",
            ServiceRequest::decode("sessions", json!({"project_id":"project-b","scope":"all"}))
                .unwrap(),
        )
        .to_json();
    assert!(explicit_sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|session| session["project_id"] == "project-b"));
}

#[test]
fn papercut_list_uses_the_selected_project_without_changing_write_binding() {
    let service = service();
    for project in ["project-a", "project-b"] {
        service.handle_remote(
            project,
            ServiceRequest::decode(
                "papercuts",
                json!({"action":"report", "session_id":"tui", "title":project}),
            )
            .unwrap(),
        );
    }
    let selected = service.handle_remote(
        "project-a",
        ServiceRequest::decode(
            "papercuts",
            json!({"action":"list", "session_id":"tui", "project_id":"project-b"}),
        )
        .unwrap(),
    );
    let selected = selected.to_json();
    assert_eq!(selected["papercuts"].as_array().unwrap().len(), 1);
    assert_eq!(selected["papercuts"][0]["title"], "project-b");
    let current = service.handle_remote(
        "project-a",
        ServiceRequest::decode("papercuts", json!({"action":"list", "session_id":"tui"})).unwrap(),
    );
    let current = current.to_json();
    assert_eq!(current["papercuts"][0]["title"], "project-a");
}
