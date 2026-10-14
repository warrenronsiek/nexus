// @feature coordination
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
use super::*;
use serde_json::json;

#[test]
fn sos_report_rejects_blank_model_and_oversized_metadata_during_decode() {
    let base = json!({
        "action":"report",
        "session_id":"session-a",
        "agent":"codex",
        "model":"   ",
        "message":"Need help with a failing build",
        "type":"build_failure",
        "environment":"ci",
        "version":"1.0.0",
        "trace":"build timed out",
        "metadata":{}
    });
    let blank = ServiceRequest::decode("sos", base.clone()).unwrap_err();
    assert!(blank.to_string().contains("SOS model"));

    let mut oversized = base;
    oversized["model"] = json!("gpt-test");
    oversized["metadata"] = json!({
        "payload":"x".repeat(32 * 1024 + 1)
    });
    let error = ServiceRequest::decode("sos", oversized).unwrap_err();
    assert!(error.to_string().contains("SOS metadata exceeds"));
}

#[test]
fn query_scope_is_an_enum_not_a_boolean_switch() {
    let request =
        ServiceRequest::decode("claims", json!({"project_id":"project-1","scope":"all"})).unwrap();
    let ServiceRequest::Claims(query) = request else {
        panic!("expected claims query");
    };
    assert_eq!(query.scope(), RecordScope::All);

    let error = ServiceRequest::decode("claims", json!({"active_only":false})).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn explicit_read_project_ids_are_canonical_and_database_safe() {
    for method in [
        "events",
        "sessions",
        "claims",
        "conflicts",
        "dashboard",
        "usage",
    ] {
        let request =
            ServiceRequest::decode(method, json!({"project_id":" shared-project "})).unwrap();
        let project_id = match request {
            ServiceRequest::Events(query) => query.project_id,
            ServiceRequest::Sessions(query) => query.project_id,
            ServiceRequest::Claims(query) => query.project_id,
            ServiceRequest::Conflicts(query) => query.project_id,
            ServiceRequest::Dashboard(query) => query.project_id,
            ServiceRequest::Usage(query) => query.project_id,
            _ => panic!("expected a project-scoped read"),
        };
        assert_eq!(project_id.as_deref(), Some("shared-project"));

        for invalid in ["project\0suffix".to_owned(), "p".repeat(257)] {
            let error = ServiceRequest::decode(method, json!({"project_id":invalid}))
                .unwrap_err()
                .to_string();
            assert!(error.contains("project_id"), "{method}: {error}");
        }
    }
}

#[test]
fn open_ended_tool_json_is_confined_to_the_tool_payload() {
    let request = ServiceRequest::decode(
        "pre_tool_use",
        json!({
            "session_id":"session-1",
            "tool_use_id":"tool-1",
            "tool_name":"future_tool",
            "tool_input":{"future":{"shape":[1,2,3]}}
        }),
    )
    .unwrap();
    let ServiceRequest::ToolHook { input, .. } = request else {
        panic!("expected tool hook");
    };
    assert_eq!(input.tool_input.as_json()["future"]["shape"][2], 3);
}

#[test]
fn lifecycle_identity_fields_are_bounded_before_persistence_fanout() {
    for (field, value) in [
        ("session_id", "s".repeat(513)),
        ("agent", "a".repeat(257)),
        ("project_root", "p".repeat(4_097)),
        ("tool_use_id", "t".repeat(513)),
        ("tool_name", "n".repeat(257)),
    ] {
        let mut params = json!({
            "session_id":"session-1",
            "agent":"codex",
            "project_root":"/repo",
            "tool_use_id":"tool-1",
            "tool_name":"edit",
            "tool_input":{}
        });
        params[field] = value.into();

        let error = ServiceRequest::decode("pre_tool_use", params).unwrap_err();
        assert!(error.to_string().contains(field), "{field}: {error}");
    }

    let error = ServiceRequest::decode(
        "pre_tool_use",
        json!({
            "session_id":"session\nspoofed",
            "agent":"codex",
            "tool_use_id":"tool-1",
            "tool_name":"edit",
            "tool_input":{}
        }),
    )
    .unwrap_err();
    assert!(error.to_string().contains("control characters"));
}

#[test]
fn memory_add_requires_an_explicit_scope_and_round_trips_over_the_daemon_wire() {
    let request = ServiceRequest::decode(
        "memory_add",
        json!({
            "scope":"project",
            "content":"Prefer typed boundaries",
            "session_id":"session-1",
            "project_root":"/tmp/project",
            "agent":"codex",
            "model":"gpt-test"
        }),
    )
    .unwrap();
    let ServiceRequest::MemoryAdd(command) = &request else {
        panic!("expected memory add command");
    };
    assert_eq!(command.scope, MemoryScopeArg::Project);
    assert_eq!(command.content, "Prefer typed boundaries");
    assert_eq!(command.context.session_id, "session-1");

    let (method, params) = request.wire_parts().unwrap();
    assert_eq!(method, "memory_add");
    assert_eq!(params["scope"], "project");

    let error = ServiceRequest::decode(
        "memory_add",
        json!({
            "content":"Ambiguous scope",
            "session_id":"session-1"
        }),
    )
    .unwrap_err();
    assert!(error.to_string().contains("scope"));
}

#[test]
fn memory_search_limit_matches_the_persistence_bound() {
    let request = ServiceRequest::decode(
        "memory_search",
        json!({"scope":"global","regex":"typed","limit":9999}),
    )
    .unwrap();
    let ServiceRequest::MemorySearch(query) = request else {
        panic!("expected memory search");
    };
    assert_eq!(query.bounded_limit(), 64);
}

#[test]
fn memory_search_regex_is_bounded_before_compilation() {
    let error = ServiceRequest::decode(
        "memory_search",
        json!({"scope":"global","regex":"x".repeat(4_097)}),
    )
    .unwrap_err();

    assert!(error.to_string().contains("regex"), "{error}");
}

#[test]
fn event_query_limit_matches_the_bounded_response_window() {
    let request = ServiceRequest::decode("events", json!({"limit":usize::MAX})).unwrap();
    let ServiceRequest::Events(query) = request else {
        panic!("expected events query");
    };

    assert_eq!(query.bounded_limit(), 64);
}

#[test]
fn papercuts_commands_are_typed_tagged_and_round_trip_over_the_daemon_wire() {
    let request = ServiceRequest::decode(
        "papercuts",
        json!({
            "action":"report",
            "session_id":"session-1",
            "project_root":"/tmp/project",
            "agent":"codex",
            "title":"Flaky socket test"
        }),
    )
    .unwrap();
    let ServiceRequest::Papercuts(PapercutCommand::Report {
        context,
        title,
        description,
    }) = &request
    else {
        panic!("expected papercut report command");
    };
    assert_eq!(context.session_id, "session-1");
    assert_eq!(title, "Flaky socket test");
    assert_eq!(description, "");

    let (method, params) = request.wire_parts().unwrap();
    assert_eq!(method, "papercuts");
    assert_eq!(params["action"], "report");
    assert_eq!(params["project_root"], "/tmp/project");

    let error = ServiceRequest::decode(
        "papercuts",
        json!({
            "action":"report",
            "session_id":"session-1",
            "title":"Flaky socket test",
            "unexpected":true
        }),
    )
    .unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn papercut_dump_scope_defaults_to_project_and_can_select_all_projects() {
    for (params, expected) in [
        (
            json!({"action":"dump","session_id":"session-1"}),
            PapercutDumpScope::Project,
        ),
        (
            json!({"action":"dump","session_id":"session-1","scope":"all"}),
            PapercutDumpScope::All,
        ),
    ] {
        let request = ServiceRequest::decode("papercuts", params).unwrap();
        let ServiceRequest::Papercuts(PapercutCommand::Dump { scope, .. }) = request else {
            panic!("expected papercut dump command");
        };
        assert_eq!(scope, expected);
    }
}

#[test]
fn papercut_duplicate_response_requires_the_reporter_token() {
    let request = ServiceRequest::decode(
        "papercuts",
        json!({
            "action":"respond",
            "session_id":"session-1",
            "review_id":"review-1",
            "reporter_token":"secret-token",
            "decision":"approve",
            "response":"These describe the same failure."
        }),
    )
    .unwrap();
    let ServiceRequest::Papercuts(PapercutCommand::Respond { reporter_token, .. }) = request else {
        panic!("expected papercut duplicate response");
    };
    assert_eq!(reporter_token, "secret-token");

    let error = ServiceRequest::decode(
        "papercuts",
        json!({
            "action":"respond",
            "session_id":"session-1",
            "review_id":"review-1",
            "decision":"approve",
            "response":"These describe the same failure."
        }),
    )
    .unwrap_err();
    assert!(error.to_string().contains("reporter_token"));
}
