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
mod usage_detection;
mod workspace;

pub use service::NexusService;
pub(crate) use service::{PapercutDispatch, PapercutReviewJob, SosDispatch, SosTriageJob};
pub(crate) use workspace::ProjectBinding;
