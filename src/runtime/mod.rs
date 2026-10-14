// @feature runtime
// @feature usage-analytics
// @spec docs/features/runtime.md
// @spec docs/features/usage-analytics.md
mod bounded_io;
pub mod client;
pub mod daemon;
mod dispatch;
pub mod hooks;
pub mod mcp;
pub(crate) mod protocol;
pub mod script_exec;
pub mod server;
pub mod terminal;
mod terminal_runtime;
