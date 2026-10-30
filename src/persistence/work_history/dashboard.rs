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

#[cfg(test)]
mod tests;

pub(super) fn dashboard(
    connection: &mut DbConnection,
    project: &str,
    session: Option<&str>,
    origin: WorkSessionOrigin,
) -> Result<WorkResponse> {
    ensure!(
        session.is_none_or(|id| id.len() <= 512),
        "work dashboard identity exceeds limit"
    );
    let (rows, truncated) = source_rows(connection, project, origin)?;
    let mut reader = projection::Projection::new(connection, project);
    let selected = session
        .map(|id| reader.summary(&WorkSubject::Session { id: id.into() }))
        .transpose()?
        .flatten();
    let mut projection = DashboardProjection::new(session, selected, truncated);
    projection.origin = origin;
    for page in rows.chunks(64) {
        if !projection.include_page(&mut reader, page)? {
            break;
        }
    }
    if let Some(session) = session {
        projection.selected_activities(connection, project, session)?;
    }
    projection.finish()
}
fn source_rows(
    connection: &mut DbConnection,
    project: &str,
    origin: WorkSessionOrigin,
) -> Result<(Vec<SubjectRow>, bool)> {
    let mut rows = Vec::new();
    let mut truncated = false;
    // A busy operation stream cannot evict long-lived roots from inspection.
    for scope in ["session", "activity"] {
        let mut query = work_subjects::table
            .filter(work_subjects::project_id.eq(project))
            .filter(work_subjects::scope.eq(scope))
            .into_boxed();
        if scope == "session" && origin == WorkSessionOrigin::Host {
            query = query.filter(work_subjects::has_host_capture.eq(true));
        }
        let mut page = query
            .order((work_subjects::recorded_at.desc(), work_subjects::id.asc()))
            .limit((SOURCE_LIMIT + 1) as i64)
            .select(SubjectRow::as_select())
            .load::<SubjectRow>(connection)?;
        truncated |= page.len() > SOURCE_LIMIT;
        page.truncate(SOURCE_LIMIT);
        rows.extend(page);
    }
    Ok((rows, truncated))
}

struct DashboardProjection {
    origin: WorkSessionOrigin,
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
            origin: WorkSessionOrigin::All,
            view: WorkDashboard {
                work_session_id: session.map(str::to_owned),
                selected_session: selected,
                sessions: vec![],
                activities: vec![],
                intervals: vec![],
                accounting: None,
                unlinked_activity_count: 0,
                session_duration: DurationStatistics::default(),
                activity_kinds: vec![],
                daily: vec![],
                activity_daily: vec![],
                scanned_subjects: 0,
                source_limit: SOURCE_LIMIT * 2,
                truncated,
                notices: if truncated {
                    vec!["source_subject_limit: most recently received 4096 roots and 4096 activities scanned independently; aggregates are partial".into()]
                } else {
                    vec![]
                },
            },
            kinds: BTreeMap::new(),
            dates: BTreeMap::new(),
            activity_dates: BTreeMap::new(),
            bytes: 0,
            unlinked: 0,
        }
    }
    fn include_page(
        &mut self,
        reader: &mut projection::Projection<'_>,
        page: &[SubjectRow],
    ) -> Result<bool> {
        let subjects: Vec<_> = page
            .iter()
            .map(|row| {
                if row.scope == "session" {
                    WorkSubject::Session { id: row.id.clone() }
                } else {
                    WorkSubject::Activity { id: row.id.clone() }
                }
            })
            .collect();
        reader.prefetch(&subjects)?;
        for (subject, row) in subjects.iter().zip(page) {
            if let Some(summary) = reader.summary(subject)? {
                if !self.include(summary, &row.occurred_at)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
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
        if self.origin == WorkSessionOrigin::Host
            && !summary.context.labels.contains_key("nexus.capture.host")
        {
            return;
        }
        self.view.session_duration.observe(&summary);
        self.dates.entry(date).or_default().observe(&summary);
        self.view.sessions.push(summary);
    }
    fn activity(&mut self, summary: WorkSummary, date: String) {
        if self.view.work_session_id.is_none()
            && self.origin == WorkSessionOrigin::Host
            && !self.view.sessions.iter().any(|root| {
                summary
                    .work_session_ids
                    .iter()
                    .any(|id| id == root.subject.id())
            })
        {
            return;
        }
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
        self.view.intervals = selected.items.iter().map(WorkIntervalNode::from).collect();
        self.view.accounting = self
            .view
            .selected_session
            .as_ref()
            .map(|root| session_accounting(root, &selected.items, selected.truncated));
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
        self.view.unlinked_activity_count = self.unlinked;
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
        let sources = self.bound_sources()?;
        let timeline = self.bound_timeline()?;
        let charts = self.bound_charts()?;
        let transport = self.bound_transport()?;
        if sources || timeline || charts || transport {
            self.view.truncated = true;
            self.view.notices.push("display_limit: chart rows or selectable sessions are partial; duration aggregates retain the scanned population".into());
        }
        Ok(())
    }
    fn bound_sources(&mut self) -> Result<bool> {
        let sessions = bound_items(&mut self.view.sessions, 512, 192 * 1024)?;
        let activities = bound_items(&mut self.view.activities, 512, 128 * 1024)?;
        Ok(sessions || activities)
    }
    fn bound_timeline(&mut self) -> Result<bool> {
        let intervals = bound_items(&mut self.view.intervals, SOURCE_LIMIT, 384 * 1024)?;
        let mut gaps = false;
        if let Some(accounting) = &mut self.view.accounting {
            gaps = bound_items(&mut accounting.unobserved_intervals, 4097, 64 * 1024)?;
            accounting.coverage.display_partial = intervals || gaps;
        }
        Ok(intervals || gaps)
    }
    fn bound_charts(&mut self) -> Result<bool> {
        let daily = bound_items(&mut self.view.daily, 4096, 64 * 1024)?;
        let kinds = bound_items(&mut self.view.activity_kinds, 4096, 64 * 1024)?;
        let activity_daily = bound_items(&mut self.view.activity_daily, 4096, 96 * 1024)?;
        Ok(daily || kinds || activity_daily)
    }
    fn bound_transport(&mut self) -> Result<bool> {
        let mut trimmed = false;
        while serde_json::to_vec(&self.view)?.len() > 896 * 1024 {
            // Secondary source rows are reconstructible by paginated reads. Preserve
            // the primary waterfall until those rows no longer consume the budget.
            if !self.view.sessions.is_empty() {
                self.view.sessions.truncate(self.view.sessions.len() / 2);
            } else if !self.view.activities.is_empty() {
                self.view
                    .activities
                    .truncate(self.view.activities.len() / 2);
            } else if !self.view.intervals.is_empty() {
                self.view.intervals.truncate(self.view.intervals.len() / 2);
            } else {
                anyhow::bail!("bounded work dashboard metadata exceeds transport budget");
            }
            trimmed = true;
        }
        if trimmed {
            if let Some(accounting) = &mut self.view.accounting {
                accounting.coverage.display_partial = true;
            }
            self.view.notices.push("transport_limit: display rows are partial; exact aggregates retain the selected source population".into());
        }
        Ok(trimmed)
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

#[cfg(test)]
mod transport_tests {
    use super::*;
    fn populate_display_rows(
        projection: &mut DashboardProjection,
        start: chrono::DateTime<chrono::Utc>,
    ) {
        for index in 0..3000 {
            let mut item = WorkSummary::unobserved(&WorkSubject::Activity {
                id: format!("task-{index}"),
            });
            item.title = "t".repeat(512);
            item.kind = format!("{index:04}{}", "k".repeat(508));
            item.context
                .labels
                .insert("metadata".into(), "v".repeat(1024));
            projection
                .view
                .intervals
                .push(WorkIntervalNode::from(&item));
            projection.view.activities.push(item.clone());
            projection
                .view
                .activity_kinds
                .push(ActivityDurationStatistics {
                    kind: item.kind.clone(),
                    duration: DurationStatistics::default(),
                });
            projection
                .view
                .activity_daily
                .push(DailyActivityStatistics {
                    date: "2026-10-27".into(),
                    kind: item.kind.clone(),
                    duration: DurationStatistics::default(),
                });
            if index < 512 {
                item.subject = WorkSubject::Session {
                    id: format!("session-{index}"),
                };
                projection.view.sessions.push(item);
                projection.view.daily.push(DailyDurationStatistics {
                    date: (start.date_naive() + chrono::Duration::days(index)).to_string(),
                    session_count: 1,
                    known_count: 1,
                    total_seconds: 1.0,
                    average_seconds: Some(1.0),
                });
            }
        }
    }
    #[test]
    fn bounded_dashboard_with_escaped_identities_fits_the_remote_response_budget() {
        let mut root = WorkSummary::unobserved(&WorkSubject::Session { id: "root".into() });
        root.work_session_ids = (0..64)
            .map(|index| format!("{index:02}{}", "\u{0001}".repeat(510)))
            .collect();
        let mut projection = DashboardProjection::new(Some("root"), Some(root.clone()), false);
        let start: chrono::DateTime<chrono::Utc> = "2026-10-27T00:00:00Z".parse().unwrap();
        populate_display_rows(&mut projection, start);
        let mut accounting = session_accounting(&root, &[], false);
        accounting.unobserved_intervals = (0..3000)
            .map(|index| AccountingInterval {
                started_at: start + chrono::Duration::seconds(index * 2),
                finished_at: start + chrono::Duration::seconds(index * 2 + 1),
            })
            .collect();
        accounting.coverage.observed_adapters = vec!["adapter".repeat(73); 32];
        projection.view.accounting = Some(accounting);
        projection.bound_output().unwrap();
        let bytes = serde_json::to_vec(&projection.view).unwrap().len();
        assert!(
            bytes <= 896 * 1024,
            "encoded dashboard size {bytes} needs headroom inside the 1MiB remote envelope"
        );
        assert!(projection.view.accounting.unwrap().coverage.display_partial);
    }
}
