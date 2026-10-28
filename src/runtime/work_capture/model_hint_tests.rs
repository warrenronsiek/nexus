// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary native-model-hint-fixtures: exercise absent host metadata without inventing model identities.
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
