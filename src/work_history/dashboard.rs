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
    /// Shared `nexus.accounting.role` of this kind's activities, `mixed` when they
    /// differ. Container kinds (`host.session`, `request`, `host.turn`) envelope work.
    #[serde(default)]
    pub accounting_role: Option<String>,
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
    /// Sessions that day whose every human request has a qualified duration.
    #[serde(default)]
    pub active_known_count: usize,
    #[serde(default)]
    pub active_total_seconds: f64,
    #[serde(default)]
    pub active_average_seconds: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DailyActivityStatistics {
    pub date: String,
    pub kind: String,
    #[serde(flatten)]
    pub duration: DurationStatistics,
}
/// A selectable root plus its human-request activity.
///
/// `active_seconds` sums qualified `request` intervals (prompt submission to host
/// response). It excludes idle time between prompts, so a host left open for days
/// does not look busy. It is known only when every request in the session is timed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DashboardSession {
    #[serde(flatten)]
    pub summary: WorkSummary,
    pub request_count: usize,
    pub timed_request_count: usize,
    pub active_seconds: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkDashboard {
    pub work_session_id: Option<String>,
    pub selected_session: Option<WorkSummary>,
    /// Harness roots whose recorded work is only host containers are omitted.
    pub sessions: Vec<DashboardSession>,
    /// Scanned harness roots hidden because no non-container activity was recorded.
    #[serde(default)]
    pub hidden_session_count: usize,
    pub activities: Vec<WorkSummary>,
    pub intervals: Vec<WorkIntervalNode>,
    pub accounting: Option<WorkSessionAccounting>,
    pub unlinked_activity_count: usize,
    pub session_duration: DurationStatistics,
    /// Per-session active request time; see [`DashboardSession::active_seconds`].
    #[serde(default)]
    pub active_duration: DurationStatistics,
    /// Distinct roots whose member activities feed the activity statistics.
    #[serde(default)]
    pub activity_session_count: usize,
    pub activity_kinds: Vec<ActivityDurationStatistics>,
    pub daily: Vec<DailyDurationStatistics>,
    pub activity_daily: Vec<DailyActivityStatistics>,
    pub scanned_subjects: usize,
    pub source_limit: usize,
    pub truncated: bool,
    pub notices: Vec<String>,
}
