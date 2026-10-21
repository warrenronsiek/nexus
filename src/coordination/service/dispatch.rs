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
// @entrypoint NexusService::handle
use super::NexusService;
use crate::coordination::api::{ConfigResponse, ServiceRequest, ServiceResponse, ToolHookPhase};
use crate::coordination::client_evidence::ClientFilesystemEvidence;
use crate::coordination::workspace::ProjectBinding;

impl NexusService {
    pub fn handle(&self, request: ServiceRequest) -> ServiceResponse {
        self.handle_bound(request, &ProjectBinding::Local, None)
    }

    pub fn handle_remote(
        &self,
        project_id: impl Into<String>,
        request: ServiceRequest,
    ) -> ServiceResponse {
        self.handle_bound(
            request,
            &ProjectBinding::Remote {
                project_id: project_id.into(),
            },
            None,
        )
    }

    pub(crate) fn handle_remote_observed(
        &self,
        project_id: impl Into<String>,
        request: ServiceRequest,
        evidence: ClientFilesystemEvidence,
    ) -> ServiceResponse {
        let project_id = project_id.into();
        let response = self.handle_bound(
            request,
            &ProjectBinding::Remote {
                project_id: project_id.clone(),
            },
            evidence.derived_usage.clone(),
        );
        let _ = self.reconcile_remote_evidence(&project_id, &evidence, false);
        response
    }

    fn handle_bound(
        &self,
        request: ServiceRequest,
        project: &ProjectBinding,
        client_evidence: Option<crate::coordination::client_evidence::ClientDerivedUsage>,
    ) -> ServiceResponse {
        match request {
            request @ (ServiceRequest::UserPrompt(_)
            | ServiceRequest::ToolHook { .. }
            | ServiceRequest::SkillUse(_)
            | ServiceRequest::ScriptUse(_)
            | ServiceRequest::SessionStop(_)) => {
                self.handle_lifecycle(request, project, client_evidence)
            }
            request @ (ServiceRequest::Status
            | ServiceRequest::Events(_)
            | ServiceRequest::Sessions(_)
            | ServiceRequest::Claims(_)
            | ServiceRequest::Conflicts(_)
            | ServiceRequest::Projects
            | ServiceRequest::Dashboard(_)
            | ServiceRequest::Usage(_)
            | ServiceRequest::MemoryStatus(_)
            | ServiceRequest::MemoryContext(_)
            | ServiceRequest::MemorySearch(_)
            | ServiceRequest::MemoryExpand(_)
            | ServiceRequest::Config) => self.handle_read(request, project),
            request @ (ServiceRequest::Resolve(_)
            | ServiceRequest::Release(_)
            | ServiceRequest::Analyze(_)
            | ServiceRequest::MemoryAdd(_)
            | ServiceRequest::MemoryInvalidate(_)
            | ServiceRequest::MemoryConsolidate
            | ServiceRequest::Papercuts(_)
            | ServiceRequest::Analytics(_)) => self.handle_command(request, project),
            ServiceRequest::Sos(_) => self.handle_command(request, project),
        }
    }

    fn handle_lifecycle(
        &self,
        request: ServiceRequest,
        project: &ProjectBinding,
        client_evidence: Option<crate::coordination::client_evidence::ClientDerivedUsage>,
    ) -> ServiceResponse {
        match request {
            ServiceRequest::UserPrompt(input) => self.user_prompt(input, project),
            ServiceRequest::ToolHook {
                phase: ToolHookPhase::Before,
                input,
            } => self.pre_tool_use(input, project, client_evidence),
            ServiceRequest::ToolHook {
                phase: ToolHookPhase::After(completion),
                input,
            } => self.post_tool_use(input, completion, project, client_evidence),
            ServiceRequest::SkillUse(input) => self.explicit_usage(input, project),
            ServiceRequest::ScriptUse(input) => self.explicit_usage(input, project),
            ServiceRequest::SessionStop(input) => self.session_stop(input, project),
            _ => unreachable!("non-lifecycle request routed as lifecycle"),
        }
    }

    fn handle_read(&self, request: ServiceRequest, project: &ProjectBinding) -> ServiceResponse {
        match request {
            ServiceRequest::Status => self.status(),
            ServiceRequest::Events(query) => self.events(query, project),
            ServiceRequest::Sessions(query) => self.bounded_records(query, project),
            ServiceRequest::Claims(query) => self.bounded_records(query, project),
            ServiceRequest::Conflicts(query) => self.bounded_records(query, project),
            ServiceRequest::Projects => self.projects(),
            ServiceRequest::Dashboard(query) => self.dashboard(query, project),
            ServiceRequest::Usage(query) => self.usage(query, project),
            ServiceRequest::MemoryStatus(query) => self.memory_status(query, project),
            ServiceRequest::MemoryContext(query) => self.memory_context(query, project),
            ServiceRequest::MemorySearch(query) => self.memory_search(query, project),
            ServiceRequest::MemoryExpand(query) => self.memory_expand(query),
            ServiceRequest::Config => ServiceResponse::Config(Box::new(ConfigResponse {
                ok: true,
                config: self.loaded.config.clone(),
                sources: self.loaded.sources.clone(),
                hash: self.loaded.hash.clone(),
            })),
            _ => unreachable!("non-read request routed as read"),
        }
    }

    fn handle_command(&self, request: ServiceRequest, project: &ProjectBinding) -> ServiceResponse {
        match request {
            ServiceRequest::Resolve(command) => self.resolve(command),
            ServiceRequest::Release(command) => self.release(command, project),
            ServiceRequest::Analyze(command) => self.analyze(command, project),
            ServiceRequest::MemoryAdd(command) => self.memory_add(command, project),
            ServiceRequest::MemoryInvalidate(command) => self.memory_invalidate(command),
            ServiceRequest::MemoryConsolidate => self.memory_consolidate(),
            ServiceRequest::Papercuts(command) => {
                self.dispatch_papercuts(command, project).response
            }
            ServiceRequest::Sos(command) => self.dispatch_sos(command, project).response,
            ServiceRequest::Analytics(command) => self.analytics_command(command, project),
            _ => unreachable!("non-command request routed as command"),
        }
    }
}
