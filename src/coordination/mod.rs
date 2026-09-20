// @feature coordination
// @spec docs/features/coordination.md
pub mod api;
mod classifier;
pub mod domain;
mod service;
mod workspace;

pub use service::NexusService;
