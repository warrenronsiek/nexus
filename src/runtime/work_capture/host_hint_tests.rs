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

#[test]
fn capture_context_preserves_handoff_precedence() {
    let handoff = host_handoff(
        "handoff-agent",
        "handoff-session",
        Some("handoff-turn"),
        Some("handoff-model"),
    );
    if let Ok(case) = std::env::var("NEXUS_TEST_CONTEXT_CASE") {
        assert_capture_context(&case, &handoff);
        return;
    }
    for case in [
        "missing",
        "malformed",
        "handoff",
        "explicit",
        "partial",
        "nulls",
        "empty",
        "non_unicode",
    ] {
        let output = context_command(case, &handoff).output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn context_command(case: &str, handoff: &WorkHandoff) -> std::process::Command {
    use std::os::unix::ffi::OsStringExt;
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "runtime::work_capture::host_hint_tests::capture_context_preserves_handoff_precedence",
            "--nocapture",
        ])
        .env("NEXUS_TEST_CONTEXT_CASE", case);
    for variable in [
        "NEXUS_HOST_SESSION_ID",
        "NEXUS_AGENT",
        "NEXUS_WORK_SESSION_ID",
        "NEXUS_PARENT_ACTIVITY_ID",
        "NEXUS_TURN_ID",
        "NEXUS_MODEL",
        "NEXUS_WORK_ENVIRONMENT",
        "NEXUS_WORK_CONTEXT",
    ] {
        command.env_remove(variable);
    }
    let value = match case {
        "missing" => None,
        "malformed" => Some("not-json".into()),
        "nulls" => Some(
            serde_json::to_string(&WorkHandoff {
                parent_activity_id: None,
                request_activity_id: None,
                turn_id: None,
                model: None,
                ..handoff.clone()
            })
            .unwrap(),
        ),
        _ => Some(serde_json::to_string(&handoff).unwrap()),
    };
    command.envs(value.map(|value| ("NEXUS_WORK_CONTEXT", value)));
    match case {
        "explicit" => {
            for (name, value) in [
                ("NEXUS_HOST_SESSION_ID", "explicit-session"),
                ("NEXUS_AGENT", "explicit-agent"),
                ("NEXUS_WORK_SESSION_ID", "explicit-work"),
                ("NEXUS_PARENT_ACTIVITY_ID", "explicit-parent"),
                ("NEXUS_TURN_ID", "explicit-turn"),
                ("NEXUS_MODEL", "explicit-model"),
            ] {
                command.env(name, value);
            }
        }
        "partial" => {
            command.env("NEXUS_AGENT", "explicit-agent");
        }
        "empty" => {
            for name in [
                "NEXUS_HOST_SESSION_ID",
                "NEXUS_AGENT",
                "NEXUS_WORK_SESSION_ID",
                "NEXUS_PARENT_ACTIVITY_ID",
                "NEXUS_TURN_ID",
                "NEXUS_MODEL",
            ] {
                command.env(name, "");
            }
        }
        "non_unicode" => {
            command
                .env(
                    "NEXUS_HOST_SESSION_ID",
                    std::ffi::OsString::from_vec(vec![255]),
                )
                .env("NEXUS_AGENT", std::ffi::OsString::from_vec(vec![255]));
        }
        _ => {}
    }
    command
}

fn assert_capture_context(case: &str, handoff: &WorkHandoff) {
    use super::CaptureContext;
    let hook = HookContext {
        session_id: "hook-session".into(),
        agent: "hook-agent".into(),
        project_root: Some("/fixture".into()),
        turn_id: None,
        model: None,
    };
    let native = CaptureContext::from_hook(hook);
    let environment = CaptureContext::from_environment(std::path::Path::new("/fixture"));
    let work = match case {
        "missing" | "malformed" => None,
        "explicit" => Some("explicit-work"),
        "empty" => Some(""),
        _ => Some(handoff.work_session_id.as_str()),
    };
    let parent = match case {
        "missing" | "malformed" | "nulls" => None,
        "explicit" => Some("explicit-parent"),
        "empty" => Some(""),
        _ => handoff.parent_activity_id.as_deref(),
    };
    for context in [&native, &environment] {
        assert_eq!(context.work_session_id.as_deref(), work);
        assert_eq!(context.parent_activity_id.as_deref(), parent);
        assert_eq!(
            context
                .work_context()
                .labels
                .get("nexus.capture.gap")
                .map(String::as_str),
            (case == "malformed").then_some("invalid_context_handoff")
        );
    }
    assert_eq!(
        (
            &*native.hook.session_id,
            &*native.hook.agent,
            native.hook.turn_id,
            native.hook.model
        ),
        ("hook-session", "hook-agent", None, None)
    );
    assert_environment_context(case, environment, handoff);
}

fn assert_environment_context(case: &str, context: super::CaptureContext, handoff: &WorkHandoff) {
    let generated = format!("command-{}", super::process_instance_id());
    let (session, agent) = match case {
        "missing" | "malformed" | "non_unicode" => (&*generated, "nexus-cli"),
        "explicit" => ("explicit-session", "explicit-agent"),
        "empty" => ("", ""),
        "partial" => (&*handoff.host_session_id, "explicit-agent"),
        _ => (&*handoff.host_session_id, &*handoff.agent),
    };
    let (turn, model) = match case {
        "missing" | "malformed" | "nulls" => (None, None),
        "explicit" => (Some("explicit-turn"), Some("explicit-model")),
        "empty" => (Some(""), Some("")),
        _ => (handoff.turn_id.as_deref(), handoff.model.as_deref()),
    };
    assert_eq!(
        (
            &*context.hook.session_id,
            &*context.hook.agent,
            context.hook.turn_id.as_deref(),
            context.hook.model.as_deref()
        ),
        (session, agent, turn, model)
    );
}

#[test]
fn active_handoff_preserves_bound_identity_and_supplied_executor() {
    use super::{CaptureContext, WorkExecutor};
    let handoff = host_handoff(
        "active-agent",
        "active-host",
        Some("active-turn"),
        Some("m"),
    );
    let mut context = CaptureContext::from_hook(HookContext {
        session_id: "bound-host".into(),
        agent: "bound-agent".into(),
        project_root: Some("/fixture".into()),
        turn_id: None,
        model: Some("explicit-model".into()),
    });
    context.work_session_id = None;
    context.parent_activity_id = Some("supplied-parent".into());
    context.environment = Some("test-environment".into());
    context.executor = Some(WorkExecutor {
        name: "executor".into(),
        model_id: Some("executor-model".into()),
        instance_id: "executor-instance".into(),
    });
    let inherited = context.with_handoff_defaults(handoff.clone());
    assert_eq!(inherited.work_session_id, Some(handoff.work_session_id));
    assert_eq!(
        inherited.parent_activity_id.as_deref(),
        Some("supplied-parent")
    );
    assert_eq!(inherited.environment.as_deref(), Some("test-environment"));
    assert_eq!(
        (
            &*inherited.hook.session_id,
            &*inherited.hook.agent,
            inherited.hook.turn_id,
            inherited.hook.model
        ),
        (
            "bound-host",
            "bound-agent",
            handoff.turn_id,
            Some("explicit-model".into())
        )
    );
    let executor = inherited.executor.unwrap();
    assert_eq!(
        (executor.name, executor.model_id, executor.instance_id),
        (
            "executor".into(),
            Some("executor-model".into()),
            "executor-instance".into()
        )
    );
}
