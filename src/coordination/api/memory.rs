// @feature agent-memory
// @feature coordination
// @spec docs/features/agent-memory.md
// @spec docs/features/coordination.md
use crate::coordination::domain::HookContext;
use crate::memory::{
    MemoryContextStatus, MemoryEntry, MemoryHealth, MemoryNode, MemoryWriteResult,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScopeArg {
    Global,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryReadScopeArg {
    #[default]
    Layered,
    Global,
    Project,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MemoryStatusQuery {
    #[serde(default)]
    pub project_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MemoryContextQuery {
    #[serde(default)]
    pub scope: MemoryReadScopeArg,
    #[serde(default)]
    pub project_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryAddCommand {
    #[serde(flatten)]
    pub context: HookContext,
    pub scope: MemoryScopeArg,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySearchQuery {
    #[serde(default)]
    pub scope: MemoryReadScopeArg,
    #[serde(default)]
    pub project_root: Option<String>,
    pub regex: String,
    #[serde(default = "default_memory_search_limit")]
    pub limit: usize,
}

impl MemorySearchQuery {
    pub fn bounded_limit(&self) -> usize {
        self.limit
            .clamp(1, crate::memory::MAX_MEMORY_SEARCH_RESULTS)
    }
}

fn default_memory_search_limit() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySummaryQuery {
    pub summary_id: String,
}

#[derive(Debug, Serialize)]
pub struct MemoryWriteResponse {
    pub ok: bool,
    #[serde(flatten)]
    pub result: MemoryWriteResult,
}

#[derive(Debug, Serialize)]
pub struct MemoryEntriesResponse {
    pub ok: bool,
    pub memories: Vec<MemoryEntry>,
}

#[derive(Debug, Serialize)]
pub struct MemoryContextResponse {
    pub ok: bool,
    pub context: MemoryContextStatus,
}

#[derive(Debug, Serialize)]
pub struct MemoryNodesResponse {
    pub ok: bool,
    pub nodes: Vec<MemoryNode>,
}

#[derive(Debug, Serialize)]
pub struct MemoryInvalidationResponse {
    pub ok: bool,
    pub invalidated_summaries: usize,
}

#[derive(Debug, Serialize)]
pub struct MemoryStatusResponse {
    pub ok: bool,
    pub health: MemoryHealth,
}

#[derive(Debug, Default, Serialize)]
pub struct MemoryConsolidationReport {
    pub ok: bool,
    pub in_progress: bool,
    pub attempted: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub stale: usize,
    pub fallback_uses: usize,
}
