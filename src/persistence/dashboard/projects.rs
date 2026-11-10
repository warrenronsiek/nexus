// @feature persistence
// @feature observability-ui
// @spec docs/features/persistence.md
// @spec docs/features/observability-ui.md
// @entrypoint recent_projects
// The catalog is a rebuildable projection: history tables remain authoritative, while
// discovery reads one indexed row per project instead of aggregating their history.
use super::Store;
use crate::coordination::api::MAX_PROJECT_RESULTS;
use crate::coordination::domain::{query_window_limit, RecordWindow};
use crate::persistence::backend::DbConnection;
use crate::persistence::models::parse_time;
use crate::persistence::schema::{
    analytics_calls, analytics_exposures, analytics_resources, capability_uses, code_map_snapshots,
    concern_observations, eval_records, events, memory_spaces, papercuts, projects, sessions,
    sos_issues, sos_reports, work_observations,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use diesel::dsl::{case_when, max, min};
use diesel::prelude::*;
use diesel::upsert::excluded;
use std::collections::BTreeMap;

pub(super) struct RecentProject {
    pub(super) project_id: String,
    pub(super) display_name: Option<String>,
    pub(super) last_seen_at: DateTime<Utc>,
}

pub(super) fn recent_projects(
    connection: &mut DbConnection,
) -> Result<RecordWindow<RecentProject>> {
    let rows = projects::table
        .order((projects::last_seen_at.desc(), projects::project_id.asc()))
        .limit(query_window_limit(MAX_PROJECT_RESULTS))
        .select((
            projects::project_id,
            projects::display_name,
            projects::last_seen_at,
        ))
        .load::<(String, Option<String>, String)>(connection)?;
    let rows = rows
        .into_iter()
        .map(|(project_id, display_name, last_seen_at)| {
            Ok(RecentProject {
                project_id,
                display_name,
                last_seen_at: parse_time(&last_seen_at)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(RecordWindow::bounded(rows, MAX_PROJECT_RESULTS))
}

type ActivityRows = Vec<(String, Option<String>, Option<String>)>;

// Each store owns its durable receipt/update timestamps; null global-memory scopes never
// become a synthetic repository.
macro_rules! activity {
    ($connection:expr, $table:ident, $first:expr, $last:expr) => {
        $table::table
            .filter($table::project_id.nullable().is_not_null())
            .group_by($table::project_id)
            .select(($table::project_id.nullable(), min($first), max($last)))
            .load::<(Option<String>, Option<String>, Option<String>)>($connection)
            .map(|rows| {
                rows.into_iter()
                    .filter_map(|(project, first, last)| project.map(|id| (id, first, last)))
                    .collect::<ActivityRows>()
            })
    };
}

impl Store {
    /// Advances a project's recency, keeping an earlier label when the caller sent none.
    pub(crate) fn record_project_activity(
        &mut self,
        project_id: &str,
        display_name: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let now = now.to_rfc3339();
        let insert = diesel::insert_into(projects::table)
            .values((
                projects::project_id.eq(project_id),
                projects::display_name.eq(display_name),
                projects::first_seen_at.eq(&now),
                projects::last_seen_at.eq(&now),
            ))
            .on_conflict(projects::project_id)
            .do_update();
        match display_name {
            Some(_) => insert
                .set((
                    projects::last_seen_at.eq(excluded(projects::last_seen_at)),
                    projects::display_name.eq(excluded(projects::display_name)),
                ))
                .execute(&mut self.connection)?,
            None => insert
                .set(projects::last_seen_at.eq(excluded(projects::last_seen_at)))
                .execute(&mut self.connection)?,
        };
        Ok(())
    }
}

/// Restores the catalog from authoritative history after a portable import. Snapshots
/// carry history, not this projection, so display names return with the next activity.
pub(crate) fn rebuild_catalog(connection: &mut DbConnection) -> Result<usize> {
    let mut sources = coordination_activity(connection)?;
    sources.extend(record_activity(connection)?);
    let spans = activity_spans(sources)?;
    let rows = spans
        .iter()
        .map(|(project, (first, last))| {
            (
                projects::project_id.eq(project),
                projects::first_seen_at.eq(first.to_rfc3339()),
                projects::last_seen_at.eq(last.to_rfc3339()),
            )
        })
        .collect::<Vec<_>>();
    for chunk in rows.chunks(256) {
        diesel::insert_into(projects::table)
            .values(chunk)
            .on_conflict_do_nothing()
            .execute(connection)?;
    }
    Ok(rows.len())
}

/// Host-observed activity: sessions, events, and usage capture.
fn coordination_activity(connection: &mut DbConnection) -> QueryResult<Vec<ActivityRows>> {
    let capability_time = case_when(
        capability_uses::completed_at.is_null(),
        capability_uses::first_observed_at.nullable(),
    )
    .otherwise(capability_uses::completed_at);
    [
        activity!(
            connection,
            sessions,
            sessions::started_at,
            sessions::last_seen_at
        ),
        activity!(connection, events, events::created_at, events::created_at),
        activity!(
            connection,
            capability_uses,
            capability_uses::first_observed_at,
            capability_time
        ),
        activity!(
            connection,
            analytics_calls,
            analytics_calls::observed_at,
            analytics_calls::observed_at
        ),
        activity!(
            connection,
            analytics_resources,
            analytics_resources::first_seen,
            analytics_resources::last_seen
        ),
        activity!(
            connection,
            analytics_exposures,
            analytics_exposures::first_seen,
            analytics_exposures::last_seen
        ),
    ]
    .into_iter()
    .collect()
}

/// Caller-owned records that exist without any host session.
fn record_activity(connection: &mut DbConnection) -> QueryResult<Vec<ActivityRows>> {
    [
        activity!(
            connection,
            work_observations,
            work_observations::recorded_at,
            work_observations::recorded_at
        ),
        activity!(
            connection,
            eval_records,
            eval_records::recorded_at,
            eval_records::recorded_at
        ),
        activity!(
            connection,
            code_map_snapshots,
            code_map_snapshots::observed_at,
            code_map_snapshots::observed_at
        ),
        activity!(
            connection,
            concern_observations,
            concern_observations::recorded_at,
            concern_observations::recorded_at
        ),
        activity!(
            connection,
            papercuts,
            papercuts::created_at,
            papercuts::updated_at
        ),
        activity!(
            connection,
            sos_reports,
            sos_reports::created_at,
            sos_reports::created_at
        ),
        activity!(
            connection,
            sos_issues,
            sos_issues::created_at,
            sos_issues::updated_at
        ),
        activity!(
            connection,
            memory_spaces,
            memory_spaces::created_at,
            memory_spaces::updated_at
        ),
    ]
    .into_iter()
    .collect()
}

/// First and last activity per project.
type ActivitySpans = BTreeMap<String, (DateTime<Utc>, DateTime<Utc>)>;

fn activity_spans(sources: Vec<ActivityRows>) -> Result<ActivitySpans> {
    let mut spans = ActivitySpans::new();
    for (project, first, last) in sources.into_iter().flatten() {
        let (Some(first), Some(last)) = (first, last) else {
            continue;
        };
        let (first, last) = (parse_time(&first)?, parse_time(&last)?);
        spans
            .entry(project)
            .and_modify(|span| *span = (span.0.min(first), span.1.max(last)))
            .or_insert((first, last));
    }
    Ok(spans)
}
