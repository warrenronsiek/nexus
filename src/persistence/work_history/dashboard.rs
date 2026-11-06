// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::{projection, rows::SubjectRow};
use crate::persistence::{backend::DbConnection, schema::work_subjects};
use crate::work_history::*;
use anyhow::{ensure, Result};
use diesel::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};
pub(super) const SOURCE_LIMIT: usize = 4096;
pub(super) const SOURCE_BYTES: usize = 8 * 1024 * 1024;

mod bounds;
mod population;
mod retention;
mod sessions;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transport_tests;

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
    let sources = source_rows(connection, project, origin, session)?;
    let mut projection = DashboardProjection::new(session, sources.truncated, origin);
    projection.work_bearing = sources.work_bearing;
    if projection.include_roots(connection, project, &sources.roots)? {
        projection.include_population(connection, project, sources.activities)?;
    }
    if let Some(session) = session {
        projection.selected_activities(connection, project, session)?;
    }
    projection.finish()
}
struct DashboardSources {
    roots: Vec<SubjectRow>,
    work_bearing: HashSet<String>,
    activities: Vec<SubjectRow>,
    truncated: bool,
}
/// Roots are always a recency window. Only the unscoped all-origin overview also
/// samples the most recently received activities, so it can count unlinked work.
fn source_rows(
    connection: &mut DbConnection,
    project: &str,
    origin: WorkSessionOrigin,
    session: Option<&str>,
) -> Result<DashboardSources> {
    let (roots, roots_partial) = scope_rows(connection, project, "session", origin)?;
    let ids: Vec<_> = roots.iter().map(|row| row.id.clone()).collect();
    let work_bearing = population::work_bearing_roots(connection, project, &ids)?;
    let (activities, activities_partial) = if session.is_none() && origin == WorkSessionOrigin::All
    {
        scope_rows(connection, project, "activity", WorkSessionOrigin::All)?
    } else {
        (Vec::new(), false)
    };
    Ok(DashboardSources {
        roots,
        work_bearing,
        activities,
        truncated: roots_partial || activities_partial,
    })
}
fn scope_rows(
    connection: &mut DbConnection,
    project: &str,
    scope: &str,
    origin: WorkSessionOrigin,
) -> Result<(Vec<SubjectRow>, bool)> {
    let mut query = work_subjects::table
        .filter(work_subjects::project_id.eq(project))
        .filter(work_subjects::scope.eq(scope))
        .into_boxed();
    if origin == WorkSessionOrigin::Host {
        query = query.filter(work_subjects::has_host_capture.eq(true));
    }
    let mut rows = query
        .order((work_subjects::recorded_at.desc(), work_subjects::id.asc()))
        .limit((SOURCE_LIMIT + 1) as i64)
        .select(SubjectRow::as_select())
        .load::<SubjectRow>(connection)?;
    let truncated = rows.len() > SOURCE_LIMIT;
    rows.truncate(SOURCE_LIMIT);
    Ok((rows, truncated))
}

/// Duration statistics for one activity kind and the accounting roles it carries.
#[derive(Default)]
struct KindStatistics {
    duration: DurationStatistics,
    roles: BTreeSet<Option<String>>,
}

struct DashboardProjection {
    origin: WorkSessionOrigin,
    work_bearing: HashSet<String>,
    sessions: sessions::SessionRows,
    view: WorkDashboard,
    kinds: BTreeMap<String, KindStatistics>,
    dates: BTreeMap<String, DurationStatistics>,
    activity_dates: BTreeMap<(String, String), DurationStatistics>,
    activity_roots: HashSet<String>,
    bytes: usize,
    unlinked: usize,
}
impl DashboardProjection {
    fn new(session: Option<&str>, truncated: bool, origin: WorkSessionOrigin) -> Self {
        Self {
            origin,
            work_bearing: HashSet::new(),
            sessions: sessions::SessionRows::default(),
            view: WorkDashboard {
                work_session_id: session.map(str::to_owned),
                selected_session: None,
                sessions: vec![],
                hidden_session_count: 0,
                activities: vec![],
                intervals: vec![],
                accounting: None,
                unlinked_activity_count: 0,
                session_duration: DurationStatistics::default(),
                active_duration: DurationStatistics::default(),
                activity_session_count: 0,
                activity_kinds: vec![],
                daily: vec![],
                activity_daily: vec![],
                scanned_subjects: 0,
                source_limit: SOURCE_LIMIT * 2,
                truncated,
                notices: if truncated {
                    vec!["source_subject_limit: most recently received 4096 roots (and, for the all-origin overview, 4096 activities) scanned; aggregates are partial".into()]
                } else {
                    vec![]
                },
            },
            kinds: BTreeMap::new(),
            dates: BTreeMap::new(),
            activity_dates: BTreeMap::new(),
            activity_roots: HashSet::new(),
            bytes: 0,
            unlinked: 0,
        }
    }
    /// The selected root's detail, then the scanned roots; false when a source bound
    /// stopped the scan.
    fn include_roots(
        &mut self,
        connection: &mut DbConnection,
        project: &str,
        roots: &[SubjectRow],
    ) -> Result<bool> {
        let mut reader = projection::Projection::new(connection, project);
        if let Some(id) = self.view.work_session_id.clone() {
            self.view.selected_session = reader.summary(&WorkSubject::Session { id })?;
        }
        self.include_rows(&mut reader, roots)
    }
    fn include_rows(
        &mut self,
        reader: &mut projection::Projection<'_>,
        rows: &[SubjectRow],
    ) -> Result<bool> {
        for page in rows.chunks(64) {
            if !self.include_page(reader, page)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn include_page(
        &mut self,
        reader: &mut projection::Projection<'_>,
        page: &[SubjectRow],
    ) -> Result<bool> {
        let subjects = subjects(page);
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
    /// Request timing for every visible root, then the activity population: whole
    /// visible harness roots, or the recency window for the all-origin overview.
    fn include_population(
        &mut self,
        connection: &mut DbConnection,
        project: &str,
        recent: Vec<SubjectRow>,
    ) -> Result<()> {
        let roots = self.sessions.root_ids();
        let (requests, requests_partial) = population::request_rows(connection, project, &roots)?;
        if requests_partial {
            self.disclose("request_limit: active request time covers the first 4096 requests of visible sessions");
        }
        let activities = match (self.view.work_session_id.is_some(), self.origin) {
            (true, _) => Vec::new(),
            (false, WorkSessionOrigin::All) => recent,
            (false, WorkSessionOrigin::Host) => {
                let sample = population::member_sample(connection, project, &roots)?;
                if sample.truncated {
                    self.disclose("activity_session_limit: activity statistics cover the newest visible sessions whose combined activities fit 4096 rows; see activity_session_count");
                }
                sample.rows
            }
        };
        let mut reader = projection::Projection::new(connection, project);
        for page in requests.chunks(64) {
            let subjects = subjects(page);
            reader.prefetch(&subjects)?;
            for subject in &subjects {
                if let Some(request) = reader.summary(subject)? {
                    self.sessions.observe_request(&request);
                }
            }
        }
        self.include_rows(&mut reader, &activities)?;
        Ok(())
    }
    fn disclose(&mut self, notice: &str) {
        self.view.truncated = true;
        self.view.notices.push(notice.into());
    }
    fn include(&mut self, summary: WorkSummary, fallback: &str) -> Result<bool> {
        self.bytes += serde_json::to_vec(&summary)?.len();
        if self.bytes > SOURCE_BYTES {
            self.disclose("source_byte_limit: history aggregates are partial");
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
    /// Harness roots that recorded only host containers are hidden, not deleted.
    fn session(&mut self, summary: WorkSummary, date: String) {
        let host = summary.context.labels.contains_key("nexus.capture.host");
        if self.origin == WorkSessionOrigin::Host && !host {
            return;
        }
        if host && !self.work_bearing.contains(summary.subject.id()) {
            self.view.hidden_session_count += 1;
            return;
        }
        self.view.session_duration.observe(&summary);
        self.dates
            .entry(date.clone())
            .or_default()
            .observe(&summary);
        self.sessions.push(summary, date);
    }
    fn activity(&mut self, summary: WorkSummary, date: String) {
        let roots: Vec<_> = match &self.view.work_session_id {
            Some(id) if summary.work_session_ids.contains(id) => vec![id.clone()],
            Some(_) => return,
            None if self.origin == WorkSessionOrigin::Host => summary
                .work_session_ids
                .iter()
                .filter(|id| self.sessions.contains(id))
                .cloned()
                .collect(),
            None => summary.work_session_ids.clone(),
        };
        if roots.is_empty() && self.origin == WorkSessionOrigin::Host {
            return;
        }
        if self.view.work_session_id.is_none() && summary.work_session_ids.is_empty() {
            self.unlinked += 1;
        }
        self.activity_roots.extend(roots);
        let kind = self.kinds.entry(summary.kind.clone()).or_default();
        kind.duration.observe(&summary);
        kind.roles
            .insert(summary.context.labels.get("nexus.accounting.role").cloned());
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
        self.activity_roots.clear();
        self.view.activities.clear();
        if selected.truncated {
            self.disclose("selected_activity_limit: selected descendant coverage is partial");
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
        let (rows, active, mut active_dates) = std::mem::take(&mut self.sessions).finish();
        self.view.sessions = rows;
        self.view.active_duration = active;
        self.view.activity_session_count = self.activity_roots.len();
        self.view.activity_kinds = std::mem::take(&mut self.kinds)
            .into_iter()
            .map(|(kind, statistics)| ActivityDurationStatistics {
                kind,
                accounting_role: shared_role(statistics.roles),
                duration: statistics.duration,
            })
            .collect();
        self.view.daily = std::mem::take(&mut self.dates)
            .into_iter()
            .map(|(date, duration)| {
                let active = active_dates.remove(&date).unwrap_or_default();
                DailyDurationStatistics {
                    date,
                    session_count: duration.count,
                    known_count: duration.known_count,
                    total_seconds: duration.total_seconds,
                    average_seconds: duration.average_seconds,
                    active_known_count: active.known_count,
                    active_total_seconds: active.total_seconds,
                    active_average_seconds: active.average_seconds,
                }
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
        self.view.notices.push("Inclusive observed interval sums overlap for nested/parallel work; they are not effort or prompt-to-production elapsed time. Session duration is the host lifetime, including idle time; active time sums timed human requests. Unknown/conflicted durations are excluded from means. Days use UTC start, or first source occurrence when start is missing.".into());
        Ok(WorkResponse::Dashboard {
            dashboard: Box::new(self.view),
        })
    }
}

fn subjects(page: &[SubjectRow]) -> Vec<WorkSubject> {
    page.iter()
        .map(|row| {
            if row.scope == "session" {
                WorkSubject::Session { id: row.id.clone() }
            } else {
                WorkSubject::Activity { id: row.id.clone() }
            }
        })
        .collect()
}

fn shared_role(roles: BTreeSet<Option<String>>) -> Option<String> {
    let mut roles = roles.into_iter();
    match (roles.next(), roles.next()) {
        (Some(role), None) => role,
        (Some(_), Some(_)) => Some("mixed".into()),
        (None, _) => None,
    }
}
