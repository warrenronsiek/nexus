// @feature observability-ui
// @feature persistence
// @spec docs/features/observability-ui.md
// @spec docs/features/persistence.md
// @boundary persisted-event-json
use super::models::{
    claim_from_row, conflict_from_row, parse_time, session_from_row, ClaimRow, ConflictRow,
    EventRow, SessionRow,
};
use super::schema::{claims, conflicts, events, sessions};
use super::Store;
use crate::coordination::domain::{
    DashboardRecords, EventPayload, EventRecord, ProjectSummary, RecordWindow, StatusCounts,
};
use anyhow::Result;
use chrono::Utc;
use diesel::dsl::count_star;
use diesel::prelude::*;

impl Store {
    pub fn list_projects(&mut self) -> Result<Vec<ProjectSummary>> {
        let rows = sessions::table
            .order(sessions::last_seen_at.desc())
            .select(SessionRow::as_select())
            .load::<SessionRow>(&mut self.connection)?;
        let mut projects: Vec<ProjectSummary> = Vec::new();
        for row in rows {
            if let Some(project) = projects
                .iter_mut()
                .find(|project| project.project_id == row.project_id)
            {
                if !project.agents.contains(&row.agent) {
                    project.agents.push(row.agent);
                }
                if let Some(worktree) = row.worktree {
                    if !project.worktrees.contains(&worktree) {
                        project.worktrees.push(worktree);
                    }
                }
            } else {
                projects.push(ProjectSummary {
                    project_id: row.project_id,
                    worktrees: row.worktree.into_iter().collect(),
                    agents: vec![row.agent],
                    last_seen_at: parse_time(&row.last_seen_at)?,
                });
            }
        }
        Ok(projects)
    }

    pub fn dashboard(
        &mut self,
        project_key: Option<&str>,
        event_limit: usize,
        record_limit: usize,
    ) -> Result<DashboardRecords> {
        let counts = self.dashboard_counts(project_key)?;

        let mut event_query = events::table.into_boxed();
        if let Some(project_key) = project_key {
            event_query = event_query.filter(events::project_id.eq(project_key));
        }
        let event_rows = event_query
            .order(events::id.desc())
            .limit(window_limit(event_limit))
            .select(EventRow::as_select())
            .load(&mut self.connection)?;
        let events = RecordWindow::bounded(
            event_rows
                .into_iter()
                .map(|row| {
                    Ok(EventRecord {
                        id: row.id,
                        project_id: row.project_id,
                        session_id: row.session_id,
                        kind: row.kind,
                        payload: EventPayload::from(serde_json::from_str::<serde_json::Value>(
                            &row.payload_json,
                        )?),
                        created_at: parse_time(&row.created_at)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            event_limit,
        );

        let mut session_query = sessions::table.into_boxed();
        if let Some(project_key) = project_key {
            session_query = session_query.filter(sessions::project_id.eq(project_key));
        }
        let session_rows = session_query
            .order(sessions::last_seen_at.desc())
            .limit(window_limit(record_limit))
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
            .order(claims::updated_at.desc())
            .limit(window_limit(record_limit))
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
            .order(conflicts::updated_at.desc())
            .limit(window_limit(record_limit))
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

fn window_limit(maximum: usize) -> i64 {
    i64::try_from(maximum.saturating_add(1)).unwrap_or(i64::MAX)
}
