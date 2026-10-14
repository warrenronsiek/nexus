// @feature sos
// @feature coordination
// @spec docs/features/sos.md
// @spec docs/features/coordination.md
// @boundary dynamic-json
use crate::coordination::domain::HookContext;
use crate::sos::{SosIssue, SosIssueDetail, SosMetadata, SosRegistry, SosReport};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SosDumpScope {
    #[default]
    Project,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SosCommand {
    Report {
        #[serde(flatten)]
        context: HookContext,
        message: String,
        #[serde(rename = "type")]
        kind: String,
        environment: String,
        version: String,
        trace: String,
        #[serde(default)]
        metadata: SosMetadata,
    },
    List {
        #[serde(flatten)]
        context: HookContext,
    },
    Get {
        #[serde(flatten)]
        context: HookContext,
        issue_id: String,
    },
    Dump {
        #[serde(flatten)]
        context: HookContext,
        #[serde(default)]
        scope: SosDumpScope,
    },
    Resolve {
        #[serde(flatten)]
        context: HookContext,
        issue_id: String,
        resolution: String,
    },
    Deprecate {
        #[serde(flatten)]
        context: HookContext,
        issue_id: String,
        reason: String,
    },
}

impl SosCommand {
    pub(super) fn context(&self) -> &HookContext {
        match self {
            Self::Report { context, .. }
            | Self::List { context }
            | Self::Get { context, .. }
            | Self::Dump { context, .. }
            | Self::Resolve { context, .. }
            | Self::Deprecate { context, .. } => context,
        }
    }

    pub fn is_complete_dump(&self) -> bool {
        matches!(self, Self::Dump { .. })
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SosResponse {
    Reported {
        ok: bool,
        report: SosReport,
        triage_requested: bool,
    },
    Listed {
        ok: bool,
        issues: Vec<SosIssue>,
        untriaged_report_count: u64,
        truncated: bool,
    },
    Detailed {
        ok: bool,
        detail: SosIssueDetail,
    },
    Dumped {
        ok: bool,
        registry: SosRegistry,
    },
    Resolved {
        ok: bool,
        issue: SosIssue,
    },
    Deprecated {
        ok: bool,
        issue: SosIssue,
    },
}
