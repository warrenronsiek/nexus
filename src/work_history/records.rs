// @feature work-history
// @spec docs/features/work-history.md
// @boundary work-labels: caller names and bounded text metadata describe observed work without becoming executable instructions.
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkSubject {
    Session { id: String },
    Activity { id: String },
}
impl WorkSubject {
    pub fn id(&self) -> &str {
        match self {
            Self::Session { id } | Self::Activity { id } => id,
        }
    }
    pub fn scope(&self) -> &'static str {
        match self {
            Self::Session { .. } => "session",
            Self::Activity { .. } => "activity",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkActor {
    pub agent: String,
    pub session_id: String,
    pub model_id: Option<String>,
    pub config_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkProducer {
    pub name: String,
    pub version: String,
    pub instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitRevision {
    pub repository: String,
    pub object_format: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalIdentity {
    pub provider: String,
    pub kind: String,
    pub id: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WorkContext {
    pub host_session_id: Option<String>,
    pub turn_id: Option<String>,
    pub agent_instance_id: Option<String>,
    pub model_id: Option<String>,
    pub repository: Option<String>,
    pub worktree: Option<String>,
    pub environment: Option<String>,
    pub revision: Option<GitRevision>,
    pub external: Vec<ExternalIdentity>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Elapsed {
    pub nanoseconds: String,
    pub method: String,
    pub clock_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkOutcome {
    Succeeded,
    Failed,
    Canceled,
    TimedOut,
    Skipped,
    InfrastructureError,
    Abandoned,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkRelation {
    Parent,
    MemberOf,
    DependsOn,
    RetryOf,
    Delegates,
    Evidence,
    Produced,
    Deployed,
}
impl WorkRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::MemberOf => "member_of",
            Self::DependsOn => "depends_on",
            Self::RetryOf => "retry_of",
            Self::Delegates => "delegates",
            Self::Evidence => "evidence",
            Self::Produced => "produced",
            Self::Deployed => "deployed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkReference {
    Session {
        id: String,
    },
    Activity {
        id: String,
    },
    Eval {
        id: String,
    },
    Revision {
        repository: String,
        object_format: String,
        commit: String,
    },
    External {
        provider: String,
        #[serde(rename = "type")]
        kind: String,
        id: String,
        url: Option<String>,
    },
}
impl WorkReference {
    pub fn subject(&self) -> Option<WorkSubject> {
        match self {
            Self::Session { id } => Some(WorkSubject::Session { id: id.clone() }),
            Self::Activity { id } => Some(WorkSubject::Activity { id: id.clone() }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkEvent {
    SessionOpened {
        title: String,
        #[serde(default)]
        context: WorkContext,
    },
    ActivityDeclared {
        name: String,
        kind: String,
        work_session_id: Option<String>,
        parent_activity_id: Option<String>,
        initiator: Option<String>,
        executor: Option<String>,
        #[serde(default)]
        context: WorkContext,
    },
    Queued,
    Started,
    Finished {
        outcome: WorkOutcome,
        elapsed: Option<Elapsed>,
        exit_code: Option<i32>,
        signal: Option<i32>,
        assessment: Option<String>,
    },
    Milestone {
        name: String,
        #[serde(default)]
        context: WorkContext,
    },
    Linked {
        relation: WorkRelation,
        target: WorkReference,
    },
}
impl WorkEvent {
    pub fn family(&self) -> &'static str {
        match self {
            Self::SessionOpened { .. } => "session_opened",
            Self::ActivityDeclared { .. } => "activity_declared",
            Self::Queued => "queued",
            Self::Started => "started",
            Self::Finished { .. } => "finished",
            Self::Milestone { .. } => "milestone",
            Self::Linked { .. } => "linked",
        }
    }
    pub fn context(&self) -> Option<&WorkContext> {
        match self {
            Self::SessionOpened { context, .. }
            | Self::ActivityDeclared { context, .. }
            | Self::Milestone { context, .. } => Some(context),
            _ => None,
        }
    }
    pub fn references(&self) -> Vec<(WorkRelation, WorkReference)> {
        match self {
            Self::ActivityDeclared {
                work_session_id,
                parent_activity_id,
                ..
            } => {
                let mut links = Vec::new();
                if let Some(id) = work_session_id {
                    links.push((
                        WorkRelation::MemberOf,
                        WorkReference::Session { id: id.clone() },
                    ));
                }
                if let Some(id) = parent_activity_id {
                    links.push((
                        WorkRelation::Parent,
                        WorkReference::Activity { id: id.clone() },
                    ));
                }
                links
            }
            Self::Linked { relation, target } => vec![(*relation, target.clone())],
            _ => vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkCorrection {
    pub replaces: String,
    pub reason: String,
}

/// An observed OS or provider clock domain. Independent producers may report the same
/// source clock; this is never a Rust Instant borrowed from another process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkSourceClock {
    pub clock_id: String,
    pub nanoseconds: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkObservationInput {
    pub id: String,
    pub subject: WorkSubject,
    pub occurred_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_clock: Option<WorkSourceClock>,
    pub producer: WorkProducer,
    pub method: String,
    pub body: WorkEvent,
    #[serde(default)]
    pub correction: Option<WorkCorrection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkObservation {
    #[serde(flatten)]
    pub input: WorkObservationInput,
    pub project_id: String,
    pub recorded_at: DateTime<Utc>,
    pub provenance: WorkActor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<WorkSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkSource {
    pub project_id: String,
    pub recorded_at: DateTime<Utc>,
    pub provenance: WorkActor,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkLink {
    pub event_id: String,
    pub source: WorkSubject,
    pub relation: String,
    pub target: WorkReference,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkSummary {
    pub subject: WorkSubject,
    pub title: String,
    pub kind: String,
    pub work_session_ids: Vec<String>,
    pub parent_activity_id: Option<String>,
    pub has_children: bool,
    pub executor: Option<String>,
    pub state: String,
    pub outcome: Option<WorkOutcome>,
    pub queued_at: Option<DateTime<Utc>>,
    pub queue_elapsed: Option<Elapsed>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub elapsed: Option<Elapsed>,
    pub conflicts: Vec<String>,
    pub gaps: Vec<String>,
    pub context: WorkContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkDetail {
    pub summary: WorkSummary,
    pub observations: Vec<WorkObservation>,
    pub links: Vec<WorkLink>,
    pub truncated: bool,
}

impl WorkSummary {
    pub(crate) fn unobserved(subject: &WorkSubject) -> Self {
        Self {
            subject: subject.clone(),
            title: subject.id().into(),
            kind: subject.scope().into(),
            work_session_ids: vec![],
            parent_activity_id: None,
            has_children: false,
            executor: None,
            state: "unknown".into(),
            outcome: None,
            queued_at: None,
            queue_elapsed: None,
            started_at: None,
            finished_at: None,
            elapsed: None,
            conflicts: vec![],
            gaps: vec![],
            context: WorkContext::default(),
        }
    }
    pub(crate) fn disclose_limit(&mut self) {
        self.gaps.push("projection_limit".into());
        self.state = "unknown".into();
        self.outcome = None;
        self.queued_at = None;
        self.queue_elapsed = None;
        self.started_at = None;
        self.finished_at = None;
        self.elapsed = None;
    }
    pub(crate) fn normalize_diagnostics(&mut self) {
        self.gaps.sort();
        self.gaps.dedup();
        self.conflicts.sort();
        self.conflicts.dedup();
    }
}
