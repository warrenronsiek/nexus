// @feature work-history
// @spec docs/features/work-history.md
use super::*;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WorkQuery {
    pub repository: Option<String>,
    pub host_session_id: Option<String>,
    pub kind: Option<String>,
    pub outcome: Option<WorkOutcome>,
    pub search: Option<String>,
    pub environment: Option<String>,
    pub occurred_after: Option<DateTime<Utc>>,
    pub occurred_before: Option<DateTime<Utc>>,
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkRejection {
    #[error("invalid work observation: {0}")]
    Invalid(String),
    #[error("immutable work observation conflict: {0}")]
    Conflict(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkAction {
    Record {
        observations: Vec<WorkObservationInput>,
    },
    Import {
        observations: Vec<WorkObservation>,
    },
    Export {
        project_id: Option<String>,
        after: Option<String>,
        limit: Option<u32>,
    },
    List {
        project_id: Option<String>,
        #[serde(default)]
        query: WorkQuery,
    },
    Activities {
        project_id: Option<String>,
        #[serde(default)]
        query: WorkQuery,
    },
    Dashboard {
        project_id: Option<String>,
        work_session_id: Option<String>,
    },
    Get {
        project_id: Option<String>,
        subject: WorkSubject,
    },
    Timeline {
        project_id: Option<String>,
        work_session_id: String,
        parent_activity_id: Option<String>,
        after: Option<String>,
        limit: Option<u32>,
    },
    Events {
        project_id: Option<String>,
        subject: WorkSubject,
        after: Option<String>,
        limit: Option<u32>,
    },
}
impl WorkAction {
    pub fn read_project(&self) -> Option<&str> {
        match self {
            Self::Record { .. } | Self::Import { .. } => None,
            Self::List { project_id, .. }
            | Self::Activities { project_id, .. }
            | Self::Dashboard { project_id, .. }
            | Self::Get { project_id, .. }
            | Self::Timeline { project_id, .. }
            | Self::Events { project_id, .. }
            | Self::Export { project_id, .. } => project_id.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkReceipt {
    pub id: String,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "response", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkResponse {
    Dashboard {
        #[serde(flatten)]
        dashboard: Box<WorkDashboard>,
    },
    Recorded {
        inserted: usize,
        replayed: usize,
        receipts: Vec<WorkReceipt>,
    },
    Page {
        items: Vec<WorkSummary>,
        next_cursor: Option<String>,
    },
    Detail {
        record: Option<Box<WorkDetail>>,
    },
    Timeline {
        work_session_id: String,
        parent_activity_id: Option<String>,
        items: Vec<WorkSummary>,
        next_cursor: Option<String>,
        unresolved_links: Vec<WorkLink>,
        truncated: bool,
    },
    Events {
        observations: Vec<WorkObservation>,
        next_cursor: Option<String>,
    },
}
