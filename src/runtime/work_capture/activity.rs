// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
use super::*;
use crate::work_history::{
    Elapsed, WorkAccountingRole, WorkContext, WorkEvent, WorkOutcome, WorkSubject,
};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct CaptureContext {
    pub hook: HookContext,
    pub work_session_id: Option<String>,
    pub parent_activity_id: Option<String>,
    pub executor: Option<WorkExecutor>,
    pub environment: Option<String>,
    pub accounting_role: WorkAccountingRole,
    context_gap: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkExecutor {
    pub name: String,
    pub model_id: Option<String>,
    pub instance_id: String,
}

impl CaptureContext {
    pub fn from_environment(root: &Path) -> Self {
        let mut context = Self::from_hook(HookContext {
            session_id: std::env::var("NEXUS_HOST_SESSION_ID")
                .unwrap_or_else(|_| format!("command-{}", process_instance_id())),
            agent: std::env::var("NEXUS_AGENT").unwrap_or_else(|_| "nexus-cli".into()),
            project_root: Some(root.to_string_lossy().into_owned()),
            turn_id: std::env::var("NEXUS_TURN_ID").ok(),
            model: std::env::var("NEXUS_MODEL").ok(),
        });
        if let Ok(value) = std::env::var("NEXUS_WORK_CONTEXT") {
            if let Ok(handoff) = super::WorkHandoff::decode(value.as_bytes()) {
                handoff.apply_defaults(&mut context);
            }
        }
        context
    }

    pub fn from_hook(hook: HookContext) -> Self {
        let mut context = Self {
            hook,
            work_session_id: std::env::var("NEXUS_WORK_SESSION_ID").ok(),
            parent_activity_id: std::env::var("NEXUS_PARENT_ACTIVITY_ID").ok(),
            executor: None,
            environment: std::env::var("NEXUS_WORK_ENVIRONMENT").ok(),
            accounting_role: WorkAccountingRole::Execution,
            context_gap: None,
        };
        if let Ok(value) = std::env::var("NEXUS_WORK_CONTEXT") {
            match super::WorkHandoff::decode(value.as_bytes()) {
                Ok(handoff) => {
                    context.work_session_id =
                        context.work_session_id.or(Some(handoff.work_session_id));
                    context.parent_activity_id =
                        context.parent_activity_id.or(handoff.parent_activity_id);
                }
                Err(_) => context.context_gap = Some("invalid_context_handoff".into()),
            }
        }
        context
    }

    pub fn with_active_host(mut self, loaded: &LoadedConfig) -> Self {
        // Only an exact caller-supplied host identity can consult this project bridge.
        if std::env::var_os("NEXUS_HOST_SESSION_ID").is_some()
            && std::env::var_os("NEXUS_AGENT").is_some()
        {
            if let Ok(active) =
                super::active_host_context(loaded, &self.hook.agent, &self.hook.session_id)
            {
                active.apply_defaults(&mut self);
            }
        }
        self
    }

    pub(super) fn work_context(&self) -> WorkContext {
        let mut context = lifecycle::context_from_hook(&self.hook);
        context.environment = self.environment.clone();
        let role = match self.accounting_role {
            WorkAccountingRole::Container => "container",
            WorkAccountingRole::Execution => "execution",
            WorkAccountingRole::Wait => "wait",
            WorkAccountingRole::Unknown => "unknown",
        };
        context
            .labels
            .insert("nexus.accounting.role".into(), role.into());
        if let Some(gap) = &self.context_gap {
            context
                .labels
                .insert("nexus.capture.gap".into(), gap.clone());
        }
        if let Some(executor) = &self.executor {
            context.model_id = executor.model_id.clone();
            context.agent_instance_id = Some(executor.instance_id.clone());
        }
        context
    }

    pub fn apply(&self, command: &mut Command) {
        if let Some(environment) = &self.environment {
            command.env("NEXUS_WORK_ENVIRONMENT", environment);
        }
        if let Some(id) = &self.work_session_id {
            command.env("NEXUS_WORK_SESSION_ID", id);
        }
        if let Some(id) = &self.parent_activity_id {
            command.env("NEXUS_PARENT_ACTIVITY_ID", id);
        }
        let executor = self.executor.as_ref();
        let actor = executor
            .map(|value| value.name.as_str())
            .unwrap_or(&self.hook.agent);
        let session = executor
            .map(|value| value.instance_id.as_str())
            .unwrap_or(&self.hook.session_id);
        command
            .env("NEXUS_HOST_SESSION_ID", session)
            .env("NEXUS_AGENT", actor);
        if let Some(id) = &self.hook.turn_id {
            command.env("NEXUS_TURN_ID", id);
        }
        let model = executor
            .map(|value| value.model_id.as_ref())
            .unwrap_or(self.hook.model.as_ref());
        match model {
            Some(model) => {
                command.env("NEXUS_MODEL", model);
            }
            None => {
                command.env_remove("NEXUS_MODEL");
            }
        }
    }
}

pub struct ActivityCapture {
    loaded: LoadedConfig,
    context: CaptureContext,
    pub id: String,
    started: Instant,
}

impl ActivityCapture {
    pub fn begin(
        loaded: &LoadedConfig,
        mut context: CaptureContext,
        kind: &str,
        name: &str,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let subject = WorkSubject::Activity { id: id.clone() };
        let mut observations = Vec::new();
        if context.work_session_id.is_none() {
            let work_id = stable_id(&["work", &context.hook.agent, &context.hook.session_id]);
            observations.push(lifecycle::opened_root(&context, &work_id));
            context.work_session_id = Some(work_id);
        }
        let executor = context
            .executor
            .as_ref()
            .map(|executor| executor.instance_id.clone())
            .unwrap_or_else(|| id.clone());
        let mut activity_context = context.work_context();
        let executor_name = context
            .executor
            .as_ref()
            .map(|executor| executor.name.as_str())
            .unwrap_or(&context.hook.agent);
        activity_context
            .labels
            .insert("nexus.executor.name".into(), executor_name.into());
        observations.extend([
            lifecycle::event(
                subject.clone(),
                "declared",
                WorkEvent::ActivityDeclared {
                    name: name.into(),
                    kind: kind.into(),
                    work_session_id: context.work_session_id.clone(),
                    parent_activity_id: context.parent_activity_id.clone(),
                    initiator: Some(context.hook.agent.clone()),
                    executor: Some(executor),
                    context: activity_context,
                },
            ),
            lifecycle::event(subject, "started", WorkEvent::Started),
        ]);
        let _ = enqueue(loaded, &context.hook, observations);
        Self {
            loaded: loaded.clone(),
            context,
            id,
            started: Instant::now(),
        }
    }

    pub fn child_context(&self) -> CaptureContext {
        let mut context = self.context.clone();
        context.parent_activity_id = Some(self.id.clone());
        context
    }

    pub fn child_with_executor(&self, kind: &str, name: &str, executor: WorkExecutor) -> Self {
        let mut context = self.child_context();
        context.accounting_role = WorkAccountingRole::Execution;
        context.executor = Some(executor);
        Self::begin(&self.loaded, context, kind, name)
    }

    pub fn finish(
        self,
        outcome: WorkOutcome,
        exit_code: Option<i32>,
        signal: Option<i32>,
        assessment: Option<String>,
    ) {
        let elapsed = Elapsed {
            nanoseconds: self.started.elapsed().as_nanos().to_string(),
            method: "rust_instant_elapsed".into(),
            clock_id: Some(process_instance_id().into()),
        };
        let event = lifecycle::event(
            WorkSubject::Activity { id: self.id },
            "finished",
            WorkEvent::Finished {
                outcome,
                elapsed: Some(elapsed),
                exit_code,
                signal,
                assessment,
            },
        );
        let _ = enqueue(&self.loaded, &self.context.hook, vec![event]);
    }
}
