// @feature code-concerns
// @spec docs/features/code-concerns.md
use super::*;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConcernSort {
    #[default]
    Recent,
    WorkSessions,
    Returns,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConcernStatus {
    Open,
    Resolved,
    Consolidated,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConcernQueryStatus {
    Open,
    Resolved,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ConcernQuery {
    pub repository: Option<String>,
    pub area_id: Option<String>,
    pub status: Option<ConcernQueryStatus>,
    pub search: Option<String>,
    pub occurred_after: Option<DateTime<Utc>>,
    pub occurred_before: Option<DateTime<Utc>>,
    pub sort: ConcernSort,
    pub after: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConcernAction {
    Record {
        observations: Vec<ConcernObservationInput>,
    },
    Import {
        observations: Vec<ConcernObservation>,
    },
    Respond {
        observation: Box<ConcernObservationInput>,
        reporter_token: String,
    },
    List {
        project_id: Option<String>,
        #[serde(default)]
        query: ConcernQuery,
    },
    Dashboard {
        project_id: Option<String>,
        #[serde(default)]
        query: ConcernQuery,
    },
    Get {
        project_id: Option<String>,
        concern_id: String,
    },
    History {
        project_id: Option<String>,
        concern_id: String,
        after: Option<String>,
        limit: Option<u32>,
    },
    Export {
        project_id: Option<String>,
        after: Option<String>,
        limit: Option<u32>,
    },
    Dump {
        project_id: Option<String>,
    },
}
impl ConcernAction {
    pub fn read_project(&self) -> Option<&str> {
        match self {
            Self::Record { .. } | Self::Import { .. } | Self::Respond { .. } => None,
            Self::List { project_id, .. }
            | Self::Dashboard { project_id, .. }
            | Self::Get { project_id, .. }
            | Self::History { project_id, .. }
            | Self::Export { project_id, .. }
            | Self::Dump { project_id } => project_id.as_deref(),
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ConcernRejection {
    #[error("invalid code concern: {0}")]
    Invalid(String),
    #[error("immutable code concern conflict: {0}")]
    Conflict(String),
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernCounts {
    pub raw_observations: usize,
    pub known_source_occurrences: usize,
    pub work_sessions: usize,
    pub host_sessions: usize,
    pub unknown_source_observations: usize,
    pub unknown_work_observations: usize,
    pub unknown_host_observations: usize,
    pub unknown_source_time_observations: usize,
    pub concerns: usize,
    pub returns_after_resolution: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernSummary {
    pub id: String,
    pub canonical_id: String,
    pub title: String,
    pub description: String,
    pub hypothesis: Option<String>,
    pub impact: Option<String>,
    pub status: ConcernStatus,
    pub counts: ConcernCounts,
    pub last_observed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub lifecycle_qualification: Option<String>,
    pub areas: Vec<ConcernArea>,
    pub areas_partial: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernAreaSummary {
    pub area: ConcernArea,
    pub counts: ConcernCounts,
    pub open_concern_count: usize,
    pub last_observed_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernDaily {
    pub date: String,
    pub raw_observations: usize,
    pub known_source_occurrences: usize,
    pub work_sessions: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernDetail {
    pub summary: ConcernSummary,
    pub history: Vec<ConcernObservation>,
    pub session_titles: Vec<ConcernSessionTitle>,
    pub next_cursor: Option<String>,
    pub partial: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernSessionTitle {
    pub agent: String,
    pub host_session_id: Option<String>,
    pub work_session_id: Option<String>,
    pub host_title: Option<String>,
    pub work_title: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernReceipt {
    pub id: String,
    pub concern_id: String,
    pub canonical_id: String,
    pub recorded_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reporter_token: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "response", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConcernResponse {
    Recorded {
        inserted: usize,
        replayed: usize,
        receipts: Vec<ConcernReceipt>,
    },
    Page {
        project_id: String,
        query: ConcernQuery,
        items: Vec<ConcernSummary>,
        next_cursor: Option<String>,
        total: usize,
        partial: bool,
    },
    Dashboard {
        project_id: String,
        query: ConcernQuery,
        counts: ConcernCounts,
        areas: Vec<ConcernAreaSummary>,
        daily: Vec<ConcernDaily>,
        next_cursor: Option<String>,
        partial: bool,
        notices: Vec<String>,
    },
    Detail {
        concern: Option<Box<ConcernDetail>>,
    },
    History {
        concern_id: String,
        observations: Vec<ConcernObservation>,
        session_titles: Vec<ConcernSessionTitle>,
        next_cursor: Option<String>,
    },
    Export {
        observations: Vec<ConcernObservation>,
        next_cursor: Option<String>,
    },
    Dump {
        project_id: String,
        observations: Vec<ConcernObservation>,
    },
}
