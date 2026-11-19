// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
use super::{AgentIdentity, OriginBasis, OriginQualification, OriginQuery};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemediationDraft {
    pub id: Option<String>,
    #[serde(default)]
    pub origin: OriginQuery,
    pub concern_id: Option<String>,
    pub recipient: Option<AgentIdentity>,
    pub title: String,
    pub message: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemediationDelivery {
    Queued,
    Received,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RemediationNotice {
    pub id: String,
    pub title: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct RemediationOrigin {
    pub matched_by: OriginBasis,
    pub qualification: OriginQualification,
}
