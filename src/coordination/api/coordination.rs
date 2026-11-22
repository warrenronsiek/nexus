// @feature coordination
// @feature usage-analytics
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
use super::*;
use crate::coordination::domain::{ClaimRelease, ConflictScope, RecordScope};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EventQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default = "default_event_limit")]
    pub limit: usize,
}

impl EventQuery {
    pub fn bounded_limit(&self) -> usize {
        self.limit.clamp(1, MAX_EVENT_RESULTS)
    }
}

fn default_event_limit() -> usize {
    MAX_EVENT_RESULTS
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SessionQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub scope: RecordScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

impl SessionQuery {
    pub fn scope(&self) -> RecordScope {
        self.scope
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ClaimQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub scope: RecordScope,
}

impl ClaimQuery {
    pub fn scope(&self) -> RecordScope {
        self.scope
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConflictQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub scope: ConflictScope,
}

impl ConflictQuery {
    pub fn scope(&self) -> ConflictScope {
        self.scope
    }
}

/// One session's coordination history inside one project.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionTimelineQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    pub session_id: String,
}

/// Sessions ranked by their most recent coordination activity inside one project.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SessionIndexQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveCommand {
    pub conflict_id: String,
    #[serde(default = "default_resolution")]
    pub resolution: String,
}

fn default_resolution() -> String {
    "acknowledged".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseCommand {
    pub session_id: String,
    #[serde(default)]
    pub path: Option<String>,
}

impl ReleaseCommand {
    pub fn release(&self) -> ClaimRelease {
        self.path
            .clone()
            .map(ClaimRelease::Path)
            .unwrap_or(ClaimRelease::Session)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeCommand {
    pub conflict_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DashboardQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UsageQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<crate::persistence::CapabilityKind>,
}
