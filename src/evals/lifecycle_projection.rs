// @feature evals
// @spec docs/features/evals.md
use super::ReportedState;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleProjection {
    pub initial_state: ReportedState,
    pub frontiers: Vec<LifecycleFrontier>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleFrontier {
    pub id: String,
    pub state: ReportedState,
    pub occurred_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
}
