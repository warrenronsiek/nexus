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
pub mod code_concerns;
pub mod config;
pub mod coordination;
pub mod evals;
mod git_process;
mod identity;
pub mod installation;
pub mod memory;
pub mod papercuts;
pub mod persistence;
mod process_capture;
mod record_source;
pub mod runtime;
mod secret_file;
pub mod sos;
pub mod updater;
pub mod work_history;

pub use config::{Config, LoadedConfig};
pub use coordination::NexusService;

pub(crate) mod private_state;
