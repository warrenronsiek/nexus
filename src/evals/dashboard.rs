// @feature evals
// @spec docs/features/evals.md
use super::MeasurementValue;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalDashboardGroup {
    pub id: String,
    pub label: String,
    pub suite_identity: String,
    pub suite_revision_ids: Vec<String>,
    pub target_identity: String,
    pub target_revision_ids: Vec<String>,
    pub repository: Vec<String>,
    pub models: Vec<String>,
    pub components: Vec<String>,
    pub run_count: usize,
    pub result_count: usize,
    pub latest_at: DateTime<Utc>,
    pub statuses: BTreeMap<String, usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalDashboardPoint {
    pub record_id: String,
    pub run_id: String,
    pub suite_revision_id: String,
    pub target_revision_id: String,
    pub occurred_at: DateTime<Utc>,
    pub value: MeasurementValue,
    /// A conservative presentation approximation; the original value remains authoritative.
    pub numeric_value: Option<f64>,
    pub replaces: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalDashboardSeries {
    pub id: String,
    pub label: String,
    pub unit: String,
    pub definition_id: String,
    pub description: String,
    pub calculation: String,
    pub subject_scope: String,
    pub superseded_count: usize,
    pub points: Vec<EvalDashboardPoint>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalDashboard {
    pub groups: Vec<EvalDashboardGroup>,
    pub selected_group_id: Option<String>,
    pub series: Vec<EvalDashboardSeries>,
    pub run_count: usize,
    pub result_count: usize,
    pub scanned_records: usize,
    pub source_limit: usize,
    pub truncated: bool,
    pub notices: Vec<String>,
}
