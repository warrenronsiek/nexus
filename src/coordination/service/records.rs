// @feature coordination
// @spec docs/features/coordination.md
use super::NexusService;
use crate::coordination::api::{
    ClaimQuery, ClaimsResponse, ConflictQuery, ConflictsResponse, ServiceResponse, SessionQuery,
    SessionsResponse,
};
use crate::coordination::workspace::ProjectBinding;
use crate::persistence::Store;
use anyhow::Result;

pub(super) enum BoundedRecordQuery {
    Claims(ClaimQuery),
    Sessions(SessionQuery),
    Conflicts(ConflictQuery),
}

impl From<ClaimQuery> for BoundedRecordQuery {
    fn from(query: ClaimQuery) -> Self {
        Self::Claims(query)
    }
}

impl From<SessionQuery> for BoundedRecordQuery {
    fn from(query: SessionQuery) -> Self {
        Self::Sessions(query)
    }
}

impl From<ConflictQuery> for BoundedRecordQuery {
    fn from(query: ConflictQuery) -> Self {
        Self::Conflicts(query)
    }
}

impl BoundedRecordQuery {
    fn bind_project(&mut self, binding: &ProjectBinding) {
        let project_id = match self {
            Self::Claims(query) => &mut query.project_id,
            Self::Sessions(query) => &mut query.project_id,
            Self::Conflicts(query) => &mut query.project_id,
        };
        *project_id = binding.default_read_project(project_id.take());
    }

    fn load(self, store: &mut Store, activity_window_seconds: i64) -> Result<ServiceResponse> {
        match self {
            Self::Claims(query) => {
                let window =
                    store.list_claims_window(query.project_id.as_deref(), query.scope())?;
                Ok(ServiceResponse::Claims(ClaimsResponse {
                    ok: true,
                    claims: window.items,
                    truncated: window.truncated,
                }))
            }
            Self::Sessions(query) => {
                let window = store.list_sessions_window(
                    query.project_id.as_deref(),
                    query.scope(),
                    activity_window_seconds,
                )?;
                Ok(ServiceResponse::Sessions(SessionsResponse {
                    ok: true,
                    sessions: window.items,
                    truncated: window.truncated,
                }))
            }
            Self::Conflicts(query) => {
                let window =
                    store.list_conflicts_window(query.project_id.as_deref(), query.scope())?;
                Ok(ServiceResponse::Conflicts(ConflictsResponse {
                    ok: true,
                    conflicts: window.items,
                    truncated: window.truncated,
                }))
            }
        }
    }
}

impl NexusService {
    pub(super) fn bounded_records<Q>(&self, query: Q, binding: &ProjectBinding) -> ServiceResponse
    where
        Q: Into<BoundedRecordQuery>,
    {
        let mut query = query.into();
        query.bind_project(binding);
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| {
                query.load(
                    &mut store,
                    self.loaded.config.coordination.claim_ttl_seconds,
                )
            }) {
            Ok(response) => response,
            Err(error) => ServiceResponse::error(error),
        }
    }
}
