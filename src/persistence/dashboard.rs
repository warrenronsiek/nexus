// @feature observability-ui
// @feature persistence
// @spec docs/features/observability-ui.md
// @spec docs/features/persistence.md
// @boundary persisted-event-json
use super::models::{conflict_from_row, ConflictRow, SessionRow};
use super::schema::{advisories, claims, conflicts, events, sessions};
use super::Store;
use crate::coordination::domain::{
    query_window_limit, DashboardRecords, ProjectSummary, RecentCoordinationCounts, RecordWindow,
    StatusCounts,
};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use diesel::dsl::count_star;
use diesel::prelude::*;

#[cfg(all(test, feature = "sqlite"))]
mod project_tests;
mod projects;
pub(super) use projects::{
    discover_project, rebuild_catalog as rebuild_project_catalog, touch_project,
};

#[cfg(all(test, feature = "postgres"))]
mod tests;

impl Store {
    pub fn list_projects(&mut self) -> Result<Vec<ProjectSummary>> {
        Ok(self.list_projects_window()?.items)
    }

    pub(crate) fn list_projects_window(&mut self) -> Result<RecordWindow<ProjectSummary>> {
        let project_window = projects::recent_projects(&mut self.connection)?;
        let mut truncated = project_window.truncated;
        let mut projects = Vec::with_capacity(project_window.items.len());
        for projects::RecentProject {
            project_id,
            display_name,
            last_seen_at,
        } in project_window.items
        {
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
                display_name,
                worktrees,
                agents,
                last_seen_at,
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
        self.dashboard_window(
            project_key,
            event_limit,
            record_limit,
            activity_window_seconds,
            None,
        )
    }

    pub(crate) fn dashboard_window(
        &mut self,
        project_key: Option<&str>,
        event_limit: usize,
        record_limit: usize,
        activity_window_seconds: i64,
        since: Option<DateTime<Utc>>,
    ) -> Result<DashboardRecords> {
        let event_limit = event_limit.min(crate::coordination::api::MAX_DASHBOARD_EVENT_RESULTS);
        let record_limit = record_limit.min(crate::coordination::api::MAX_DASHBOARD_RECORD_RESULTS);
        let counts = self.dashboard_counts_window(project_key, activity_window_seconds, since)?;
        let (event_items, events_truncated) =
            self.load_event_window(project_key, event_limit, since)?;
        let events = RecordWindow {
            items: event_items,
            truncated: events_truncated,
        };

        let sessions = self.list_sessions_window(
            &crate::coordination::api::SessionQuery {
                project_id: project_key.map(str::to_owned),
                ..Default::default()
            },
            activity_window_seconds,
            record_limit,
        )?;
        let claims = self.list_claims_window(
            project_key,
            crate::coordination::domain::RecordScope::Active,
            record_limit,
        )?;

        let conflicts = self.live_conflicts_window(project_key, record_limit)?;
        Ok(DashboardRecords {
            counts,
            events,
            sessions,
            claims,
            conflicts,
        })
    }

    fn live_conflicts_window(
        &mut self,
        project_key: Option<&str>,
        record_limit: usize,
    ) -> Result<RecordWindow<crate::coordination::domain::ConflictRecord>> {
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
        Ok(RecordWindow::bounded(
            conflict_rows
                .into_iter()
                .map(conflict_from_row)
                .collect::<Result<Vec<_>>>()?,
            record_limit,
        ))
    }

    pub(super) fn dashboard_counts(
        &mut self,
        project_key: Option<&str>,
        activity_window_seconds: i64,
    ) -> Result<StatusCounts> {
        self.dashboard_counts_window(project_key, activity_window_seconds, None)
    }

    fn dashboard_counts_window(
        &mut self,
        project_key: Option<&str>,
        activity_window_seconds: i64,
        since: Option<DateTime<Utc>>,
    ) -> Result<StatusCounts> {
        let recent = self.recent_history_counts(project_key, since)?;
        self.live_counts(project_key, activity_window_seconds, recent, since)
    }

    fn live_counts(
        &mut self,
        project_key: Option<&str>,
        activity_window_seconds: i64,
        recent: RecentCoordinationCounts,
        since: Option<DateTime<Utc>>,
    ) -> Result<StatusCounts> {
        let counts = self.active_counts(project_key, activity_window_seconds, recent)?;
        let mut conflict_query = conflicts::table
            .filter(super::liveness::live_conflict())
            .into_boxed();
        if let Some(project_key) = project_key {
            conflict_query = conflict_query.filter(conflicts::project_id.eq(project_key));
        }
        Ok(StatusCounts {
            active_sessions: counts.active_sessions,
            active_claims: counts.active_claims,
            open_conflicts: conflict_query
                .select(count_star())
                .first(&mut self.connection)?,
            events: self.count_events(project_key, since)?,
            recent: counts.recent,
        })
    }

    pub(crate) fn coordination_summary(
        &mut self,
        project_key: Option<&str>,
        activity_window_seconds: i64,
        since: Option<DateTime<Utc>>,
    ) -> Result<crate::coordination::domain::CoordinationSummaryCounts> {
        let recent = self.recent_history_counts(project_key, since)?;
        self.active_counts(project_key, activity_window_seconds, recent)
    }

    fn active_counts(
        &mut self,
        project_key: Option<&str>,
        activity_window_seconds: i64,
        recent: RecentCoordinationCounts,
    ) -> Result<crate::coordination::domain::CoordinationSummaryCounts> {
        let mut session_query = sessions::table
            .filter(super::liveness::recent_session(activity_window_seconds))
            .into_boxed();
        let mut claim_query = claims::table.into_boxed();
        if let Some(project_key) = project_key {
            session_query = session_query.filter(sessions::project_id.eq(project_key));
            claim_query = claim_query.filter(claims::project_id.eq(project_key));
        }
        Ok(crate::coordination::domain::CoordinationSummaryCounts {
            active_sessions: session_query
                .select(count_star())
                .first(&mut self.connection)?,
            active_claims: claim_query
                .filter(claims::state.eq_any(["claimed", "modified"]))
                .filter(claims::expires_at.gt(Utc::now().to_rfc3339()))
                .select(count_star())
                .first(&mut self.connection)?,
            recent,
        })
    }

    fn count_events(
        &mut self,
        project_key: Option<&str>,
        since: Option<DateTime<Utc>>,
    ) -> Result<i64> {
        let mut query = events::table.into_boxed();
        if let Some(project_key) = project_key {
            query = query.filter(events::project_id.eq(project_key));
        }
        if let Some(since) = since {
            query = query.filter(events::created_at.ge(since.to_rfc3339()));
        }
        Ok(query.select(count_star()).first(&mut self.connection)?)
    }

    /// Conflicts first detected and advisories delivered within the recent history window.
    fn recent_history_counts(
        &mut self,
        project_key: Option<&str>,
        since: Option<DateTime<Utc>>,
    ) -> Result<RecentCoordinationCounts> {
        let since = since
            .unwrap_or_else(|| {
                Utc::now()
                    - Duration::days(crate::coordination::api::RECENT_COORDINATION_HISTORY_DAYS)
            })
            .to_rfc3339();
        let mut conflict_query = conflicts::table
            .filter(conflicts::created_at.ge(&since))
            .into_boxed();
        let mut advisory_query = advisories::table
            .filter(advisories::created_at.ge(&since))
            .into_boxed();
        if let Some(project_key) = project_key {
            conflict_query = conflict_query.filter(conflicts::project_id.eq(project_key));
            advisory_query = advisory_query.filter(advisories::project_id.eq(project_key));
        }
        Ok(RecentCoordinationCounts {
            recent_conflicts: conflict_query
                .select(count_star())
                .first(&mut self.connection)?,
            recent_advisories: advisory_query
                .select(count_star())
                .first(&mut self.connection)?,
        })
    }
}
