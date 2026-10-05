// @feature agent-memory
// @spec docs/features/agent-memory.md
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub const DEFAULT_MEMORY_MAX_ITEMS: usize = 64;
pub const DEFAULT_MEMORY_MAX_BYTES: usize = 16 * 1024;
pub const MAX_MEMORY_ENTRY_BYTES: usize = 512;
pub const MAX_MEMORY_SUMMARY_BYTES: usize = 2 * 1024;
pub const MAX_MEMORY_SEARCH_RESULTS: usize = 256;
pub const MAX_MEMORY_FRONTIER_OMISSIONS: usize = DEFAULT_MEMORY_MAX_ITEMS + 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum MemoryScope {
    Global,
    Project { project_id: String },
}

impl MemoryScope {
    pub fn project(project_id: impl Into<String>) -> Self {
        Self::Project {
            project_id: project_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum MemoryReadScope {
    Global,
    Project { project_id: String },
    Layered { project_id: String },
}

impl MemoryReadScope {
    pub fn project(project_id: impl Into<String>) -> Self {
        Self::Project {
            project_id: project_id.into(),
        }
    }

    pub fn layered(project_id: impl Into<String>) -> Self {
        Self::Layered {
            project_id: project_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProvenance {
    pub agent: String,
    pub session_id: String,
    pub model_id: Option<String>,
    pub config_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub scope: MemoryScope,
    pub ordinal: u64,
    pub content: String,
    pub content_hash: String,
    pub provenance: MemoryProvenance,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySummary {
    pub id: String,
    pub scope: MemoryScope,
    pub level: u32,
    pub start_ordinal: u64,
    pub end_ordinal: u64,
    pub content: String,
    pub source_hash: String,
    pub provider: String,
    pub model_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryNode {
    Raw(MemoryEntry),
    Summary(MemorySummary),
}

impl MemoryNode {
    pub fn id(&self) -> &str {
        match self {
            Self::Raw(entry) => &entry.id,
            Self::Summary(summary) => &summary.id,
        }
    }

    pub fn scope(&self) -> &MemoryScope {
        match self {
            Self::Raw(entry) => &entry.scope,
            Self::Summary(summary) => &summary.scope,
        }
    }

    pub fn start_ordinal(&self) -> u64 {
        match self {
            Self::Raw(entry) => entry.ordinal,
            Self::Summary(summary) => summary.start_ordinal,
        }
    }

    pub fn end_ordinal(&self) -> u64 {
        match self {
            Self::Raw(entry) => entry.ordinal + 1,
            Self::Summary(summary) => summary.end_ordinal,
        }
    }

    pub fn level(&self) -> u32 {
        match self {
            Self::Raw(_) => 0,
            Self::Summary(summary) => summary.level,
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Self::Raw(entry) => &entry.content,
            Self::Summary(summary) => &summary.content,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryWriteResult {
    pub entry: MemoryEntry,
    pub inserted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryContextLimits {
    pub max_items: usize,
    pub max_bytes: usize,
}

impl Default for MemoryContextLimits {
    fn default() -> Self {
        Self {
            max_items: DEFAULT_MEMORY_MAX_ITEMS,
            max_bytes: DEFAULT_MEMORY_MAX_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOmissionReason {
    MissingSummary,
    Budget,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryOmission {
    pub scope: MemoryScope,
    pub start_ordinal: u64,
    pub end_ordinal: u64,
    pub reason: MemoryOmissionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryContextStatus {
    pub scope: MemoryReadScope,
    pub nodes: Vec<MemoryNode>,
    pub omitted: Vec<MemoryOmission>,
    pub item_count: usize,
    pub byte_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryScopeHealth {
    pub scope: MemoryScope,
    pub raw_entries: i64,
    pub summaries: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryHealth {
    pub scopes: Vec<MemoryScopeHealth>,
    pub providers: Vec<MemoryProviderHealth>,
    pub pending_summaries: usize,
    pub failed_attempts: usize,
    pub cooling_down_attempts: usize,
    pub fallback_uses: usize,
    pub degraded: bool,
    pub last_activation_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProviderHealth {
    pub provider: String,
    pub model_id: Option<String>,
    pub last_outcome: MemoryCompactionOutcome,
    pub last_attempted_at: DateTime<Utc>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub cooling_down: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCompactionJob {
    pub job_id: String,
    pub space_id: String,
    pub scope: MemoryScope,
    pub level: u32,
    pub start_ordinal: u64,
    pub end_ordinal: u64,
    pub source_hash: String,
    pub children: Vec<MemoryNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCompactionAttempt {
    pub provider: String,
    pub model_id: Option<String>,
    pub outcome: MemoryCompactionOutcome,
    pub diagnostic: Option<String>,
    pub was_fallback: bool,
    pub attempted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "summary", rename_all = "snake_case")]
pub enum MemorySummaryInsert {
    Inserted(MemorySummary),
    AlreadyPresent(MemorySummary),
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCompactionOutcome {
    Succeeded,
    Failed,
    TimedOut,
    InvalidOutput,
    Stale,
}

impl MemoryCompactionOutcome {
    pub fn is_failure(self) -> bool {
        !matches!(self, Self::Succeeded | Self::Stale)
    }
}

impl std::fmt::Display for MemoryCompactionOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::InvalidOutput => "invalid_output",
            Self::Stale => "stale",
        })
    }
}

impl FromStr for MemoryCompactionOutcome {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "timed_out" => Ok(Self::TimedOut),
            "invalid_output" => Ok(Self::InvalidOutput),
            "stale" => Ok(Self::Stale),
            other => Err(format!("unknown memory compaction outcome {other}")),
        }
    }
}
