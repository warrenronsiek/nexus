// @feature papercuts
// @spec docs/features/papercuts.md
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

macro_rules! stored_enum {
    ($type:ty, $kind:literal, {$($variant:ident => $value:literal),+ $(,)?}) => {
        impl std::fmt::Display for $type {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(match self {
                    $(Self::$variant => $value),+
                })
            }
        }

        impl FromStr for $type {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    other => Err(format!("unknown {} {other}", $kind)),
                }
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PapercutActor {
    pub agent: String,
    pub session_id: String,
    pub model_id: Option<String>,
    pub config_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PapercutStatus {
    Open,
    Resolved,
    Consolidated,
}

stored_enum!(PapercutStatus, "papercut status", {
    Open => "open",
    Resolved => "resolved",
    Consolidated => "consolidated",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Papercut {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: String,
    pub status: PapercutStatus,
    pub consolidated_into: Option<String>,
    pub resolution: Option<String>,
    pub creator_agent: String,
    pub creator_session_id: String,
    pub creator_model_id: Option<String>,
    pub creator_config_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub upvote_count: u64,
    pub last_upvoted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PapercutReviewCandidate {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: String,
    pub status: PapercutStatus,
    pub consolidated_into: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub upvote_count: u64,
    pub last_upvoted_at: Option<DateTime<Utc>>,
}

impl From<&Papercut> for PapercutReviewCandidate {
    fn from(papercut: &Papercut) -> Self {
        Self {
            id: papercut.id.clone(),
            project_id: papercut.project_id.clone(),
            title: papercut.title.clone(),
            description: papercut.description.clone(),
            status: papercut.status,
            consolidated_into: papercut.consolidated_into.clone(),
            created_at: papercut.created_at,
            updated_at: papercut.updated_at,
            upvote_count: papercut.upvote_count,
            last_upvoted_at: papercut.last_upvoted_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PapercutUpvote {
    pub id: String,
    pub project_id: String,
    pub papercut_id: String,
    pub reported_papercut_id: String,
    pub reporter_agent: String,
    pub reporter_session_id: String,
    pub reporter_model_id: Option<String>,
    pub reporter_config_hash: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateReviewStatus {
    PendingReporter,
    Approved,
    Rejected,
}

stored_enum!(DuplicateReviewStatus, "duplicate review status", {
    PendingReporter => "pending_reporter",
    Approved => "approved",
    Rejected => "rejected",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateReview {
    pub id: String,
    pub project_id: String,
    pub papercut_id: String,
    pub canonical_id: String,
    pub reporter_session_id: String,
    pub rationale: String,
    pub question: String,
    pub response: Option<String>,
    pub status: DuplicateReviewStatus,
    pub reviewer_agent: String,
    pub reviewer_session_id: String,
    pub provider: Option<String>,
    pub model_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PapercutReviewAttemptOutcome {
    Distinct,
    Failed,
}

stored_enum!(PapercutReviewAttemptOutcome, "papercut review attempt outcome", {
    Distinct => "distinct",
    Failed => "failed",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PapercutReviewAttempt {
    pub id: String,
    pub project_id: String,
    pub papercut_id: String,
    pub outcome: PapercutReviewAttemptOutcome,
    pub provider: String,
    pub model_id: Option<String>,
    pub detail: String,
    pub config_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transition", rename_all = "snake_case")]
pub enum PapercutLifecycleTransition {
    Resolved {
        resolution: String,
    },
    Reopened,
    Consolidated {
        canonical_id: String,
        rationale: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PapercutLifecycleEvent {
    pub id: i64,
    pub project_id: String,
    pub papercut_id: String,
    #[serde(flatten)]
    pub transition: PapercutLifecycleTransition,
    pub actor: PapercutActor,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PapercutRegistry {
    pub generated_at: DateTime<Utc>,
    pub truncated: bool,
    pub papercuts: Vec<Papercut>,
    pub upvotes: Vec<PapercutUpvote>,
    pub duplicate_reviews: Vec<DuplicateReview>,
    pub review_attempts: Vec<PapercutReviewAttempt>,
    pub lifecycle_events: Vec<PapercutLifecycleEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PapercutReportResult {
    pub papercut: Papercut,
    pub created: bool,
    pub reporter_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PapercutReviewDecision {
    Approve,
    Reject,
}
