// @feature work-history
// @spec docs/features/work-history.md
use super::{WorkIntervalNode, WorkSessionAccounting, WorkSummary};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Inclusive observed intervals; nested sums are not elapsed wall time or effort.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DurationStatistics {
    pub count: usize,
    pub known_count: usize,
    pub total_seconds: f64,
    pub average_seconds: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityDurationStatistics {
    pub kind: String,
    #[serde(flatten)]
    pub duration: DurationStatistics,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DailyDurationStatistics {
    pub date: String,
    pub session_count: usize,
    pub known_count: usize,
    pub total_seconds: f64,
    pub average_seconds: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DailyActivityStatistics {
    pub date: String,
    pub kind: String,
    #[serde(flatten)]
    pub duration: DurationStatistics,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkDashboard {
    pub work_session_id: Option<String>,
    pub selected_session: Option<WorkSummary>,
    pub sessions: Vec<WorkSummary>,
    pub activities: Vec<WorkSummary>,
    pub intervals: Vec<WorkIntervalNode>,
    pub accounting: Option<WorkSessionAccounting>,
    pub unlinked_activity_count: usize,
    pub session_duration: DurationStatistics,
    pub activity_kinds: Vec<ActivityDurationStatistics>,
    pub daily: Vec<DailyDurationStatistics>,
    pub activity_daily: Vec<DailyActivityStatistics>,
    pub scanned_subjects: usize,
    pub source_limit: usize,
    pub truncated: bool,
    pub notices: Vec<String>,
}
