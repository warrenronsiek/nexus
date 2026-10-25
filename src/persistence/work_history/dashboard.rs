// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::{paging::bound_items, projection, rows::SubjectRow};
use crate::persistence::{backend::DbConnection, schema::work_subjects};
use crate::work_history::*;
use anyhow::{ensure, Result};
use diesel::prelude::*;
use std::collections::BTreeMap;
pub(super) const SOURCE_LIMIT: usize = 4096;
pub(super) const SOURCE_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn dashboard(
    connection: &mut DbConnection,
    project: &str,
    session: Option<&str>,
) -> Result<WorkResponse> {
    ensure!(
        session.is_none_or(|id| id.len() <= 512),
        "work dashboard identity exceeds limit"
    );
    let (rows, truncated) = source_rows(connection, project)?;
    let selected = session
        .map(|id| projection::detail(connection, project, &WorkSubject::Session { id: id.into() }))
        .transpose()?
        .flatten()
        .map(|detail| detail.summary);
    let mut projection = DashboardProjection::new(session, selected, truncated);
    for row in rows {
        let subject = if row.scope == "session" {
            WorkSubject::Session { id: row.id }
        } else {
            WorkSubject::Activity { id: row.id }
        };
        if let Some(detail) = projection::detail(connection, project, &subject)? {
            if !projection.include(detail.summary, &row.occurred_at)? {
                break;
            }
        }
    }
    if let Some(session) = session {
        projection.selected_activities(connection, project, session)?;
    }
    projection.finish()
}
fn source_rows(connection: &mut DbConnection, project: &str) -> Result<(Vec<SubjectRow>, bool)> {
    let mut rows = work_subjects::table
        .filter(work_subjects::project_id.eq(project))
        .order((
            work_subjects::recorded_at.desc(),
            work_subjects::scope.asc(),
            work_subjects::id.asc(),
        ))
        .limit((SOURCE_LIMIT + 1) as i64)
        .select(SubjectRow::as_select())
        .load::<SubjectRow>(connection)?;
    let truncated = rows.len() > SOURCE_LIMIT;
    rows.truncate(SOURCE_LIMIT);
    Ok((rows, truncated))
}
struct DashboardProjection {
    view: WorkDashboard,
    kinds: BTreeMap<String, DurationStatistics>,
    dates: BTreeMap<String, DurationStatistics>,
    activity_dates: BTreeMap<(String, String), DurationStatistics>,
    bytes: usize,
    unlinked: usize,
}
impl DashboardProjection {
    fn new(session: Option<&str>, selected: Option<WorkSummary>, truncated: bool) -> Self {
        Self {
            view: WorkDashboard {
                work_session_id: session.map(str::to_owned),
                selected_session: selected,
                sessions: vec![],
                activities: vec![],
                session_duration: DurationStatistics::default(),
                activity_kinds: vec![],
                daily: vec![],
                activity_daily: vec![],
                scanned_subjects: 0,
                source_limit: SOURCE_LIMIT,
                truncated,
                notices: vec![],
            },
            kinds: BTreeMap::new(),
            dates: BTreeMap::new(),
            activity_dates: BTreeMap::new(),
            bytes: 0,
            unlinked: 0,
        }
    }
    fn include(&mut self, summary: WorkSummary, fallback: &str) -> Result<bool> {
        self.bytes += serde_json::to_vec(&summary)?.len();
        if self.bytes > SOURCE_BYTES {
            self.view.truncated = true;
            self.view
                .notices
                .push("source_byte_limit: history aggregates are partial".into());
            return Ok(false);
        }
        self.view.scanned_subjects += 1;
        let date = summary
            .started_at
            .map(|time| time.date_naive().to_string())
            .unwrap_or_else(|| fallback.chars().take(10).collect());
        match &summary.subject {
            WorkSubject::Session { .. } => self.session(summary, date),
            WorkSubject::Activity { .. } => self.activity(summary, date),
        }
        Ok(true)
    }
    fn session(&mut self, summary: WorkSummary, date: String) {
        self.view.session_duration.observe(&summary);
        self.dates.entry(date).or_default().observe(&summary);
        self.view.sessions.push(summary);
    }
    fn activity(&mut self, summary: WorkSummary, date: String) {
        if self
            .view
            .work_session_id
            .as_ref()
            .is_some_and(|id| !summary.work_session_ids.contains(id))
        {
            return;
        }
        if self.view.work_session_id.is_none() && summary.work_session_ids.is_empty() {
            self.unlinked += 1;
        }
        self.kinds
            .entry(summary.kind.clone())
            .or_default()
            .observe(&summary);
        self.activity_dates
            .entry((date, summary.kind.clone()))
            .or_default()
            .observe(&summary);
        if self.view.work_session_id.is_some() {
            self.view.activities.push(summary);
        }
    }
    fn selected_activities(
        &mut self,
        connection: &mut DbConnection,
        project: &str,
        session: &str,
    ) -> Result<()> {
        let selected = super::selection::activities(connection, project, session)?;
        self.kinds.clear();
        self.activity_dates.clear();
        self.view.activities.clear();
        if selected.truncated {
            self.view.truncated = true;
            self.view
                .notices
                .push("selected_activity_limit: selected descendant coverage is partial".into());
        }
        for summary in selected.items {
            let source_date = work_subjects::table
                .find((project, "activity", summary.subject.id()))
                .select(work_subjects::occurred_at)
                .first::<String>(connection)?;
            let date = summary
                .started_at
                .map(|time| time.date_naive().to_string())
                .unwrap_or_else(|| source_date.chars().take(10).collect());
            self.activity(summary, date);
        }
        Ok(())
    }
    fn finish(mut self) -> Result<WorkResponse> {
        if self.view.scanned_subjects == SOURCE_LIMIT && self.view.truncated {
            self.view.notices.push("source_subject_limit: most recently received 4096 subjects; aggregates are partial".into());
        }
        self.view.activity_kinds = std::mem::take(&mut self.kinds)
            .into_iter()
            .map(|(kind, duration)| ActivityDurationStatistics { kind, duration })
            .collect();
        self.view.daily = std::mem::take(&mut self.dates)
            .into_iter()
            .map(|(date, duration)| DailyDurationStatistics {
                date,
                session_count: duration.count,
                known_count: duration.known_count,
                total_seconds: duration.total_seconds,
                average_seconds: duration.average_seconds,
            })
            .collect();
        self.view.activity_daily = std::mem::take(&mut self.activity_dates)
            .into_iter()
            .map(|((date, kind), duration)| DailyActivityStatistics {
                date,
                kind,
                duration,
            })
            .collect();
        if self.unlinked > 0 {
            self.view.notices.push(format!("{} activities have no resolved session membership; overview activity statistics include them",self.unlinked));
        }
        self.bound_output()?;
        self.view.notices.push("Inclusive observed interval sums overlap for nested/parallel work; they are not effort or prompt-to-production elapsed time. Unknown/conflicted durations are excluded from means. Days use UTC start, or first source occurrence when start is missing.".into());
        Ok(WorkResponse::Dashboard {
            dashboard: Box::new(self.view),
        })
    }
    fn bound_output(&mut self) -> Result<()> {
        let sessions = bound_items(&mut self.view.sessions, 512, 192 * 1024)?;
        let activities = bound_items(&mut self.view.activities, 512, 256 * 1024)?;
        let daily = bound_items(&mut self.view.daily, 4096, 64 * 1024)?;
        let kinds = bound_items(&mut self.view.activity_kinds, 4096, 64 * 1024)?;
        let activity_daily = bound_items(&mut self.view.activity_daily, 4096, 96 * 1024)?;
        if sessions || activities || daily || kinds || activity_daily {
            self.view.truncated = true;
            self.view.notices.push("display_limit: chart rows or selectable sessions are partial; duration aggregates retain the scanned population".into());
        }
        Ok(())
    }
}
impl DurationStatistics {
    fn observe(&mut self, summary: &WorkSummary) {
        self.count += 1;
        let seconds = summary
            .elapsed
            .as_ref()
            .filter(|_| summary.conflicts.is_empty())
            .and_then(|elapsed| elapsed.nanoseconds.parse::<f64>().ok())
            .map(|ns| ns / 1_000_000_000.0)
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
        if let Some(seconds) = seconds {
            self.known_count += 1;
            self.total_seconds += seconds;
            self.average_seconds = Some(self.total_seconds / self.known_count as f64);
        }
    }
}
