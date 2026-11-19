// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
use super::{AgentIdentity, AgentPresence, ChangedHunk, Contribution, RevisionMapping};
use crate::work_history::GitRevision;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct OriginQuery {
    pub fingerprint: Option<String>,
    pub revision: Option<GitRevision>,
    pub revision_prefix: Option<String>,
    pub pull_request_number: Option<u64>,
    pub pull_request_provider: Option<String>,
    pub branch: Option<String>,
    pub fallback: bool,
    pub repository: Option<String>,
    pub path: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub feature: Option<String>,
    pub pull_request_id: Option<String>,
    pub rollout_id: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OriginQualification {
    Exact,
    Candidates,
    Ambiguous,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OriginMatch {
    pub contribution: Contribution,
    pub mapping: Option<RevisionMapping>,
    pub matched_hunks: Vec<ChangedHunk>,
    pub qualification: OriginQualification,
    pub presence: Option<AgentPresence>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OriginBasis {
    Revision,
    Fingerprint,
    PullRequest,
    Rollout,
    Branch,
    Feature,
    Repository,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OriginGroup {
    pub origin: AgentIdentity,
    pub work_session_id: Option<String>,
    pub activity_id: Option<String>,
    /// Zero-based references into the evidence returned in matches.
    pub evidence: Vec<usize>,
}

impl OriginGroup {
    pub(crate) fn from_matches(matches: &[OriginMatch]) -> Vec<OriginGroup> {
        let mut groups = Vec::<OriginGroup>::new();
        for (index, item) in matches.iter().enumerate() {
            let c = &item.contribution;
            let group = groups.iter_mut().find(|group| {
                let first = &matches[group.evidence[0]].contribution;
                c.origin == first.origin
                    && c.repository == first.repository
                    && match (&c.activity_id, &first.activity_id) {
                        (Some(a), Some(b)) => a == b && c.work_session_id == first.work_session_id,
                        _ => c.id == first.id,
                    }
            });
            if let Some(group) = group {
                group.evidence.push(index);
            } else {
                groups.push(OriginGroup {
                    origin: c.origin.clone(),
                    work_session_id: c.work_session_id.clone(),
                    activity_id: c.activity_id.clone(),
                    evidence: vec![index],
                });
            }
        }
        groups
    }
}
