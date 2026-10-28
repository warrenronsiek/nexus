// @feature code-concerns
// @spec docs/features/code-concerns.md
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernActor {
    pub agent: String,
    pub session_id: String,
    pub model_id: Option<String>,
    pub config_hash: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ConcernContext {
    pub host_session_id: Option<String>,
    pub work_session_id: Option<String>,
    pub activity_id: Option<String>,
    pub repository: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConcernAreaKind {
    Feature,
    Subsystem,
    File,
    Declaration,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernArea {
    #[serde(default)]
    pub id: Option<String>,
    pub repository: String,
    pub kind: ConcernAreaKind,
    pub key: String,
    pub display_name: String,
    pub discriminator: Option<String>,
}
impl ConcernArea {
    pub fn identity(&self) -> String {
        self.id.clone().unwrap_or_else(|| {
            crate::identity::stable_id(&[
                "code-area",
                &self.repository,
                self.kind.as_str(),
                &self.key,
                self.discriminator.as_deref().unwrap_or(""),
            ])
        })
    }
}
impl ConcernAreaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Subsystem => "subsystem",
            Self::File => "file",
            Self::Declaration => "declaration",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConcernAreaRelation {
    Affected,
    SuspectedCause,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConcernVerification {
    Supplied,
    MechanicallyVerified,
    ModelSuggested,
    Unavailable,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernLocation {
    pub repository: String,
    pub object_format: Option<String>,
    pub commit: Option<String>,
    pub path: Option<String>,
    pub language: Option<String>,
    pub qualified_name: Option<String>,
    pub declaration_kind: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub dirty: Option<bool>,
    pub verification: ConcernVerification,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernAreaAttribution {
    pub area: ConcernArea,
    pub relation: ConcernAreaRelation,
    pub location: Option<ConcernLocation>,
    #[serde(default)]
    pub memberships: Vec<ConcernArea>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernFailureSource {
    pub source: String,
    pub repository: String,
    pub execution_attempt: String,
    pub failure_id: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ConcernEncounter {
    pub symptom: Option<String>,
    pub consequence: Option<String>,
    pub workaround: Option<String>,
    pub source: Option<ConcernFailureSource>,
    pub areas: Vec<ConcernAreaAttribution>,
    pub evidence: Vec<ConcernEvidence>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConcernEvidence {
    Concern {
        id: String,
    },
    Papercut {
        id: String,
    },
    Sos {
        id: String,
    },
    Eval {
        id: String,
    },
    WorkSession {
        id: String,
    },
    WorkActivity {
        id: String,
    },
    External {
        uri: String,
        description: String,
    },
    Git {
        location: ConcernLocation,
    },
    Artifact {
        backing: crate::evals::ArtifactBacking,
        description: String,
        verification: ConcernVerification,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConcernFixRelation {
    ContainsFix,
    PredatesFix,
    AnotherBranch,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConcernEvent {
    Reported {
        title: String,
        description: String,
        hypothesis: Option<String>,
        impact: Option<String>,
        encounter: ConcernEncounter,
    },
    Encountered {
        encounter: ConcernEncounter,
        fix_relation: Option<ConcernFixRelation>,
    },
    EvidenceAdded {
        encounter_id: Option<String>,
        note: String,
        evidence: Vec<ConcernEvidence>,
    },
    RemedyRecorded {
        encounter_ids: Vec<String>,
        what: String,
        why: String,
        outcome: String,
        evidence: Vec<ConcernEvidence>,
    },
    ReviewRecorded {
        encounter_ids: Vec<String>,
        hypothesis: String,
        rationale: String,
        alternatives: Vec<String>,
        evidence: Vec<ConcernEvidence>,
    },
    Resolved {
        reason: String,
        fixing_location: Option<ConcernLocation>,
        evidence: Vec<ConcernEvidence>,
    },
    Reopened {
        reason: String,
        evidence: Vec<ConcernEvidence>,
    },
    AreaMapped {
        from: ConcernArea,
        to: ConcernArea,
        relation: ConcernMappingRelation,
        verification: ConcernVerification,
        reason: String,
        replaces: Option<String>,
    },
    DuplicateProposed {
        target_concern_id: String,
        rationale: String,
    },
    DuplicateResponded {
        proposal_id: String,
        decision: ConcernDuplicateDecision,
        reason: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConcernMappingRelation {
    MemberOf,
    MovedTo,
    SplitInto,
    Corrects,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConcernDuplicateDecision {
    Approve,
    Reject,
}
impl ConcernEvent {
    pub fn encounter(&self) -> Option<&ConcernEncounter> {
        match self {
            Self::Reported { encounter, .. } | Self::Encountered { encounter, .. } => {
                Some(encounter)
            }
            _ => None,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Reported { .. } => "reported",
            Self::Encountered { .. } => "encountered",
            Self::EvidenceAdded { .. } => "evidence_added",
            Self::RemedyRecorded { .. } => "remedy_recorded",
            Self::ReviewRecorded { .. } => "review_recorded",
            Self::Resolved { .. } => "resolved",
            Self::Reopened { .. } => "reopened",
            Self::AreaMapped { .. } => "area_mapped",
            Self::DuplicateProposed { .. } => "duplicate_proposed",
            Self::DuplicateResponded { .. } => "duplicate_responded",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernObservationInput {
    pub id: String,
    pub concern_id: String,
    pub occurred_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
    #[serde(default)]
    pub context: ConcernContext,
    pub body: ConcernEvent,
}
pub type ConcernSource = crate::record_source::RecordSource<ConcernActor>;
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConcernObservation {
    pub input: ConcernObservationInput,
    pub project_id: String,
    pub recorded_at: DateTime<Utc>,
    pub provenance: ConcernActor,
    pub source: Option<ConcernSource>,
    pub reporter_token_hash: Option<String>,
    pub canonical_id: Option<String>,
}
