// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
use super::*;
use crate::coordination::api::ToolHookPhase;
use crate::coordination::domain::ToolCompletion;
use crate::work_history::{WorkContext, WorkEvent, WorkOutcome, WorkProducer, WorkSubject};

pub(super) fn stable_id(parts: &[&str]) -> String {
    crate::work_history::stable_work_id(parts)
}

pub(super) fn event(subject: WorkSubject, key: &str, body: WorkEvent) -> WorkObservationInput {
    WorkObservationInput {
        source_clock: super::source_clock::observe(),
        id: stable_id(&[subject.id(), key]),
        subject,
        occurred_at: chrono::Utc::now(),
        producer: WorkProducer {
            name: "nexus-companion".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            instance_id: process_instance_id().into(),
        },
        method: "companion_callback_wall_clock".into(),
        body,
        correction: None,
    }
}

pub(super) fn root_context(context: &CaptureContext) -> WorkContext {
    let mut source = context.work_context();
    source.turn_id = None;
    source.model_id = None;
    source
        .labels
        .insert("nexus.accounting.role".into(), "container".into());
    source
        .labels
        .insert("nexus.capture.adapter".into(), "host-hooks-v1".into());
    source
        .labels
        .insert("nexus.capture.host".into(), context.hook.agent.clone());
    source
}

pub(super) fn opened_root(context: &CaptureContext, id: &str) -> WorkObservationInput {
    event(
        WorkSubject::Session { id: id.into() },
        "opened",
        WorkEvent::SessionOpened {
            title: format!("{} session {}", context.hook.agent, context.hook.session_id),
            context: root_context(context),
        },
    )
}

pub(super) struct HostCapture {
    pub(super) context: CaptureContext,
    pub(super) work_id: String,
    pub(super) agent_id: String,
    pub(super) observations: Vec<WorkObservationInput>,
}

impl HostCapture {
    pub(super) fn new(hook: &HookContext) -> Self {
        Self::with_context(CaptureContext::from_hook(hook.clone()))
    }

    pub(super) fn with_context(context: CaptureContext) -> Self {
        let hook = &context.hook;
        let work_id = context.work_session_id.clone().unwrap_or_else(|| {
            crate::work_history::host_work_identity(&hook.agent, &hook.session_id)
        });
        let agent_id = crate::work_history::host_agent_identity(&hook.agent, &hook.session_id);
        Self {
            context,
            work_id,
            agent_id,
            observations: Vec::new(),
        }
    }

    pub(super) fn root(&mut self) {
        let context = root_context(&self.context);
        if self.context.work_session_id.is_none() {
            self.observations
                .push(opened_root(&self.context, &self.work_id));
        }
        let agent = WorkSubject::Activity {
            id: self.agent_id.clone(),
        };
        self.observations.push(event(
            agent.clone(),
            "declared",
            WorkEvent::ActivityDeclared {
                name: format!("{} agent", self.context.hook.agent),
                kind: "host.session".into(),
                work_session_id: Some(self.work_id.clone()),
                parent_activity_id: self.context.parent_activity_id.clone(),
                initiator: None,
                executor: Some(self.agent_id.clone()),
                context,
            },
        ));
        self.observations
            .push(event(agent, "started", WorkEvent::Started));
    }

    pub(super) fn tool(
        &mut self,
        input: &crate::coordination::domain::ToolHookInput,
        phase: &ToolHookPhase,
    ) {
        let hook = &self.context.hook;
        let subject = WorkSubject::Activity {
            id: stable_id(&[&hook.agent, &hook.session_id, &input.tool_use_id]),
        };
        let parent = input
            .parent_tool_use_id
            .as_deref()
            .map(|id| stable_id(&[&hook.agent, &hook.session_id, id]))
            .or_else(|| {
                input
                    .agent_id
                    .as_deref()
                    .map(|id| stable_id(&["child", &hook.agent, &hook.session_id, id]))
            })
            .unwrap_or_else(|| self.turn_parent());
        self.observations.push(event(
            subject.clone(),
            "declared",
            WorkEvent::ActivityDeclared {
                name: input.tool_name.clone(),
                kind: "tool".into(),
                work_session_id: Some(self.work_id.clone()),
                parent_activity_id: Some(parent),
                initiator: Some(hook.agent.clone()),
                executor: input
                    .agent_id
                    .clone()
                    .or_else(|| Some(self.agent_id.clone())),
                context: self.role_context("execution"),
            },
        ));
        let (key, body) = match phase {
            ToolHookPhase::Before => ("started", WorkEvent::Started),
            ToolHookPhase::After(completion) => (
                "finished",
                WorkEvent::Finished {
                    outcome: match completion {
                        ToolCompletion::Succeeded => WorkOutcome::Succeeded,
                        ToolCompletion::Failed if input.is_interrupt == Some(true) => {
                            WorkOutcome::Canceled
                        }
                        ToolCompletion::Failed => WorkOutcome::Failed,
                    },
                    elapsed: None,
                    exit_code: None,
                    signal: None,
                    assessment: None,
                },
            ),
        };
        self.observations.push(event(subject, key, body));
    }

    fn prompt(&mut self, _input: &crate::coordination::domain::UserPromptInput) {
        self.request_start(None);
    }

    pub(super) fn role_context(&self, role: &str) -> WorkContext {
        let mut context = self.context.work_context();
        context
            .labels
            .insert("nexus.accounting.role".into(), role.into());
        context
    }

    pub(super) fn turn_parent(&self) -> String {
        self.context.hook.turn_id.as_deref().map_or_else(
            || self.agent_id.clone(),
            |turn| {
                stable_id(&[
                    "turn",
                    &self.context.hook.agent,
                    &self.context.hook.session_id,
                    turn,
                ])
            },
        )
    }

    pub(super) fn request_id(&self, request: Option<&str>) -> Option<String> {
        request.or(self.context.hook.turn_id.as_deref()).map(|id| {
            stable_id(&[
                "request",
                &self.context.hook.agent,
                &self.context.hook.session_id,
                id,
            ])
        })
    }

    pub(super) fn request_start(&mut self, request: Option<&str>) {
        let Some(request_id) = self.request_id(request) else {
            let mut context = self.context.work_context();
            context.labels.insert(
                "nexus.capture.gap".into(),
                "missing_request_identity".into(),
            );
            self.observations.push(event(
                WorkSubject::Activity {
                    id: self.agent_id.clone(),
                },
                &format!("missing-request-{}", uuid::Uuid::new_v4()),
                WorkEvent::Milestone {
                    name: "Request boundary unavailable".into(),
                    context,
                },
            ));
            return;
        };
        self.declare_started(
            request_id.clone(),
            "Human request",
            "request",
            self.agent_id.clone(),
            None,
        );
        if self.context.hook.turn_id.is_some() {
            self.declare_started(
                self.turn_parent(),
                "Host turn",
                "host.turn",
                request_id,
                None,
            );
        }
    }

    pub(super) fn declare_started(
        &mut self,
        id: String,
        name: &str,
        kind: &str,
        parent: String,
        executor: Option<String>,
    ) {
        let subject = WorkSubject::Activity { id };
        self.observations.push(event(
            subject.clone(),
            "declared",
            WorkEvent::ActivityDeclared {
                name: name.into(),
                kind: kind.into(),
                work_session_id: Some(self.work_id.clone()),
                parent_activity_id: Some(parent),
                initiator: Some(self.context.hook.agent.clone()),
                executor,
                context: self.role_context("container"),
            },
        ));
        self.observations
            .push(event(subject, "started", WorkEvent::Started));
    }

    pub(super) fn finished(
        &mut self,
        subject: WorkSubject,
        outcome: WorkOutcome,
        assessment: &str,
    ) {
        self.observations.push(event(
            subject,
            "finished",
            WorkEvent::Finished {
                outcome,
                elapsed: None,
                exit_code: None,
                signal: None,
                assessment: Some(assessment.into()),
            },
        ));
    }

    pub(super) fn request_stop(&mut self, request: Option<&str>, outcome: WorkOutcome) {
        if self.context.hook.turn_id.is_some() {
            self.finished(
                WorkSubject::Activity {
                    id: self.turn_parent(),
                },
                outcome,
                "Observed host response boundary; logical work and children may continue",
            );
        }
        if let Some(id) = self.request_id(request) {
            self.finished(
                WorkSubject::Activity { id },
                outcome,
                "Observed request response boundary; task success was not asserted",
            );
        }
    }

    pub(super) fn stop(&mut self) {
        self.finished(
            WorkSubject::Activity {
                id: self.agent_id.clone(),
            },
            WorkOutcome::Unknown,
            "Host session ended; task success was not asserted",
        );
        if self.context.work_session_id.is_none() {
            self.finished(
                WorkSubject::Session {
                    id: self.work_id.clone(),
                },
                WorkOutcome::Unknown,
                "Automatically host-owned root ended with its host; task success was not asserted",
            );
        }
    }
}

pub(crate) fn lifecycle_command(request: &ServiceRequest) -> Option<WorkCommand> {
    let hook = match request {
        ServiceRequest::UserPrompt(input) => &input.context,
        ServiceRequest::ToolHook { input, .. } => &input.context,
        ServiceRequest::SessionStop(input) => &input.context,
        _ => return None,
    };
    let mut capture = HostCapture::new(hook);
    capture.root();
    match request {
        ServiceRequest::UserPrompt(input) => capture.prompt(input),
        ServiceRequest::ToolHook { input, phase } => capture.tool(input, phase),
        ServiceRequest::SessionStop(_) => capture.stop(),
        _ => {}
    }
    Some(WorkCommand {
        context: hook.clone(),
        command: WorkAction::Record {
            observations: capture.observations,
        },
    })
}

pub(super) fn context_from_hook(hook: &HookContext) -> WorkContext {
    WorkContext {
        host_session_id: Some(hook.session_id.clone()),
        turn_id: hook.turn_id.clone(),
        agent_instance_id: Some(format!("{}:{}", hook.agent, hook.session_id)),
        model_id: hook.model.clone(),
        worktree: hook.project_root.clone(),
        ..WorkContext::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_tool_completion_is_canceled_with_the_original_tool_identity() {
        let request=ServiceRequest::decode("post_tool_failure",serde_json::json!({"session_id":"s","agent":"claude","turn_id":"p","tool_use_id":"tool-1","tool_name":"Bash","is_interrupt":true})).unwrap();
        let command = lifecycle_command(&request).unwrap();
        let WorkAction::Record { observations } = command.command else {
            panic!("record")
        };
        let id = stable_id(&["claude", "s", "tool-1"]);
        assert!(observations.iter().any(|o| o.subject.id() == id
            && matches!(
                o.body,
                WorkEvent::Finished {
                    outcome: WorkOutcome::Canceled,
                    ..
                }
            )));
    }
    #[test]
    fn native_child_tool_uses_the_reported_child_as_its_parent() {
        let request=ServiceRequest::decode("pre_tool_use",serde_json::json!({"session_id":"s","agent":"claude","turn_id":"p","agent_id":"child","tool_use_id":"tool-1","tool_name":"Bash"})).unwrap();
        let WorkAction::Record { observations } = lifecycle_command(&request).unwrap().command
        else {
            panic!("record")
        };
        let expected = stable_id(&["child", "claude", "s", "child"]);
        assert!(observations.iter().any(|o|matches!(&o.body,WorkEvent::ActivityDeclared {kind,parent_activity_id,..} if kind=="tool" && parent_activity_id.as_deref()==Some(expected.as_str()))));
    }
}
