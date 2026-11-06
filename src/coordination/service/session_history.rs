// @feature coordination
// @spec docs/features/coordination.md
use super::NexusService;
use crate::coordination::api::{
    ServiceRequest, ServiceResponse, SessionIndexQuery, SessionIndexResponse, SessionTimelineQuery,
    SessionTimelineResponse, MAX_SESSION_INDEX_RESULTS, MAX_SESSION_TIMELINE_ENTRIES,
};
use crate::coordination::workspace::ProjectBinding;
use crate::persistence::Store;
use anyhow::{Context, Result};

impl NexusService {
    pub(super) fn session_history(
        &self,
        request: ServiceRequest,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        match request {
            ServiceRequest::SessionTimeline(query) => self.session_timeline(query, binding),
            ServiceRequest::SessionIndex(query) => self.session_index(query, binding),
            _ => unreachable!("non-session-history request routed as session history"),
        }
    }

    fn session_timeline(
        &self,
        query: SessionTimelineQuery,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        self.project_history(query.project_id, binding, |store, project_id| {
            let timeline = store.session_timeline(
                project_id,
                &query.session_id,
                MAX_SESSION_TIMELINE_ENTRIES,
            )?;
            Ok(ServiceResponse::SessionTimeline(SessionTimelineResponse {
                ok: true,
                timeline,
            }))
        })
    }

    fn session_index(&self, query: SessionIndexQuery, binding: &ProjectBinding) -> ServiceResponse {
        self.project_history(query.project_id, binding, |store, project_id| {
            let window = store.session_index(project_id, MAX_SESSION_INDEX_RESULTS)?;
            Ok(ServiceResponse::SessionIndex(SessionIndexResponse {
                ok: true,
                sessions: window.items,
                truncated: window.truncated,
            }))
        })
    }

    /// Session identity is project-qualified, so these reads never span projects.
    fn project_history(
        &self,
        project_id: Option<String>,
        binding: &ProjectBinding,
        read: impl FnOnce(&mut Store, &str) -> Result<ServiceResponse>,
    ) -> ServiceResponse {
        binding
            .default_read_project(project_id)
            .context("select a project for session coordination history")
            .and_then(|project_id| {
                let mut store = self
                    .store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
                read(&mut store, &project_id)
            })
            .unwrap_or_else(ServiceResponse::error)
    }
}
