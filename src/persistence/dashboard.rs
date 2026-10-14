// @feature observability-ui
// @feature persistence
// @spec docs/features/observability-ui.md
// @spec docs/features/persistence.md
// @boundary persisted-event-json
use super::models::{
    claim_from_row, conflict_from_row, parse_time, session_from_row, ClaimRow, ConflictRow,
    SessionRow,
};
use super::schema::{claims, conflicts, events, sessions};
use super::Store;
use crate::coordination::domain::{
    query_window_limit, DashboardRecords, ProjectSummary, RecordWindow, StatusCounts,
};
use anyhow::Result;
use chrono::Utc;
use diesel::dsl::{count_star, max};
use diesel::prelude::*;

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
    ) -> Result<DashboardRecords> {
        let event_limit = event_limit.min(crate::coordination::api::MAX_DASHBOARD_EVENT_RESULTS);
        let record_limit = record_limit.min(crate::coordination::api::MAX_DASHBOARD_RECORD_RESULTS);
        let counts = self.dashboard_counts(project_key)?;

        let (event_items, events_truncated) = self.load_event_window(project_key, event_limit)?;
        let events = RecordWindow {
            items: event_items,
            truncated: events_truncated,
        };

        let mut session_query = sessions::table.into_boxed();
        if let Some(project_key) = project_key {
            session_query = session_query.filter(sessions::project_id.eq(project_key));
        }
        let session_rows = session_query
            .order((sessions::last_seen_at.desc(), sessions::session_id.asc()))
            .limit(query_window_limit(record_limit))
            .select(SessionRow::as_select())
            .load(&mut self.connection)?;
        let sessions = RecordWindow::bounded(
            session_rows
                .into_iter()
                .map(session_from_row)
                .collect::<Result<Vec<_>>>()?,
            record_limit,
        );

        let mut claim_query = claims::table
            .filter(claims::state.eq_any(["claimed", "modified"]))
            .filter(claims::expires_at.gt(Utc::now().to_rfc3339()))
            .into_boxed();
        if let Some(project_key) = project_key {
            claim_query = claim_query.filter(claims::project_id.eq(project_key));
        }
        let claim_rows = claim_query
            .order((claims::updated_at.desc(), claims::id.asc()))
            .limit(query_window_limit(record_limit))
            .select(ClaimRow::as_select())
            .load(&mut self.connection)?;
        let claims = RecordWindow::bounded(
            claim_rows
                .into_iter()
                .map(claim_from_row)
                .collect::<Result<Vec<_>>>()?,
            record_limit,
        );

        let mut conflict_query = conflicts::table.into_boxed();
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

    fn dashboard_counts(&mut self, project_key: Option<&str>) -> Result<StatusCounts> {
        let mut session_query = sessions::table.into_boxed();
        let mut claim_query = claims::table.into_boxed();
        let mut conflict_query = conflicts::table.into_boxed();
        let mut event_query = events::table.into_boxed();
        if let Some(project_key) = project_key {
            session_query = session_query.filter(sessions::project_id.eq(project_key));
            claim_query = claim_query.filter(claims::project_id.eq(project_key));
            conflict_query = conflict_query.filter(conflicts::project_id.eq(project_key));
            event_query = event_query.filter(events::project_id.eq(project_key));
        }
        Ok(StatusCounts {
            active_sessions: session_query
                .filter(sessions::status.eq("active"))
                .select(count_star())
                .first(&mut self.connection)?,
            active_claims: claim_query
                .filter(claims::state.eq_any(["claimed", "modified"]))
                .filter(claims::expires_at.gt(Utc::now().to_rfc3339()))
                .select(count_star())
                .first(&mut self.connection)?,
            open_conflicts: conflict_query
                .filter(conflicts::status.eq("open"))
                .select(count_star())
                .first(&mut self.connection)?,
            events: event_query
                .select(count_star())
                .first(&mut self.connection)?,
        })
    }
}
