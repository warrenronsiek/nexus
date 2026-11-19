// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
use super::{environment::*, origin::*, remediation::*};
use crate::work_history::{GitRevision, WorkActor};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryKind {
    Contribution,
    PullRequest,
    Rollout,
    Deployment,
    Remediation,
    ReleaseRequirements,
    EnvironmentInventory,
    EnvironmentAdvisory,
}
impl DeliveryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Contribution => "contribution",
            Self::PullRequest => "pull_request",
            Self::Rollout => "rollout",
            Self::Deployment => "deployment",
            Self::Remediation => "remediation",
            Self::ReleaseRequirements => "release_requirements",
            Self::EnvironmentInventory => "environment_inventory",
            Self::EnvironmentAdvisory => "environment_advisory",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliverySubject {
    pub kind: DeliveryKind,
    pub id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentity {
    pub agent: String,
    pub session_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentRoute {
    pub host: String,
    pub thread_id: String,
    pub url: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Reachability {
    Available,
    Unavailable,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPresence {
    pub agent: AgentIdentity,
    pub reachability: Reachability,
    pub route: Option<AgentRoute>,
    pub observed_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub id: String,
    pub repository: String,
    pub title: String,
    pub origin: AgentIdentity,
    pub work_session_id: Option<String>,
    pub activity_id: Option<String>,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub base_revision: Option<GitRevision>,
    pub head_revision: Option<GitRevision>,
    #[serde(default)]
    pub features: Vec<String>,
    pub rationale: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    pub route: Option<AgentRoute>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MappingOperation {
    Commit,
    Merge,
    Squash,
    Rebase,
    CherryPick,
    ConflictResolution,
    Revert,
    Manual,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MappingVerification {
    Supplied,
    MechanicallyVerified,
    ModelSuggested,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangedHunk {
    #[serde(default)]
    pub fingerprint: Option<String>,
    pub path: String,
    pub old_path: Option<String>,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionMapping {
    pub id: String,
    pub contribution_id: String,
    pub revision: GitRevision,
    #[serde(default)]
    pub source_revisions: Vec<GitRevision>,
    pub operation: MappingOperation,
    pub verification: MappingVerification,
    #[serde(default)]
    pub hunks: Vec<ChangedHunk>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullRequest {
    pub id: String,
    pub repository: String,
    pub provider: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub base: GitRevision,
    pub head: GitRevision,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
    #[serde(default)]
    pub contribution_ids: Vec<String>,
    pub rollout_id: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestState {
    Open,
    Merged,
    Closed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Skipped,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliveryCheck {
    pub id: String,
    pub name: String,
    pub state: CheckState,
    pub revision: GitRevision,
    pub url: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullRequestSnapshot {
    pub observed_at: DateTime<Utc>,
    pub state: PullRequestState,
    pub base: GitRevision,
    pub head: GitRevision,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
    #[serde(default)]
    pub checks: Vec<DeliveryCheck>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rollout {
    pub id: String,
    pub repository: String,
    pub title: String,
    #[serde(default)]
    pub pull_request_ids: Vec<String>,
    #[serde(default)]
    pub contribution_ids: Vec<String>,
    #[serde(default)]
    pub environments: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentOutcome {
    Running,
    Succeeded,
    Failed,
    Canceled,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub id: String,
    pub repository: String,
    pub rollout_id: String,
    pub environment: String,
    pub revision: GitRevision,
    pub artifact_digest: Option<String>,
    pub attempt: u32,
    pub outcome: DeploymentOutcome,
    pub activity_id: Option<String>,
    pub external_url: Option<String>,
    pub rollback_of: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemediationState {
    Requested,
    Acknowledged,
    InProgress,
    Resolved,
    Declined,
    Unreachable,
    Canceled,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Remediation {
    pub id: String,
    pub repository: String,
    pub title: String,
    pub recipient: AgentIdentity,
    pub pull_request_id: Option<String>,
    pub rollout_id: Option<String>,
    pub contribution_id: Option<String>,
    pub deployment_id: Option<String>,
    pub concern_id: Option<String>,
    pub activity_id: Option<String>,
    pub failure_revision: Option<GitRevision>,
    pub evidence: Vec<String>,
    pub request: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_evidence: Option<RemediationOrigin>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryEvent {
    ContributionRegistered {
        contribution: Contribution,
    },
    RevisionMapped {
        mapping: RevisionMapping,
    },
    PullRequestRegistered {
        pull_request: PullRequest,
    },
    PullRequestObserved {
        pull_request_id: String,
        snapshot: PullRequestSnapshot,
    },
    PullRequestCompositionRecorded {
        pull_request_id: String,
        head: GitRevision,
        expected_composition: Option<String>,
        contribution_ids: Vec<String>,
        reason: String,
    },
    RolloutRegistered {
        rollout: Rollout,
    },
    DeploymentRecorded {
        deployment: Deployment,
    },
    DeploymentFinished {
        deployment_id: String,
        expected_outcome: DeploymentOutcome,
        outcome: DeploymentOutcome,
    },
    ShepherdAssigned {
        subject: DeliverySubject,
        expected_assignment: Option<String>,
        agent: Option<AgentIdentity>,
        reason: String,
    },
    AgentPresenceObserved {
        presence: AgentPresence,
    },
    ReleaseRequirementsRecorded {
        requirements: ReleaseRequirements,
    },
    EnvironmentInventoryObserved {
        inventory: EnvironmentInventory,
    },
    EnvironmentAdvisoryReported {
        advisory: EnvironmentAdvisory,
    },
    RemediationRequested {
        remediation: Remediation,
    },
    RemediationTransitioned {
        remediation_id: String,
        expected_state: RemediationState,
        state: RemediationState,
        reason: String,
        fixing_revision: Option<GitRevision>,
    },
    WatchRegistered {
        watch: super::DeliveryWatchRegistration,
    },
    WatchStopped {
        watch_id: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliveryObservationInput {
    pub id: String,
    pub occurred_at: DateTime<Utc>,
    pub body: DeliveryEvent,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliveryObservation {
    pub input: DeliveryObservationInput,
    pub project_id: String,
    pub recorded_at: DateTime<Utc>,
    pub provenance: WorkActor,
    pub source: Option<crate::record_source::RecordSource<WorkActor>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShepherdAssignment {
    pub event_id: String,
    pub agent: AgentIdentity,
    pub assigned_at: DateTime<Utc>,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryBody {
    Contribution {
        contribution: Contribution,
        mappings: Vec<RevisionMapping>,
    },
    PullRequest {
        pull_request: PullRequest,
        snapshot: Option<PullRequestSnapshot>,
    },
    Rollout {
        rollout: Rollout,
    },
    Deployment {
        deployment: Deployment,
    },
    ReleaseRequirements {
        requirements: ReleaseRequirements,
    },
    EnvironmentInventory {
        inventory: EnvironmentInventory,
    },
    EnvironmentAdvisory {
        advisory: EnvironmentAdvisory,
    },
    Remediation {
        remediation: Remediation,
        state: RemediationState,
        reason: Option<String>,
        fixing_revision: Option<GitRevision>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRecord {
    pub subject: DeliverySubject,
    pub title: String,
    pub repository: String,
    pub state: String,
    pub updated_at: DateTime<Utc>,
    pub assignment_event_id: Option<String>,
    #[serde(default)]
    pub composition_event_id: Option<String>,
    pub shepherd: Option<ShepherdAssignment>,
    #[serde(default)]
    pub shepherd_presence: Option<AgentPresence>,
    pub body: DeliveryBody,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DeliveryQuery {
    pub kind: Option<DeliveryKind>,
    pub repository: Option<String>,
    pub state: Option<String>,
    pub feature: Option<String>,
    pub target: Option<String>,
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_since: Option<DateTime<Utc>>,
    pub after: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryAction {
    Record {
        observations: Vec<DeliveryObservationInput>,
    },
    Import {
        observations: Vec<DeliveryObservation>,
    },
    List {
        project_id: Option<String>,
        #[serde(default)]
        query: DeliveryQuery,
    },
    Get {
        project_id: Option<String>,
        subject: DeliverySubject,
    },
    ResolveOrigin {
        project_id: Option<String>,
        query: OriginQuery,
    },
    EnvironmentContext {
        project_id: Option<String>,
        environment: String,
        candidate: Option<GitRevision>,
        pull_request_id: Option<String>,
    },
    CheckHistory {
        project_id: Option<String>,
        repository: String,
        check: String,
        since: DateTime<Utc>,
    },
    Remediate {
        request: Box<RemediationDraft>,
    },
    Acknowledge {
        id: String,
    },
    RemediationStatus {
        project_id: Option<String>,
        id: String,
    },
    Inbox {
        project_id: Option<String>,
        recipient: Option<AgentIdentity>,
        after: Option<String>,
        limit: Option<u32>,
    },
    Events {
        project_id: Option<String>,
        subject: DeliverySubject,
        after: Option<String>,
        limit: Option<u32>,
    },
    Export {
        project_id: Option<String>,
        after: Option<String>,
        limit: Option<u32>,
    },
    Watches {
        project_id: Option<String>,
    },
}
impl DeliveryAction {
    pub fn read_project(&self) -> Option<&str> {
        match self {
            Self::Record { .. }
            | Self::Import { .. }
            | Self::Remediate { .. }
            | Self::Acknowledge { .. } => None,
            Self::List { project_id, .. }
            | Self::Get { project_id, .. }
            | Self::ResolveOrigin { project_id, .. }
            | Self::EnvironmentContext { project_id, .. }
            | Self::CheckHistory { project_id, .. }
            | Self::RemediationStatus { project_id, .. }
            | Self::Inbox { project_id, .. }
            | Self::Events { project_id, .. }
            | Self::Export { project_id, .. }
            | Self::Watches { project_id } => project_id.as_deref(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "response", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryResponse {
    Recorded {
        inserted: usize,
        replayed: usize,
    },
    Page {
        items: Vec<DeliveryRecord>,
        next_cursor: Option<String>,
    },
    Detail {
        record: Option<Box<DeliveryRecord>>,
        related: Vec<DeliveryRecord>,
        observations: Vec<DeliveryObservation>,
        truncated: bool,
    },
    Origins {
        matches: Vec<OriginMatch>,
        #[serde(default)]
        groups: Vec<OriginGroup>,
        matched_by: Option<OriginBasis>,
        qualification: OriginQualification,
        gaps: Vec<String>,
    },
    EnvironmentContext {
        environment: String,
        candidate: Option<GitRevision>,
        requirements_id: Option<String>,
        dependencies: Vec<DependencyAssessment>,
        advisories: Vec<EnvironmentAdvisory>,
        advisories_partial: bool,
    },
    CheckHistory {
        pull_request_ids: Vec<String>,
        partial: bool,
    },
    Remediation {
        remediation: Box<Remediation>,
        state: RemediationState,
        delivery: RemediationDelivery,
        presence: Option<AgentPresence>,
        recipient_last_seen_at: Option<DateTime<Utc>>,
    },
    Events {
        observations: Vec<DeliveryObservation>,
        next_cursor: Option<String>,
    },
    Watches {
        watches: Vec<super::DeliveryWatch>,
        truncated: bool,
    },
}
#[derive(Debug, thiserror::Error)]
pub enum DeliveryRejection {
    #[error("invalid delivery observation: {0}")]
    Invalid(String),
    #[error("delivery conflict: {0}")]
    Conflict(String),
}
