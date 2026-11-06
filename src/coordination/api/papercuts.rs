// @feature papercuts
// @feature coordination
// @spec docs/features/papercuts.md
// @spec docs/features/coordination.md
use crate::coordination::domain::HookContext;
use crate::papercuts::{
    DuplicateReview, Papercut, PapercutDetail, PapercutRegistry, PapercutReviewDecision,
    PapercutUpvote,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Keyset position in the open list, which ranks by encounter count, then recent activity, then id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PapercutListCursor {
    pub upvote_count: u64,
    pub updated_at: DateTime<Utc>,
    pub id: String,
}

impl PapercutListCursor {
    /// The cursor that continues after the given listed papercut.
    pub fn after(papercut: &Papercut) -> Self {
        Self {
            upvote_count: papercut.upvote_count,
            updated_at: papercut.updated_at,
            id: papercut.id.clone(),
        }
    }
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
    Get {
        #[serde(flatten)]
        context: HookContext,
        #[serde(default)]
        project_id: Option<String>,
        papercut_id: String,
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
    Detail {
        ok: bool,
        #[serde(flatten)]
        detail: PapercutDetail,
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
