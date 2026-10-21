// @feature papercuts
// @feature coordination
// @spec docs/features/papercuts.md
// @spec docs/features/coordination.md
use crate::coordination::domain::HookContext;
use crate::papercuts::{
    DuplicateReview, Papercut, PapercutRegistry, PapercutReviewDecision, PapercutUpvote,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PapercutListCursor {
    pub updated_at: DateTime<Utc>,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PapercutDumpScope {
    #[default]
    Project,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PapercutCommand {
    List {
        #[serde(flatten)]
        context: HookContext,
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<PapercutListCursor>,
    },
    Dump {
        #[serde(flatten)]
        context: HookContext,
        #[serde(default)]
        scope: PapercutDumpScope,
    },
    Report {
        #[serde(flatten)]
        context: HookContext,
        title: String,
        #[serde(default)]
        description: String,
    },
    Upvote {
        #[serde(flatten)]
        context: HookContext,
        papercut_id: String,
        #[serde(default)]
        note: Option<String>,
    },
    Remove {
        #[serde(flatten)]
        context: HookContext,
        papercut_id: String,
        resolution: String,
    },
    Respond {
        #[serde(flatten)]
        context: HookContext,
        review_id: String,
        reporter_token: String,
        decision: PapercutReviewDecision,
        response: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PapercutResponse {
    Listed {
        ok: bool,
        papercuts: Vec<Papercut>,
        truncated: bool,
    },
    Dumped {
        ok: bool,
        registry: PapercutRegistry,
    },
    Reported {
        ok: bool,
        created: bool,
        papercut: Papercut,
        reporter_token: Option<String>,
        review_requested: bool,
    },
    Upvoted {
        ok: bool,
        papercut: Box<Papercut>,
        upvote: PapercutUpvote,
    },
    Removed {
        ok: bool,
        papercut: Papercut,
    },
    Responded {
        ok: bool,
        review: DuplicateReview,
    },
}
