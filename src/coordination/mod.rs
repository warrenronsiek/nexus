// @feature coordination
// @feature sos
// @spec docs/features/coordination.md
// @spec docs/features/sos.md
pub mod api;
mod classifier;
pub(crate) mod client_evidence;
pub mod domain;
mod service;
mod usage;
pub(crate) mod usage_detection;
mod workspace;

pub use service::NexusService;
pub(crate) use service::{PapercutDispatch, PapercutReviewJob, SosDispatch, SosTriageJob};
pub(crate) use usage::derive_filesystem_evidence;
pub(crate) use workspace::ProjectBinding;

/// Reuse the service's canonical local project binding for private runtime state.
pub(crate) fn local_project_id(root: Option<&str>) -> anyhow::Result<String> {
    workspace::identify(&workspace::ProjectBinding::Local, root).map(|identity| identity.id)
}
