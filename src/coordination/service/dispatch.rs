// @feature coordination
// @feature usage-analytics
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @entrypoint NexusService::handle
use super::NexusService;
use crate::coordination::api::{ConfigResponse, ServiceRequest, ServiceResponse, ToolHookPhase};

impl NexusService {
    pub fn handle(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            request @ (ServiceRequest::UserPrompt(_)
            | ServiceRequest::ToolHook { .. }
            | ServiceRequest::SkillUse(_)
            | ServiceRequest::ScriptUse(_)
            | ServiceRequest::SessionStop(_)) => self.handle_lifecycle(request),
            request @ (ServiceRequest::Status
            | ServiceRequest::Events(_)
            | ServiceRequest::Sessions(_)
            | ServiceRequest::Claims(_)
            | ServiceRequest::Conflicts(_)
            | ServiceRequest::Projects
            | ServiceRequest::Dashboard(_)
            | ServiceRequest::Usage(_)
            | ServiceRequest::Config) => self.handle_read(request),
            request @ (ServiceRequest::Resolve(_)
            | ServiceRequest::Release(_)
            | ServiceRequest::Analyze(_)) => self.handle_command(request),
        }
    }

    fn handle_lifecycle(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::UserPrompt(input) => self.user_prompt(input),
            ServiceRequest::ToolHook {
                phase: ToolHookPhase::Before,
                input,
            } => self.pre_tool_use(input),
            ServiceRequest::ToolHook {
                phase: ToolHookPhase::After(completion),
                input,
            } => self.post_tool_use(input, completion),
            ServiceRequest::SkillUse(input) => self.skill_use(input),
            ServiceRequest::ScriptUse(input) => self.script_use(input),
            ServiceRequest::SessionStop(input) => self.session_stop(input),
            _ => unreachable!("non-lifecycle request routed as lifecycle"),
        }
    }

    fn handle_read(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::Status => self.status(),
            ServiceRequest::Events(query) => self.events(query),
            ServiceRequest::Sessions(query) => self.sessions(query),
            ServiceRequest::Claims(query) => self.claims(query),
            ServiceRequest::Conflicts(query) => self.conflicts(query),
            ServiceRequest::Projects => self.projects(),
            ServiceRequest::Dashboard(query) => self.dashboard(query),
            ServiceRequest::Usage(query) => self.usage(query),
            ServiceRequest::Config => ServiceResponse::Config(Box::new(ConfigResponse {
                ok: true,
                config: self.loaded.config.clone(),
                sources: self.loaded.sources.clone(),
                hash: self.loaded.hash.clone(),
            })),
            _ => unreachable!("non-read request routed as read"),
        }
    }

    fn handle_command(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::Resolve(command) => self.resolve(command),
            ServiceRequest::Release(command) => self.release(command),
            ServiceRequest::Analyze(command) => self.analyze(command),
            _ => unreachable!("non-command request routed as command"),
        }
    }
}
