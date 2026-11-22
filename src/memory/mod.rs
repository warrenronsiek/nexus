// @feature agent-memory
// @spec docs/features/agent-memory.md
pub mod domain;
pub(crate) mod tree;

pub use domain::*;

pub(crate) const NO_DURABLE_KNOWLEDGE: &str = "No durable knowledge in this range.";
