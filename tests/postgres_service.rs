// @feature persistence
// @feature runtime
// @feature coordination
// @feature agent-memory
// @feature papercuts
// @spec docs/features/persistence.md
// @spec docs/features/runtime.md
// @spec docs/features/coordination.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @boundary external-postgres-and-json
#![cfg(feature = "postgres")]

use nexus::coordination::api::ServiceRequest;
use nexus::{Config, LoadedConfig, NexusService};
use serde_json::{json, Value};
use std::sync::Mutex;
use uuid::Uuid;

static POSTGRES_TEST_LOCK: Mutex<()> = Mutex::new(());

fn request(method: &str, params: Value) -> ServiceRequest {
    ServiceRequest::decode(method, params).unwrap()
}

#[test]
fn postgres_service_preserves_coordination_memory_and_papercut_semantics() {
    let _guard = POSTGRES_TEST_LOCK.lock().unwrap();
    let database_url = std::env::var("NEXUS_TEST_DATABASE_URL")
        .expect("NEXUS_TEST_DATABASE_URL is required for the PostgreSQL integration test");
    let config = Config::default();
    let loaded = LoadedConfig {
        config,
        sources: Vec::new(),
        hash: "postgres-test-config".into(),
    };
    let service = NexusService::connect(loaded, &database_url).unwrap();
    let project_id = format!("postgres-test-{}", Uuid::new_v4());

    let first = request(
        "pre_tool_use",
        json!({
            "session_id":format!("{project_id}-one"), "agent":"codex",
            "project_root":"/compute/one/repo", "tool_use_id":"edit-one", "tool_name":"Edit",
            "tool_input":{"file_path":"/compute/one/repo/src/lib.rs","line_start":10,"line_end":20}
        }),
    );
    let second = request(
        "pre_tool_use",
        json!({
            "session_id":format!("{project_id}-two"), "agent":"claude",
            "project_root":"/compute/two/repo", "tool_use_id":"edit-two", "tool_name":"Edit",
            "tool_input":{"file_path":"/compute/two/repo/src/lib.rs","line_start":15,"line_end":18}
        }),
    );
    assert!(
        service.handle_remote(&project_id, first).to_json()["advisories"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let overlap = service.handle_remote(&project_id, second).to_json();
    assert_eq!(overlap["advisories"][0]["kind"], "hunk_overlap");

    let memory_text = format!("PostgreSQL memory {}", Uuid::new_v4());
    let added = service
        .handle_remote(
            &project_id,
            request(
                "memory_add",
                json!({
                    "scope":"project", "content":memory_text,
                    "session_id":format!("{project_id}-memory"), "agent":"test",
                    "project_root":"/compute/one/repo"
                }),
            ),
        )
        .to_json();
    assert_eq!(added["ok"], true, "{added}");
    let memories = service
        .handle_remote(
            &project_id,
            request(
                "memory_search",
                json!({"scope":"project","project_root":"/other/clone","regex":"PostgreSQL memory"}),
            ),
        )
        .to_json();
    assert!(memories["memories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|memory| memory["content"] == memory_text));

    let title = format!("PostgreSQL papercut {}", Uuid::new_v4());
    let reported = service
        .handle_remote(
            &project_id,
            request(
                "papercuts",
                json!({
                    "action":"report", "title":title, "description":"shared service friction",
                    "session_id":format!("{project_id}-reporter"), "agent":"test",
                    "project_root":"/compute/one/repo"
                }),
            ),
        )
        .to_json();
    assert_eq!(reported["created"], true, "{reported}");
    let papercut_id = reported["papercut"]["id"].as_str().unwrap();
    let upvoted = service
        .handle_remote(
            &project_id,
            request(
                "papercuts",
                json!({
                    "action":"upvote", "papercut_id":papercut_id,
                    "session_id":format!("{project_id}-voter"), "agent":"other",
                    "project_root":"/compute/two/repo"
                }),
            ),
        )
        .to_json();
    assert_eq!(upvoted["papercut"]["upvote_count"], 2, "{upvoted}");
}

#[test]
fn postgres_service_isolates_identical_session_ids_between_projects() {
    let _guard = POSTGRES_TEST_LOCK.lock().unwrap();
    let database_url = std::env::var("NEXUS_TEST_DATABASE_URL")
        .expect("NEXUS_TEST_DATABASE_URL is required for the PostgreSQL integration test");
    let service = NexusService::connect(
        LoadedConfig {
            config: Config::default(),
            sources: Vec::new(),
            hash: "postgres-project-isolation".into(),
        },
        &database_url,
    )
    .unwrap();
    let suffix = Uuid::new_v4();
    let project_a = format!("postgres-isolation-a-{suffix}");
    let project_b = format!("postgres-isolation-b-{suffix}");
    let session_id = format!("shared-session-{suffix}");

    for (project, root) in [
        (&project_a, "/compute/project-a"),
        (&project_b, "/compute/project-b"),
    ] {
        let response = service
            .handle_remote(
                project,
                request(
                    "pre_tool_use",
                    json!({
                        "session_id":session_id,
                        "agent":"codex",
                        "project_root":root,
                        "tool_use_id":"shared-tool",
                        "tool_name":"Edit",
                        "tool_input":{"file_path":format!("{root}/src/lib.rs")}
                    }),
                ),
            )
            .to_json();
        assert_eq!(response["recorded"], true, "{response}");
    }

    service.handle_remote(
        &project_a,
        request(
            "post_tool_use",
            json!({
                "session_id":session_id,
                "agent":"codex",
                "project_root":"/compute/project-a",
                "tool_use_id":"shared-tool",
                "tool_name":"Edit",
                "tool_input":{"file_path":"/compute/project-a/src/lib.rs"},
                "tool_output":"done"
            }),
        ),
    );
    assert_eq!(claim_state(&service, &project_b), "claimed");

    let released = service
        .handle_remote(
            &project_a,
            request("release", json!({"session_id":session_id})),
        )
        .to_json();
    assert_eq!(released["released"], 1, "{released}");
    assert_eq!(claim_state(&service, &project_b), "claimed");

    service.handle_remote(
        &project_a,
        request(
            "session_stop",
            json!({
                "session_id":session_id,
                "agent":"codex",
                "project_root":"/compute/project-a"
            }),
        ),
    );
    let project_a_sessions = service
        .handle_remote(&project_a, request("sessions", json!({"scope":"all"})))
        .to_json();
    let project_b_sessions = service
        .handle_remote(&project_b, request("sessions", json!({"scope":"all"})))
        .to_json();
    let project_a_sessions = project_a_sessions["sessions"].as_array().unwrap();
    let project_b_sessions = project_b_sessions["sessions"].as_array().unwrap();
    assert_eq!(project_a_sessions.len(), 1, "{project_a_sessions:?}");
    assert_eq!(project_b_sessions.len(), 1, "{project_b_sessions:?}");
    assert_eq!(
        session_status(project_a_sessions, &project_a, &session_id),
        "stopped"
    );
    assert_eq!(
        session_status(project_b_sessions, &project_b, &session_id),
        "active"
    );
    assert_eq!(claim_state(&service, &project_b), "claimed");
}

fn claim_state(service: &NexusService, project_id: &str) -> String {
    service
        .handle_remote(
            project_id,
            request("claims", json!({"project_id":project_id,"scope":"all"})),
        )
        .to_json()["claims"][0]["state"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn session_status(sessions: &[Value], project_id: &str, session_id: &str) -> String {
    sessions
        .iter()
        .find(|session| session["project_id"] == project_id && session["session_id"] == session_id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_owned()
}
