// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary synthetic-native-hook-json
use nexus::coordination::api::ServiceRequest;
use nexus::runtime::work_capture::{host_event, host_handoff};
use nexus::work_history::{WorkAction, WorkEvent, WorkObservationInput, WorkOutcome};
use serde_json::json;

fn events(value: serde_json::Value) -> Vec<WorkObservationInput> {
    let command = host_event("codex", &serde_json::to_vec(&value).unwrap()).unwrap();
    let WorkAction::Record { observations } = command.command else {
        panic!("record")
    };
    observations
}

fn declared<'a>(events: &'a [WorkObservationInput], expected: &str) -> &'a WorkObservationInput {
    events
        .iter()
        .find(|item| matches!(&item.body,WorkEvent::ActivityDeclared {kind,..} if kind==expected))
        .unwrap()
}

#[test]
fn request_turn_tools_and_child_keep_distinct_causal_boundaries() {
    let input = json!({"session_id":"host-1","turn_id":"turn-1","cwd":"/checkout"});
    let mut start = input.clone();
    start["hook_event_name"] = json!("UserPromptSubmit");
    let observed = events(start);
    for kind in ["request", "host.turn"] {
        assert!(observed
            .iter()
            .any(|o| matches!(&o.body, WorkEvent::ActivityDeclared { kind: k, .. } if k == kind)));
    }
    let context = host_handoff("codex", "host-1", Some("turn-1"), None);
    let mut child = input.clone();
    child["hook_event_name"] = json!("SubagentStart");
    child["agent_id"] = json!("child-9");
    child["agent_type"] = json!("reviewer");
    let child = events(child);
    let declaration = declared(&child, "agent.task");
    let WorkEvent::ActivityDeclared {
        parent_activity_id,
        executor,
        ..
    } = &declaration.body
    else {
        panic!("declaration")
    };
    assert_eq!(parent_activity_id, &context.parent_activity_id);
    assert_eq!(executor.as_deref(), Some(declaration.subject.id()));
    let mut stop = input;
    stop["hook_event_name"] = json!("Stop");
    let ended = events(stop);
    assert_eq!(
        ended
            .iter()
            .filter(|o| matches!(o.body, WorkEvent::Finished { .. }))
            .count(),
        2
    );
    assert!(!ended
        .iter()
        .any(|o| o.subject.scope() == "session" && matches!(o.body, WorkEvent::Finished { .. })));
}

#[test]
fn host_end_closes_only_automatically_owned_root_and_interrupt_is_canceled() {
    let ended = events(json!({"session_id":"owned","hook_event_name":"SessionEnd"}));
    assert!(ended.iter().any(|o| o.subject.scope() == "session"
        && matches!(
            o.body,
            WorkEvent::Finished {
                outcome: WorkOutcome::Unknown,
                ..
            }
        )));
    let ended = events(
        json!({"session_id":"shared-host","hook_event_name":"SessionEnd","work_session_id":"shared"}),
    );
    assert!(!ended
        .iter()
        .any(|o| o.subject.scope() == "session" && matches!(o.body, WorkEvent::Finished { .. })));
    let stopped = events(json!({"session_id":"owned","turn_id":"t","hook_event_name":"Interrupt"}));
    assert!(stopped.iter().any(|o| matches!(
        o.body,
        WorkEvent::Finished {
            outcome: WorkOutcome::Canceled,
            ..
        }
    )));
}

#[test]
fn missing_turn_identity_never_deduplicates_requests_by_prompt_text() {
    let first = events(
        json!({"session_id":"legacy","hook_event_name":"UserPromptSubmit","prompt":"same secret prompt"}),
    );
    assert!(!first
        .iter()
        .any(|o| matches!(&o.body, WorkEvent::ActivityDeclared { kind, .. } if kind == "request")));
    assert!(!serde_json::to_string(&first).unwrap().contains("secret"));
    assert!(first.iter().any(|o| matches!(&o.body, WorkEvent::Milestone { context, .. } if context.labels.contains_key("nexus.capture.gap"))));
}

#[test]
fn asynchronous_execution_is_not_finished_by_dispatch_stop_and_poll_is_not_execution() {
    let base = json!({"session_id":"host","turn_id":"turn","work_session_id":"work","execution_id":"job-1","parent_activity_id":"launcher","executor":"runner-1","name":"review","kind":"review","occurred_at":"2026-10-27T12:00:00Z"});
    let mut start = base.clone();
    start["hook_event_name"] = json!("ExecutionStart");
    let started = events(start);
    assert!(
        started
            .iter()
            .all(|observation| observation.source_clock.is_none()),
        "historical source UTC cannot acquire current callback clocks"
    );
    let mut poll = base.clone();
    poll["hook_event_name"] = json!("ExecutionPoll");
    let polled = events(poll);
    assert!(!polled
        .iter()
        .any(|o| matches!(o.body, WorkEvent::Started | WorkEvent::Finished { .. })));
    let id = started
        .iter()
        .find(|o| matches!(o.body, WorkEvent::Started))
        .unwrap()
        .subject
        .clone();
    let mut finish = base;
    finish["hook_event_name"] = json!("ExecutionFinish");
    finish["outcome"] = json!("failed");
    let finished = events(finish);
    assert!(finished.iter().any(|o| o.subject == id
        && matches!(
            o.body,
            WorkEvent::Finished {
                outcome: WorkOutcome::Failed,
                ..
            }
        )));
}

#[test]
fn host_session_title_changes_are_evidence_on_the_same_root() {
    let a = events(
        json!({"session_id":"h","hook_event_name":"SessionStart","session_title":"Implement accounting"}),
    );
    let b = events(
        json!({"session_id":"h","hook_event_name":"SessionTitle","session_title":"Close accounting gaps","source_event_id":"rename-1"}),
    );
    let root = a
        .iter()
        .find(|o| matches!(o.body, WorkEvent::SessionOpened { .. }))
        .unwrap();
    assert!(b.iter().any(|o| o.subject==root.subject && matches!(&o.body, WorkEvent::Milestone { context, .. } if context.labels.get("nexus.session_title").map(String::as_str)==Some("Close accounting gaps"))));
    let _ = ServiceRequest::is_lifecycle_method("pre_tool_use");
}

#[test]
fn resumed_native_sessions_disclose_the_missing_execution_epoch() {
    let resumed =
        events(json!({"session_id":"resumed","hook_event_name":"SessionStart","source":"resume"}));
    assert!(resumed.iter().any(|item|matches!(&item.body,WorkEvent::Milestone {context,..} if context.labels.get("nexus.capture.gap").map(String::as_str)==Some("host_resume_epoch_unavailable"))));
}

#[test]
fn same_host_type_does_not_merge_distinct_host_executor_instances() {
    let executor = |session| {
        events(json!({"session_id":session,"hook_event_name":"SessionStart"}))
            .into_iter()
            .find_map(|item| match item.body {
                WorkEvent::ActivityDeclared { kind, executor, .. } if kind == "host.session" => {
                    executor
                }
                _ => None,
            })
            .unwrap()
    };
    assert_ne!(executor("host-one"), executor("host-two"));
}
