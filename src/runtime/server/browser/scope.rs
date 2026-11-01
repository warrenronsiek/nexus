// @feature runtime
// @feature observability-ui
// @spec docs/features/runtime.md
// @spec docs/features/observability-ui.md
use crate::analytics::types::AnalyticsCommand;
use crate::config::ClientConfig;
use crate::coordination::api::{MemoryInspection, MemoryReadScopeArg, ServiceRequest};

/// A remote agent transport defaults omitted read projects; browser global scope must be explicit.
pub(super) enum ProjectScope {
    Global,
    Remote { default_project_id: String },
}

impl ProjectScope {
    pub(super) fn for_client(client: &ClientConfig) -> Self {
        match client {
            ClientConfig::Local => Self::Global,
            ClientConfig::Remote(remote) => Self::Remote {
                default_project_id: remote.project_id.trim().to_owned(),
            },
        }
    }

    pub(super) fn default_project_id(&self) -> Option<&str> {
        match self {
            Self::Global => None,
            Self::Remote { default_project_id } => Some(default_project_id),
        }
    }

    pub(super) fn global_project_reads(&self) -> bool {
        matches!(self, Self::Global)
    }

    pub(super) fn permits(&self, request: &ServiceRequest, project: Option<&str>) -> bool {
        self.global_project_reads() || project.is_some() || independent_of_project_binding(request)
    }
}

fn independent_of_project_binding(request: &ServiceRequest) -> bool {
    matches!(
        request,
        ServiceRequest::Projects
            | ServiceRequest::Analytics(AnalyticsCommand::Categories)
            | ServiceRequest::MemoryExpand(_)
            | ServiceRequest::MemoryInspect(
                MemoryInspection::Context {
                    scope: MemoryReadScopeArg::Global,
                    ..
                } | MemoryInspection::Search {
                    scope: MemoryReadScopeArg::Global,
                    ..
                }
            )
    )
}
