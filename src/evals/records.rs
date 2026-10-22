// @feature evals
// @spec docs/features/evals.md
use super::lifecycle_projection::LifecycleProjection;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! labels {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        pub enum $name { $(#[serde(rename = $label)] $variant),+ }
        impl $name {
            pub fn as_str(self) -> &'static str { match self { $(Self::$variant => $label),+ } }
        }
    };
}
labels!(ReasonKind {
    ObservedFailure=>"observed_failure", UserRequirement=>"user_requirement", NewCapability=>"new_capability",
    CoverageHypothesis=>"coverage_hypothesis", GraderCorrection=>"grader_correction", DistributionChange=>"distribution_change",
    Removal=>"removal", Maintenance=>"maintenance"
});
labels!(RationaleProvenance { Contemporaneous=>"contemporaneous", Retrospective=>"retrospective", Imported=>"imported", Inferred=>"inferred" });
labels!(ReportedState { Planned=>"planned", Running=>"running", Completed=>"completed", Failed=>"failed", Canceled=>"canceled", Skipped=>"skipped", Interrupted=>"interrupted", Abandoned=>"abandoned", Unknown=>"unknown" });
labels!(CaptureState { Exact=>"exact", Partial=>"partial", Redacted=>"redacted", Unavailable=>"unavailable", Unsupported=>"unsupported", Unknown=>"unknown" });
labels!(ObjectFormat { Sha1=>"sha1", Sha256=>"sha256" });
labels!(Split { Development=>"development", Validation=>"validation", Holdout=>"holdout", Unspecified=>"unspecified" });
labels!(Direction { Higher=>"higher", Lower=>"lower", None=>"none" });
labels!(DecisionState { Accepted=>"accepted", Rejected=>"rejected", Inconclusive=>"inconclusive", Abandoned=>"abandoned" });
labels!(ComparisonValidity { Matched=>"matched", DisclosedDifferences=>"disclosed_differences", Incompatible=>"incompatible", Unknown=>"unknown" });

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalRecordInput {
    pub id: String,
    pub title: String,
    pub occurred_at: DateTime<Utc>,
    pub body: EvalBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<EvalSource>,
}

macro_rules! record_bodies {
    ($($variant:ident($payload:ident) => $label:literal),+ $(,)?) => {
        labels!(EvalKind { $($variant => $label),+ });
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        pub enum EvalBody { $($variant($payload)),+ }
        impl EvalBody {
            pub fn kind(&self) -> EvalKind {
                match self { $(Self::$variant(_) => EvalKind::$variant),+ }
            }
            pub(crate) fn append_references(&self, refs: &mut Vec<super::validation::Reference>) {
                match self { $(Self::$variant(value) => super::validation::RecordReferences::append_references(value,refs)),+ }
            }
            pub(crate) fn validate_payload(&self) -> anyhow::Result<()> {
                match self { $(Self::$variant(value) => super::validation::ValidatePayload::validate_payload(value)),+ }
            }
        }
    };
}
record_bodies! {
    Change(Change)=>"change", Repository(RepositoryRevision)=>"repository", Model(ModelRevision)=>"model",
    Case(CaseRevision)=>"case", Suite(SuiteRevision)=>"suite", Grader(GraderRevision)=>"grader",
    Target(TargetRevision)=>"target", Experiment(Experiment)=>"experiment", Run(Run)=>"run", Trial(Trial)=>"trial",
    Attempt(Attempt)=>"attempt", Call(Call)=>"call", Trace(Trace)=>"trace", Grade(Grade)=>"grade",
    Comparison(Comparison)=>"comparison", Decision(Decision)=>"decision", Selection(Selection)=>"selection",
    Artifact(Artifact)=>"artifact", Verification(Verification)=>"verification", Lifecycle(Lifecycle)=>"lifecycle",
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Motivation {
    Recorded {
        reason: ReasonKind,
        problem: String,
        expected_behavior: String,
        rationale: String,
        provenance: RationaleProvenance,
    },
    Missing {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceLink {
    Record {
        id: String,
    },
    External {
        uri: String,
        description: String,
        snapshot: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub motivation: Motivation,
    #[serde(default)]
    pub evidence: Vec<EvidenceLink>,
    #[serde(default)]
    pub supersedes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Revision {
    pub identity: String,
    #[serde(default)]
    pub parents: Vec<String>,
    pub change_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileOverride {
    pub path: String,
    pub artifact_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositoryRevision {
    #[serde(flatten)]
    pub revision: Revision,
    pub repository: String,
    pub commit: String,
    pub object_format: ObjectFormat,
    #[serde(default)]
    pub overrides: Vec<FileOverride>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelRevision {
    #[serde(flatten)]
    pub revision: Revision,
    pub label: String,
    pub provider: String,
    pub requested_model: String,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    pub name: String,
    pub expected_behavior: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseRevision {
    #[serde(flatten)]
    pub revision: Revision,
    #[serde(default)]
    pub inputs: Vec<String>,
    pub expected_behavior: String,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub derived_from: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuiteMember {
    pub case_id: String,
    pub grader_ids: Vec<String>,
    pub weight: f64,
    pub split: Split,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuiteRevision {
    #[serde(flatten)]
    pub revision: Revision,
    pub cases: Vec<SuiteMember>,
    pub aggregation: String,
    pub sampling: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum GraderMethod {
    Code {
        artifact_id: String,
    },
    Rubric {
        artifact_id: String,
        model_id: Option<String>,
    },
    Human {
        instructions: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Metric {
    pub name: String,
    pub unit: String,
    pub direction: Direction,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub threshold: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GraderRevision {
    #[serde(flatten)]
    pub revision: Revision,
    pub grading: GraderMethod,
    pub metric: Metric,
    #[serde(default)]
    pub calibration: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "assignment", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelAssignment {
    Fixed { model_id: String },
    Policy { artifact_id: String },
    Unspecified { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub id: String,
    pub role: String,
    pub repository_id: Option<String>,
    pub model: ModelAssignment,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetRevision {
    #[serde(flatten)]
    pub revision: Revision,
    pub components: Vec<Component>,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Experiment {
    pub hypothesis: String,
    pub expected_effect: String,
    pub parent_id: Option<String>,
    pub baseline_id: String,
    pub candidate_id: String,
    pub suite_id: String,
    pub plan: String,
    pub plan_recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub dimension: String,
    pub state: CaptureState,
    pub producer: String,
    pub reason: String,
    pub omitted_items: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub target_id: String,
    pub suite_id: String,
    pub experiment_id: Option<String>,
    pub runner: String,
    pub runner_version: String,
    pub state: ReportedState,
    pub selection_rule: String,
    #[serde(default)]
    pub selected_cases: Vec<String>,
    #[serde(default)]
    pub capture: Vec<Capture>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Trial {
    pub run_id: String,
    pub case_id: String,
    pub replicate: u32,
    pub seed: Option<String>,
    pub state: ReportedState,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    pub trial_id: String,
    pub parent_id: Option<String>,
    pub retry_reason: Option<String>,
    pub state: ReportedState,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleSubject {
    Run { id: String },
    Trial { id: String },
    Attempt { id: String },
}

impl LifecycleSubject {
    pub(crate) fn record_id(&self) -> &str {
        match self {
            Self::Run { id } | Self::Trial { id } | Self::Attempt { id } => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lifecycle {
    pub subject: LifecycleSubject,
    pub state: ReportedState,
    pub previous_id: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextEntry {
    pub role: String,
    pub artifact_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolSurface {
    pub name: String,
    pub definition_artifact: Option<String>,
    pub state: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
    pub pricing_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub attempt_id: String,
    pub parent_id: Option<String>,
    pub component: String,
    pub instance: String,
    pub sequence: u64,
    pub model_id: String,
    pub served_model: Option<String>,
    pub request_id: Option<String>,
    #[serde(default)]
    pub context: Vec<ContextEntry>,
    #[serde(default)]
    pub tools: Vec<ToolSurface>,
    #[serde(default)]
    pub response_artifacts: Vec<String>,
    #[serde(default)]
    pub capture: Vec<Capture>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum TraceEvent {
    Tool {
        name: String,
        input_artifact: Option<String>,
        output_artifact: Option<String>,
        state: ReportedState,
    },
    Handoff {
        from_instance: String,
        to_instance: String,
        destination: String,
        payload_artifact: Option<String>,
        eligible_destinations: Vec<String>,
    },
    Outcome {
        state: ReportedState,
        evidence_artifacts: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Trace {
    pub attempt_id: String,
    pub call_id: Option<String>,
    #[serde(default)]
    pub causes: Vec<String>,
    pub sequence: u64,
    pub detail: TraceEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssessmentSubject {
    Trial {
        id: String,
    },
    Attempt {
        id: String,
    },
    Call {
        id: String,
    },
    Component {
        attempt_id: String,
        instance: String,
        #[serde(default)]
        call_ids: Vec<String>,
    },
    Routing {
        trace_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AspectGrade {
    pub name: String,
    pub score: Option<f64>,
    pub passed: Option<bool>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Grade {
    pub subject: AssessmentSubject,
    pub grader_id: String,
    pub pass_id: String,
    pub state: ReportedState,
    pub score: Option<f64>,
    pub explanation: String,
    #[serde(default)]
    pub aspects: Vec<AspectGrade>,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub replaces: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Comparison {
    pub baseline_ids: Vec<String>,
    pub candidate_ids: Vec<String>,
    pub grade_ids: Vec<String>,
    pub population: String,
    pub exclusions: String,
    pub metric: Metric,
    pub validity: ComparisonValidity,
    pub differences: Vec<String>,
    pub result: String,
    pub uncertainty: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub subject_id: String,
    pub comparison_ids: Vec<String>,
    pub state: DecisionState,
    pub reason: String,
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub scope: String,
    pub revision_id: Option<String>,
    pub previous_id: Option<String>,
    pub change_id: String,
    pub effective_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactBacking {
    GitFile {
        repository: String,
        commit: String,
        object_format: ObjectFormat,
        path: String,
        blob_oid: Option<String>,
        entry_mode: String,
    },
    GitLfs {
        repository: String,
        commit: String,
        path: String,
        oid: String,
        size: u64,
    },
    StoredBlob {
        digest: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub media_type: String,
    pub byte_size: u64,
    pub digest: Option<String>,
    pub backings: Vec<ArtifactBacking>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub artifact_id: String,
    pub backing: ArtifactBacking,
    pub state: CaptureState,
    pub checked_at: DateTime<Utc>,
    pub producer: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalProvenance {
    pub agent: String,
    pub session_id: String,
    pub model_id: Option<String>,
    pub config_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalSource {
    pub project_id: String,
    pub recorded_at: DateTime<Utc>,
    pub provenance: EvalProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalLink {
    pub relation: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalRecord {
    #[serde(flatten)]
    pub input: EvalRecordInput,
    pub project_id: String,
    pub recorded_at: DateTime<Utc>,
    pub provenance: EvalProvenance,
    pub links: Vec<EvalLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<LifecycleProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalHeader {
    pub id: String,
    pub kind: EvalKind,
    pub title: String,
    pub occurred_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
    pub repository: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalQuery {
    pub kind: Option<EvalKind>,
    pub repository: Option<String>,
    pub model: Option<String>,
    pub search: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalPage {
    pub records: Vec<EvalHeader>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalReceipt {
    pub inserted: usize,
    pub replayed: usize,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalHistory {
    pub records: Vec<EvalRecord>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalBlob {
    pub digest: String,
    pub upload_id: String,
    pub byte_size: u64,
    pub chunk_count: u32,
    pub verified: bool,
}
