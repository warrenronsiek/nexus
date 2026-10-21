// @feature coordination
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @boundary serialized-service-response
use super::memory::{
    MemoryConsolidationReport, MemoryContextResponse, MemoryEntriesResponse,
    MemoryInvalidationResponse, MemoryNodesResponse, MemoryStatusResponse, MemoryWriteResponse,
};
use super::papercuts::PapercutResponse;
use super::sos::SosResponse;
use crate::agents::analyst::AnalysisResult;
use crate::config::Config;
use crate::coordination::domain::{
    Claim, ConflictRecord, DashboardRecords, EventRecord, HookResponse, ProjectSummary,
    SessionRecord, StatusCounts,
};
use crate::memory::MemoryContextStatus;
use crate::persistence::UsageSummary;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ServiceResponse {
    Hook(HookResponse),
    Prompt(PromptResponse),
    Status(StatusResponse),
    Events(EventsResponse),
    Sessions(SessionsResponse),
    Claims(ClaimsResponse),
    Conflicts(ConflictsResponse),
    Resolved(ResolvedResponse),
    Released(ReleasedResponse),
    Analysis(AnalysisResponse),
    Projects(ProjectsResponse),
    Dashboard(DashboardResponse),
    Usage(UsageResponse),
    MemoryWrite(MemoryWriteResponse),
    MemoryEntries(MemoryEntriesResponse),
    MemoryContext(MemoryContextResponse),
    MemoryNodes(MemoryNodesResponse),
    MemoryInvalidation(MemoryInvalidationResponse),
    MemoryStatus(MemoryStatusResponse),
    MemoryConsolidation(MemoryConsolidationReport),
    Papercuts(Box<PapercutResponse>),
    Sos(Box<SosResponse>),
    Analytics(crate::analytics::types::AnalyticsResponse),
    Config(Box<ConfigResponse>),
    Error(ErrorResponse),
}

impl ServiceResponse {
    pub fn error(error: impl std::fmt::Display) -> Self {
        Self::Error(ErrorResponse {
            ok: false,
            error: error.to_string(),
        })
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("service responses are serializable")
    }
}

#[derive(Debug, Serialize)]
pub struct PromptResponse {
    pub permitted: bool,
    pub recorded: bool,
    pub project_id: String,
    pub prompt_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_context: Option<MemoryContextStatus>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub ok: bool,
    pub status: &'static str,
    pub counts: StatusCounts,
    pub config_hash: String,
}

#[derive(Debug, Serialize)]
pub struct EventsResponse {
    pub ok: bool,
    pub events: Vec<EventRecord>,
}

#[derive(Debug, Serialize)]
pub struct SessionsResponse {
    pub ok: bool,
    pub sessions: Vec<SessionRecord>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ClaimsResponse {
    pub ok: bool,
    pub claims: Vec<Claim>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ConflictsResponse {
    pub ok: bool,
    pub conflicts: Vec<ConflictRecord>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ResolvedResponse {
    pub ok: bool,
    pub conflict_id: String,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ReleasedResponse {
    pub ok: bool,
    pub released: usize,
}

#[derive(Debug, Serialize)]
pub struct AnalysisResponse {
    pub ok: bool,
    pub analysis: AnalysisResult,
}

#[derive(Debug, Serialize)]
pub struct ProjectsResponse {
    pub ok: bool,
    pub projects: Vec<ProjectSummary>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct DashboardResponse {
    pub ok: bool,
    pub project_id: Option<String>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub refresh_interval_ms: u64,
    #[serde(flatten)]
    pub records: DashboardRecords,
}

#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub ok: bool,
    pub project_id: Option<String>,
    #[serde(flatten)]
    pub summary: UsageSummary,
    pub capture_health: CaptureHealthResponse,
}

#[derive(Debug, Serialize)]
pub struct CaptureHealthResponse {
    pub label: &'static str,
    pub received: u64,
    pub recorded: u64,
    pub dropped: u64,
}

#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub ok: bool,
    pub config: Config,
    pub sources: Vec<PathBuf>,
    pub hash: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub ok: bool,
    pub error: String,
}
