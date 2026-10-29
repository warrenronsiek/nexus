// @feature observability-ui
// @feature persistence
// @spec docs/features/observability-ui.md
// @spec docs/features/persistence.md
// @boundary persisted-event-json
use super::models::{conflict_from_row, parse_time, ConflictRow, SessionRow};
use super::schema::{claims, conflicts, events, sessions};
use super::Store;
use crate::coordination::domain::{
    query_window_limit, DashboardRecords, ProjectSummary, RecordWindow, StatusCounts,
};
use anyhow::Result;
use chrono::Utc;
use diesel::dsl::{count_star, max};
use diesel::prelude::*;

#[cfg(all(test, feature = "postgres"))]
mod tests;

impl Store {
    pub fn list_projects(&mut self) -> Result<Vec<ProjectSummary>> {
        Ok(self.list_projects_window()?.items)
    }

    pub(crate) fn list_projects_window(&mut self) -> Result<RecordWindow<ProjectSummary>> {
        let project_rows = sessions::table
            .group_by(sessions::project_id)
            .select((sessions::project_id, max(sessions::last_seen_at)))
            .order((
                max(sessions::last_seen_at).desc(),
                sessions::project_id.asc(),
            ))
            .limit(query_window_limit(
                crate::coordination::api::MAX_PROJECT_RESULTS,
            ))
            .load::<(String, Option<String>)>(&mut self.connection)?;
        let project_window =
            RecordWindow::bounded(project_rows, crate::coordination::api::MAX_PROJECT_RESULTS);
        let mut truncated = project_window.truncated;
        let mut projects = Vec::with_capacity(project_window.items.len());
        for (project_id, last_seen_at) in project_window.items {
            let rows = sessions::table
                .filter(sessions::project_id.eq(&project_id))
                .order((sessions::last_seen_at.desc(), sessions::session_id.asc()))
                .limit(query_window_limit(
                    crate::coordination::api::MAX_PROJECT_SUMMARY_SESSIONS,
                ))
                .select(SessionRow::as_select())
                .load::<SessionRow>(&mut self.connection)?;
            let rows =
                RecordWindow::bounded(rows, crate::coordination::api::MAX_PROJECT_SUMMARY_SESSIONS);
            truncated |= rows.truncated;
            let mut agents = Vec::new();
            let mut worktrees = Vec::new();
            for row in rows.items {
                if !agents.contains(&row.agent) {
                    agents.push(row.agent);
                }
                if let Some(worktree) = row.worktree {
                    if !worktrees.contains(&worktree) {
                        worktrees.push(worktree);
                    }
                }
            }
            projects.push(ProjectSummary {
                project_id,
                worktrees,
                agents,
                last_seen_at: parse_time(last_seen_at.as_deref().unwrap_or_default())?,
            });
        }
        Ok(RecordWindow {
            items: projects,
            truncated,
        })
    }

    pub fn dashboard(
        &mut self,
        project_key: Option<&str>,
        event_limit: usize,
        record_limit: usize,
        activity_window_seconds: i64,
    ) -> Result<DashboardRecords> {
        let event_limit = event_limit.min(crate::coordination::api::MAX_DASHBOARD_EVENT_RESULTS);
        let record_limit = record_limit.min(crate::coordination::api::MAX_DASHBOARD_RECORD_RESULTS);
        let counts = self.dashboard_counts(project_key, activity_window_seconds)?;

        let (event_items, events_truncated) = self.load_event_window(project_key, event_limit)?;
        let events = RecordWindow {
            items: event_items,
            truncated: events_truncated,
        };

        let sessions = self.list_sessions_window(
            project_key,
            crate::coordination::domain::RecordScope::Active,
            activity_window_seconds,
            record_limit,
        )?;
        let claims = self.list_claims_window(
            project_key,
            crate::coordination::domain::RecordScope::Active,
            record_limit,
        )?;

        // The dashboard prioritizes recently changed conflicts; history lists use creation order.
        let mut conflict_query = conflicts::table
            .filter(super::liveness::live_conflict())
            .into_boxed();
        if let Some(project_key) = project_key {
            conflict_query = conflict_query.filter(conflicts::project_id.eq(project_key));
        }
        let conflict_rows = conflict_query
            .order((conflicts::updated_at.desc(), conflicts::id.asc()))
            .limit(query_window_limit(record_limit))
            .select(ConflictRow::as_select())
            .load(&mut self.connection)?;
        let conflicts = RecordWindow::bounded(
            conflict_rows
                .into_iter()
                .map(conflict_from_row)
                .collect::<Result<Vec<_>>>()?,
            record_limit,
        );

        Ok(DashboardRecords {
            counts,
            events,
            sessions,
            claims,
            conflicts,
        })
    }

    pub(super) fn dashboard_counts(
        &mut self,
        project_key: Option<&str>,
        activity_window_seconds: i64,
    ) -> Result<StatusCounts> {
        let mut session_query = sessions::table
            .filter(super::liveness::recent_session(activity_window_seconds))
            .into_boxed();
        let mut claim_query = claims::table.into_boxed();
        let mut conflict_query = conflicts::table
            .filter(super::liveness::live_conflict())
            .into_boxed();
        let mut event_query = events::table.into_boxed();
        if let Some(project_key) = project_key {
            session_query = session_query.filter(sessions::project_id.eq(project_key));
            claim_query = claim_query.filter(claims::project_id.eq(project_key));
            conflict_query = conflict_query.filter(conflicts::project_id.eq(project_key));
            event_query = event_query.filter(events::project_id.eq(project_key));
        }
        Ok(StatusCounts {
            active_sessions: session_query
                .select(count_star())
                .first(&mut self.connection)?,
            active_claims: claim_query
                .filter(claims::state.eq_any(["claimed", "modified"]))
                .filter(claims::expires_at.gt(Utc::now().to_rfc3339()))
                .select(count_star())
                .first(&mut self.connection)?,
            open_conflicts: conflict_query
                .select(count_star())
                .first(&mut self.connection)?,
            events: event_query
                .select(count_star())
                .first(&mut self.connection)?,
        })
    }
}
