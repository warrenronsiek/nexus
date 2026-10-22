// @feature runtime
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
pub mod agents;
pub(crate) mod analytics;
pub mod config;
pub mod coordination;
pub mod evals;
mod git_process;
pub mod installation;
pub mod memory;
pub mod papercuts;
pub mod persistence;
mod process_capture;
pub mod runtime;
mod secret_file;
pub mod sos;

pub use config::{Config, LoadedConfig};
pub use coordination::NexusService;
