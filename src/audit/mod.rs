// @feature audit-registry
// @spec docs/features/audit-registry.md
// @entrypoint AuditAction
mod validation;
pub(crate) use validation::identity;

use crate::delivery::DeliverySubject;
use crate::work_history::{GitRevision, WorkActor, WorkSubject};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_RECORD_BYTES: usize = 48 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuditAction {
    Append {
        record: Box<AuditInput>,
        expected_project: Option<String>,
    },
    Control {
        control: ControlVersion,
        project_id: Option<String>,
    },
    List {
        #[serde(default)]
        filter: RunFilter,
        project_id: Option<String>,
    },
    Get {
        run_id: String,
        after: Option<i64>,
        cutoff: Option<i64>,
        project_id: Option<String>,
    },
    Journal {
        after: Option<i64>,
        cutoff: Option<i64>,
        project_id: Option<String>,
        control_key: Option<String>,
    },
}

impl AuditAction {
    pub fn read_project(&self) -> Option<&str> {
        match self {
            Self::Append { .. } => None,
            Self::Control { project_id, .. }
            | Self::List { project_id, .. }
            | Self::Get { project_id, .. }
            | Self::Journal { project_id, .. } => project_id.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RunFilter {
    pub control_key: Option<String>,
    pub version: Option<i32>,
    pub run_ids: Vec<String>,
    pub from: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub after: Option<i64>,
    pub cutoff: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlVersion {
    pub key: String,
    pub version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlDefinition {
    pub owner: String,
    pub objective: String,
    pub scope: String,
    pub trigger: String,
    pub procedure: String,
    pub effective_at: DateTime<Utc>,
    pub reason: String,
    pub results: ResultSchema,
    #[serde(default)]
    pub finding_categories: Vec<Category>,
    #[serde(default)]
    pub framework_references: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResultSchema {
    Narrative,
    Categorical { codes: Vec<Category> },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub code: String,
    pub label: String,
    pub order: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditInput {
    pub id: String,
    pub occurred_at: DateTime<Utc>,
    pub event: AuditEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuditEvent {
    Define {
        control: ControlVersion,
        previous: Option<String>,
        definition: ControlDefinition,
    },
    Retire {
        control: ControlVersion,
        previous: String,
        reason: String,
        effective_at: DateTime<Utc>,
    },
    Run {
        control: ControlVersion,
        run: AuditRun,
    },
    CorrectRun {
        run_id: String,
        previous: String,
        reason: String,
        result: RunResult,
    },
    Disposition {
        run_id: String,
        finding_id: Option<String>,
        previous: Option<String>,
        disposition: Disposition,
        reason: String,
        #[serde(default)]
        evidence: Vec<AuditEvidence>,
    },
    Verification {
        run_id: String,
        disposition_id: String,
        verifier: String,
        procedure: String,
        outcome: VerificationOutcome,
        reason: String,
        #[serde(default)]
        evidence: Vec<AuditEvidence>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditRun {
    pub subject: AuditSubject,
    pub executor: String,
    pub duration_ms: Option<u64>,
    pub retry_of: Option<String>,
    pub result: RunResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuditSubject {
    External {
        source: String,
        id: String,
    },
    Git {
        revision: GitRevision,
    },
    StagedDiff {
        repository: String,
        base: Option<GitRevision>,
        sha256: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunResult {
    pub status: RunStatus,
    pub output: Option<RunOutput>,
    pub summary: Option<String>,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub evidence: Vec<AuditEvidence>,
    #[serde(default)]
    pub review_passes: Vec<ReviewPass>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Partial,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RunOutput {
    Narrative(String),
    Code(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub description: String,
    pub category: Option<String>,
    pub recommendation: Option<String>,
    #[serde(default)]
    pub evidence: Vec<AuditEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewPass {
    pub perspective: String,
    pub provider: String,
    pub model: Option<String>,
    pub instructions_sha256: Option<String>,
    pub outcome: ReviewPassOutcome,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPassOutcome {
    Pass,
    ChangesRequested,
    InvalidOutput,
    ExecutorError,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuditEvidence {
    Git {
        revision: GitRevision,
        description: String,
    },
    Work {
        subject: WorkSubject,
        description: String,
    },
    Delivery {
        subject: DeliverySubject,
        description: String,
    },
    StoredBlob {
        digest: String,
        description: String,
    },
    External {
        uri: String,
        description: String,
    },
    Unavailable {
        reason: String,
    },
}

impl AuditEvent {
    pub fn evidence_lists(&self) -> impl Iterator<Item = &[AuditEvidence]> {
        let result = match self {
            Self::Run { run, .. } => Some(&run.result),
            Self::CorrectRun { result, .. } => Some(result),
            _ => None,
        };
        let direct = match self {
            Self::Disposition { evidence, .. } | Self::Verification { evidence, .. } => {
                Some(evidence.as_slice())
            }
            _ => result.map(|result| result.evidence.as_slice()),
        };
        direct.into_iter().chain(
            result
                .into_iter()
                .flat_map(|result| &result.findings)
                .map(|finding| finding.evidence.as_slice()),
        )
    }

    pub fn redact_evidence(&mut self) {
        let result = match self {
            Self::Run { run, .. } => &mut run.result,
            Self::CorrectRun { result, .. } => result,
            Self::Disposition { evidence, .. } | Self::Verification { evidence, .. } => {
                redact(evidence);
                return;
            }
            _ => return,
        };
        redact(&mut result.evidence);
        for finding in &mut result.findings {
            redact(&mut finding.evidence);
        }
    }
}

fn redact(evidence: &mut [AuditEvidence]) {
    for item in evidence {
        *item = AuditEvidence::Unavailable {
            reason: "Redacted in export".into(),
        };
    }
}

#[derive(
    Debug, Clone, Copy, Eq, Ord, PartialOrd, PartialEq, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Remediated,
    RiskAccepted,
    Dismissed,
    Reopened,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    Confirmed,
    Failed,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub ordinal: i64,
    pub recorded_at: DateTime<Utc>,
    pub actor: WorkActor,
    pub input: AuditInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: String,
    pub ordinal: i64,
    pub control: ControlVersion,
    pub occurred_at: DateTime<Utc>,
    pub subject: AuditSubject,
    pub status: RunStatus,
    pub duration_ms: Option<u64>,
    pub result_code: Option<String>,
    pub result_label: Option<String>,
    pub result_order: Option<i32>,
    pub findings: usize,
    pub open_findings: usize,
    pub finding_dispositions: std::collections::BTreeMap<Disposition, usize>,
    pub disposition: Option<Disposition>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum AuditResponse {
    Appended {
        project_id: String,
        record: Box<AuditRecord>,
    },
    Control {
        project_id: String,
        definition: Option<Box<AuditRecord>>,
        retired: bool,
    },
    Page {
        project_id: String,
        cutoff: i64,
        total: i64,
        items: Vec<RunSummary>,
        next_cursor: Option<i64>,
    },
    Detail {
        project_id: String,
        cutoff: i64,
        definition: Box<AuditRecord>,
        records: Vec<AuditRecord>,
        next_cursor: Option<i64>,
    },
    Journal {
        project_id: String,
        cutoff: i64,
        records: Vec<AuditRecord>,
        next_cursor: Option<i64>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AuditRejection {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
}
