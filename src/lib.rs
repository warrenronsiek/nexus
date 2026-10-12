// @feature runtime
// @feature agent-memory
// @feature papercuts
// @spec docs/features/runtime.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
pub mod agents;
pub mod config;
pub mod coordination;
pub mod installation;
pub mod memory;
pub mod papercuts;
pub mod persistence;
pub mod runtime;

pub use config::{Config, LoadedConfig};
pub use coordination::NexusService;
