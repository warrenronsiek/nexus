// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
use super::*;
use crate::coordination::api::ToolHookPhase;
use crate::coordination::domain::ToolCompletion;
use crate::work_history::{WorkContext, WorkEvent, WorkOutcome, WorkProducer, WorkSubject};

pub(super) fn stable_id(parts: &[&str]) -> String {
    let mut hash = blake3::Hasher::new();
    for part in parts {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize().to_hex().to_string()
}

pub(super) fn event(subject: WorkSubject, key: &str, body: WorkEvent) -> WorkObservationInput {
    WorkObservationInput {
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

struct HostCapture {
    context: CaptureContext,
    work_id: String,
    agent_id: String,
    observations: Vec<WorkObservationInput>,
}

impl HostCapture {
    fn new(hook: &HookContext) -> Self {
        let context = CaptureContext::from_hook(hook.clone());
        let work_id = context
            .work_session_id
            .clone()
            .unwrap_or_else(|| stable_id(&["work", &hook.agent, &hook.session_id]));
        let agent_id = stable_id(&["agent", &hook.agent, &hook.session_id]);
        Self {
            context,
            work_id,
            agent_id,
            observations: Vec::new(),
        }
    }

    fn root(&mut self) {
        if self.context.work_session_id.is_none() {
            self.observations.push(event(
                WorkSubject::Session {
                    id: self.work_id.clone(),
                },
                "opened",
                WorkEvent::SessionOpened {
                    title: "Observed agent work".into(),
                    context: self.context.work_context(),
                },
            ));
        }
        let agent = WorkSubject::Activity {
            id: self.agent_id.clone(),
        };
        self.observations.push(event(
            agent.clone(),
            "declared",
            WorkEvent::ActivityDeclared {
                name: format!("{} agent", self.context.hook.agent),
                kind: "agent.task".into(),
                work_session_id: Some(self.work_id.clone()),
                parent_activity_id: self.context.parent_activity_id.clone(),
                initiator: None,
                executor: Some(self.context.hook.agent.clone()),
                context: self.context.work_context(),
            },
        ));
        self.observations
            .push(event(agent, "started", WorkEvent::Started));
    }

    fn tool(&mut self, input: &crate::coordination::domain::ToolHookInput, phase: &ToolHookPhase) {
        let hook = &self.context.hook;
        let subject = WorkSubject::Activity {
            id: stable_id(&[&hook.agent, &hook.session_id, &input.tool_use_id]),
        };
        let parent = input
            .parent_tool_use_id
            .as_deref()
            .map(|id| stable_id(&[&hook.agent, &hook.session_id, id]))
            .unwrap_or_else(|| self.agent_id.clone());
        self.observations.push(event(
            subject.clone(),
            "declared",
            WorkEvent::ActivityDeclared {
                name: input.tool_name.clone(),
                kind: "tool".into(),
                work_session_id: Some(self.work_id.clone()),
                parent_activity_id: Some(parent),
                initiator: Some(hook.agent.clone()),
                executor: None,
                context: self.context.work_context(),
            },
        ));
        let (key, body) = match phase {
            ToolHookPhase::Before => ("started", WorkEvent::Started),
            ToolHookPhase::After(completion) => (
                "finished",
                WorkEvent::Finished {
                    outcome: match completion {
                        ToolCompletion::Succeeded => WorkOutcome::Succeeded,
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

    fn prompt(&mut self, input: &crate::coordination::domain::UserPromptInput) {
        let request_id = input
            .context
            .turn_id
            .clone()
            .unwrap_or_else(|| blake3::hash(input.prompt.as_bytes()).to_hex().to_string());
        self.observations.push(event(
            WorkSubject::Activity {
                id: self.agent_id.clone(),
            },
            &format!("request-{request_id}"),
            WorkEvent::Milestone {
                name: "Human request received".into(),
                context: self.context.work_context(),
            },
        ));
    }

    fn stop(&mut self) {
        self.observations.push(event(
            WorkSubject::Activity {
                id: self.agent_id.clone(),
            },
            "finished",
            WorkEvent::Finished {
                outcome: WorkOutcome::Unknown,
                elapsed: None,
                exit_code: None,
                signal: None,
                assessment: Some("Host session ended; task success was not asserted".into()),
            },
        ));
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
