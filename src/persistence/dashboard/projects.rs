// @feature persistence
// @feature observability-ui
// @spec docs/features/persistence.md
// @spec docs/features/observability-ui.md
// @entrypoint recent_projects
// Every source returns at most one timestamp per project before its database-side bound.
use crate::coordination::api::MAX_PROJECT_RESULTS;
use crate::coordination::domain::{query_window_limit, RecordWindow};
use crate::persistence::backend::DbConnection;
use crate::persistence::models::parse_time;
use crate::persistence::schema::{
    analytics_calls, analytics_exposures, analytics_resources, capability_uses,
    concern_observations, eval_records, events, memory_spaces, papercuts, sessions, sos_issues,
    sos_reports, work_observations,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use diesel::dsl::{case_when, max};
use diesel::prelude::*;
use std::collections::BTreeMap;

type CandidateRows = Vec<(Option<String>, Option<String>)>;

// Each store owns its durable receipt/update timestamp; the query recipe and global
// project selection have the same invariants across features. Null global-memory scopes
// never become a synthetic repository. Nullable projection keeps the common row typed.
macro_rules! candidates {
    ($connection:expr, $table:ident, $timestamp:expr) => {
        $table::table
            .filter($table::project_id.nullable().is_not_null())
            .group_by($table::project_id)
            .select(($table::project_id.nullable(), max($timestamp)))
            .order((max($timestamp).desc(), $table::project_id.asc()))
            .limit(query_window_limit(MAX_PROJECT_RESULTS))
            .load::<(Option<String>, Option<String>)>($connection)
    };
}

pub(super) fn recent_projects(
    connection: &mut DbConnection,
) -> Result<RecordWindow<(String, DateTime<Utc>)>> {
    let capability_time = case_when(
        capability_uses::completed_at.is_null(),
        capability_uses::first_observed_at.nullable(),
    )
    .otherwise(capability_uses::completed_at);
    let pages: Vec<CandidateRows> = [
        candidates!(connection, sessions, sessions::last_seen_at),
        candidates!(
            connection,
            work_observations,
            work_observations::recorded_at
        ),
        candidates!(connection, eval_records, eval_records::recorded_at),
        candidates!(
            connection,
            concern_observations,
            concern_observations::recorded_at
        ),
        candidates!(connection, papercuts, papercuts::updated_at),
        candidates!(connection, sos_reports, sos_reports::created_at),
        candidates!(connection, sos_issues, sos_issues::updated_at),
        candidates!(connection, memory_spaces, memory_spaces::updated_at),
        candidates!(connection, capability_uses, capability_time),
        candidates!(connection, analytics_calls, analytics_calls::observed_at),
        candidates!(
            connection,
            analytics_resources,
            analytics_resources::last_seen
        ),
        candidates!(
            connection,
            analytics_exposures,
            analytics_exposures::last_seen
        ),
        candidates!(connection, events, events::created_at),
    ]
    .into_iter()
    .collect::<std::result::Result<_, _>>()?;
    merge(pages)
}

fn merge(pages: Vec<CandidateRows>) -> Result<RecordWindow<(String, DateTime<Utc>)>> {
    let mut by_project: BTreeMap<String, DateTime<Utc>> = BTreeMap::new();
    for (project, timestamp) in pages.into_iter().flatten() {
        let (Some(project), Some(timestamp)) = (project, timestamp) else {
            continue;
        };
        let timestamp = parse_time(&timestamp)?;
        by_project
            .entry(project)
            .and_modify(|latest| *latest = (*latest).max(timestamp))
            .or_insert(timestamp);
    }
    let mut rows: Vec<_> = by_project.into_iter().collect();
    rows.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    Ok(RecordWindow::bounded(rows, MAX_PROJECT_RESULTS))
}
