// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary native-host-hint-fixtures: exercise absent host metadata without inventing model identities.
use super::{host_event, host_handoff, WorkHandoff};
use crate::coordination::domain::HookContext;
use crate::work_history::WorkContext;
use serde_json::json;

#[test]
fn unknown_model_hints_remain_absent_at_host_and_record_boundaries() {
    for model in ["", "   "] {
        let hook: HookContext = serde_json::from_value(json!({
            "session_id":"model-fixture", "agent":"claude", "model":model
        }))
        .unwrap();
        assert!(hook.model.is_none(), "an empty host model is unknown");
        let context: WorkContext = serde_json::from_value(json!({"model_id":model})).unwrap();
        assert!(context.model_id.is_none());
    }
    let context: WorkContext =
        serde_json::from_value(json!({"model_id":"observed-model"})).unwrap();
    assert_eq!(context.model_id.as_deref(), Some("observed-model"));
    assert!(serde_json::from_value::<WorkContext>(json!({"model_id":{}})).is_err());
}

#[test]
fn unknown_model_hint_does_not_reject_native_work_events() {
    let command = host_event(
        "claude",
        &serde_json::to_vec(&json!({
            "session_id":"model-fixture", "hook_event_name":"SessionStart", "model":""
        }))
        .unwrap(),
    )
    .unwrap();
    if let crate::work_history::WorkAction::Record { observations } = command.command {
        assert!(!observations.is_empty());
        for observation in observations {
            observation.validate().unwrap();
        }
    } else {
        panic!("native work event must record observations");
    }
}

#[test]
fn unknown_model_hint_in_retained_handoff_preserves_exact_identity() {
    let original = host_handoff("claude", "model-fixture", Some("turn-1"), Some(""));
    let decoded = WorkHandoff::decode(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert!(decoded.model.is_none());
    assert_eq!(decoded.work_session_id, original.work_session_id);
    assert_eq!(decoded.request_activity_id, original.request_activity_id);
    assert_eq!(decoded.parent_activity_id, original.parent_activity_id);
}

#[test]
fn empty_agent_hint_preserves_root_tool_parent_and_executor() {
    use crate::coordination::api::ServiceRequest;
    use crate::work_history::{WorkAction, WorkEvent};
    for agent_hint in ["", "   "] {
        let request = ServiceRequest::decode(
            "pre_tool_use",
            json!({
                "session_id":"root-fixture", "agent":"claude", "turn_id":"turn-1",
                "agent_id":agent_hint, "tool_use_id":"tool-1", "tool_name":"Bash"
            }),
        )
        .unwrap();
        let WorkAction::Record { observations } =
            super::prepare_lifecycle(&request).unwrap().command
        else {
            panic!("record")
        };
        let declaration = observations
            .iter()
            .find(|o| matches!(&o.body, WorkEvent::ActivityDeclared { kind, .. } if kind=="tool"))
            .unwrap();
        declaration
            .validate()
            .expect("optional empty child identity must not invalidate root tool capture");
        if let WorkEvent::ActivityDeclared {
            parent_activity_id,
            executor,
            ..
        } = &declaration.body
        {
            assert_eq!(
                parent_activity_id.as_deref(),
                Some(
                    super::lifecycle::stable_id(&["turn", "claude", "root-fixture", "turn-1"])
                        .as_str()
                )
            );
            assert_eq!(
                executor.as_deref(),
                Some(crate::work_history::host_agent_identity("claude", "root-fixture").as_str())
            );
        }
    }
}

#[test]
fn empty_executor_hint_remains_unknown_without_weakening_parent_identity() {
    use crate::work_history::{WorkEvent, WorkObservationInput};
    let original = super::lifecycle::event(
        crate::work_history::WorkSubject::Activity {
            id: "tool-fixture".into(),
        },
        "declared",
        WorkEvent::ActivityDeclared {
            name: "fixture tool".into(),
            kind: "tool".into(),
            work_session_id: Some("work-fixture".into()),
            parent_activity_id: Some("parent-fixture".into()),
            initiator: Some("claude".into()),
            executor: Some("".into()),
            context: WorkContext::default(),
        },
    );
    let bytes = serde_json::to_vec(&original).unwrap();
    let decoded: WorkObservationInput = serde_json::from_slice(&bytes).unwrap();
    decoded
        .validate()
        .expect("an empty optional executor is unknown");
    assert_eq!(decoded.id, original.id);
    assert_eq!(decoded.subject, original.subject);
    assert_eq!(decoded.occurred_at, original.occurred_at);
    if let WorkEvent::ActivityDeclared {
        executor,
        parent_activity_id,
        ..
    } = &decoded.body
    {
        assert!(executor.is_none());
        assert_eq!(parent_activity_id.as_deref(), Some("parent-fixture"));
    }
    let mut invalid = serde_json::to_value(&original).unwrap();
    invalid["body"]["parent_activity_id"] = json!("");
    assert!(serde_json::from_value::<WorkObservationInput>(invalid)
        .unwrap()
        .validate()
        .is_err());
}
