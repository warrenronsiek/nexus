// @feature sos
// @feature persistence
// @spec docs/features/sos.md
// @spec docs/features/persistence.md
use super::records::{
    decode_issue, SosIssueRow, SosLifecycleRow, SosReportRow, SosTriageAttemptRow,
};
use crate::persistence::backend::{DbBackend, DbConnection};
use crate::persistence::models::parse_time;
use crate::persistence::schema::{
    sos_issue_lifecycle, sos_issues, sos_reports, sos_triage_attempts,
};
use crate::persistence::Store;
use crate::sos::{
    SosIssue, SosIssueDetail, SosIssueLifecycleEvent, SosIssueWindow, SosRegistry, SosReport,
    SosTriageAttempt, SosTriageCandidate,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use diesel::dsl::{count_star, max, min};
use diesel::prelude::*;
use serde::Serialize;

pub(crate) const MAX_SOS_LIST_ISSUES: usize = 32;
pub(crate) const MAX_SOS_TRIAGE_CANDIDATES: usize = 64;
pub(crate) const MAX_READY_SOS_REPORTS: usize = 32;
const READY_SCAN_MULTIPLIER: usize = 4;
pub(super) const MAX_SOS_DETAIL_REPORTS: usize = 4;
pub(super) const MAX_SOS_DETAIL_ATTEMPTS: usize = 8;
const MAX_SOS_DETAIL_LIFECYCLE_EVENTS: usize = 16;
pub(super) const MAX_SOS_DETAIL_SERIALIZED_BYTES: usize = 768 * 1024;

impl Store {
    pub fn open_sos_issues_window(&mut self, project_id: &str) -> Result<SosIssueWindow> {
        let mut rows = sos_issues::table
            .filter(sos_issues::project_id.eq(project_id))
            .filter(sos_issues::status.eq("open"))
            .order((sos_issues::updated_at.desc(), sos_issues::id.asc()))
            .limit(i64::try_from(MAX_SOS_LIST_ISSUES + 1)?)
            .select(SosIssueRow::as_select())
            .load::<SosIssueRow>(&mut self.connection)?;
        let truncated = rows.len() > MAX_SOS_LIST_ISSUES;
        rows.truncate(MAX_SOS_LIST_ISSUES);
        let issues = rows
            .into_iter()
            .map(|row| issue_from_row(&mut self.connection, row))
            .collect::<Result<Vec<_>>>()?;
        let untriaged = sos_reports::table
            .filter(sos_reports::project_id.eq(project_id))
            .filter(sos_reports::issue_id.is_null())
            .select(count_star())
            .first::<i64>(&mut self.connection)?;
        Ok(SosIssueWindow {
            issues,
            untriaged_report_count: u64::try_from(untriaged)
                .context("negative untriaged SOS report count")?,
            truncated,
        })
    }

    pub fn sos_issue_detail(&mut self, project_id: &str, issue_id: &str) -> Result<SosIssueDetail> {
        let issue = load_issue(&mut self.connection, project_id, issue_id)?;
        let (reports, reports_truncated) =
            load_reports_for_issue(&mut self.connection, project_id, issue_id)?;
        let (triage_attempts, triage_attempts_truncated) =
            load_attempts_for_issue(&mut self.connection, project_id, issue_id)?;
        let (lifecycle_events, lifecycle_events_truncated) =
            load_lifecycle_for_issue(&mut self.connection, project_id, issue_id)?;
        bound_detail_payload(SosIssueDetail {
            issue,
            reports,
            reports_truncated,
            triage_attempts,
            triage_attempts_truncated,
            lifecycle_events,
            lifecycle_events_truncated,
        })
    }

    pub fn sos_registry(&mut self, project_id: Option<&str>) -> Result<SosRegistry> {
        let issues = load_registry_issues(&mut self.connection, project_id)?;
        let reports = load_registry_reports(&mut self.connection, project_id)?;
        let triage_attempts = load_registry_attempts(&mut self.connection, project_id)?;
        let lifecycle_events = load_registry_lifecycle(&mut self.connection, project_id)?;
        Ok(SosRegistry {
            generated_at: Utc::now(),
            truncated: false,
            issues,
            reports,
            triage_attempts,
            lifecycle_events,
        })
    }

    pub(crate) fn sos_triage_candidates(
        &mut self,
        project_id: &str,
    ) -> Result<Vec<SosTriageCandidate>> {
        sos_issues::table
            .filter(sos_issues::project_id.eq(project_id))
            .order((sos_issues::updated_at.desc(), sos_issues::id.asc()))
            .limit(i64::try_from(MAX_SOS_TRIAGE_CANDIDATES)?)
            .select(SosIssueRow::as_select())
            .load::<SosIssueRow>(&mut self.connection)?
            .into_iter()
            .map(|row| issue_from_row(&mut self.connection, row).map(|issue| (&issue).into()))
            .collect()
    }

    pub(crate) fn ready_sos_reports(&mut self) -> Result<Vec<(String, String)>> {
        self.ready_sos_reports_at(Utc::now())
    }

    pub(crate) fn ready_sos_reports_at(
        &mut self,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, String)>> {
        let Some(rows) = load_recovery_page(&mut self.connection, now)? else {
            return Ok(Vec::new());
        };
        collect_ready_reports(&mut self.connection, rows, now)
    }
}

fn bound_detail_payload(mut detail: SosIssueDetail) -> Result<SosIssueDetail> {
    let mut encoded_len = serde_json::to_vec(&detail)
        .context("encode bounded SOS issue detail")?
        .len();
    while encoded_len > MAX_SOS_DETAIL_SERIALIZED_BYTES {
        let removed_len = truncate_detail_once(&mut detail)?;
        encoded_len = encoded_len.saturating_sub(removed_len);
    }
    Ok(detail)
}

#[derive(Clone, Copy)]
enum DetailSection {
    Reports,
    Attempts,
    Lifecycle,
}

fn truncation_section(detail: &SosIssueDetail) -> Option<DetailSection> {
    [
        (detail.reports.len(), DetailSection::Reports),
        (detail.triage_attempts.len(), DetailSection::Attempts),
        (detail.lifecycle_events.len(), DetailSection::Lifecycle),
    ]
    .into_iter()
    .find(|(length, _)| *length > 1)
    .or_else(|| {
        [
            (detail.reports.len(), DetailSection::Reports),
            (detail.triage_attempts.len(), DetailSection::Attempts),
            (detail.lifecycle_events.len(), DetailSection::Lifecycle),
        ]
        .into_iter()
        .find(|(length, _)| *length > 0)
    })
    .map(|(_, section)| section)
}

fn truncate_detail_once(detail: &mut SosIssueDetail) -> Result<usize> {
    match truncation_section(detail) {
        Some(DetailSection::Reports) => {
            detail.reports_truncated = true;
            remove_oldest_encoded(&mut detail.reports)
        }
        Some(DetailSection::Attempts) => {
            detail.triage_attempts_truncated = true;
            remove_oldest_encoded(&mut detail.triage_attempts)
        }
        Some(DetailSection::Lifecycle) => {
            detail.lifecycle_events_truncated = true;
            remove_oldest_encoded(&mut detail.lifecycle_events)
        }
        None => {
            anyhow::bail!("SOS issue detail exceeds {MAX_SOS_DETAIL_SERIALIZED_BYTES}-byte limit")
        }
    }
}

fn remove_oldest_encoded<T: Serialize>(values: &mut Vec<T>) -> Result<usize> {
    let separator_len = usize::from(values.len() > 1);
    let value = values.remove(0);
    Ok(serde_json::to_vec(&value)
        .context("encode truncated SOS issue detail row")?
        .len()
        + separator_len)
}

pub(super) fn sos_retry_cooldown(failure_count: u64) -> Duration {
    if failure_count == 0 {
        return Duration::zero();
    }
    const BASE_SECONDS: i64 = 5 * 60;
    const MAX_SECONDS: i64 = 24 * 60 * 60;
    let exponent = u32::try_from(failure_count.saturating_sub(1).min(16)).unwrap_or(16);
    let multiplier = 1_i64.checked_shl(exponent).unwrap_or(i64::MAX);
    Duration::seconds(BASE_SECONDS.saturating_mul(multiplier).min(MAX_SECONDS))
}

pub(super) fn load_issue(
    connection: &mut DbConnection,
    project_id: &str,
    issue_id: &str,
) -> Result<SosIssue> {
    let row = sos_issues::table
        .filter(sos_issues::project_id.eq(project_id))
        .find(issue_id)
        .select(SosIssueRow::as_select())
        .first::<SosIssueRow>(connection)
        .optional()?
        .with_context(|| format!("unknown SOS issue {issue_id}"))?;
    issue_from_row(connection, row)
}

fn issue_from_row(connection: &mut DbConnection, row: SosIssueRow) -> Result<SosIssue> {
    let (first_reported, last_reported, importance) = sos_reports::table
        .filter(sos_reports::project_id.eq(&row.project_id))
        .filter(sos_reports::issue_id.eq(Some(&row.id)))
        .select((
            min(sos_reports::created_at),
            max(sos_reports::created_at),
            count_star(),
        ))
        .first::<(Option<String>, Option<String>, i64)>(connection)?;
    decode_issue(
        row,
        first_reported.context("SOS issue has no source reports")?,
        last_reported.context("SOS issue has no source reports")?,
        importance,
    )
}

fn decode_bounded_rows<Row, Value>(
    mut rows: Vec<Row>,
    maximum: usize,
    decode: impl FnMut(Row) -> Result<Value>,
) -> Result<(Vec<Value>, bool)> {
    let truncated = rows.len() > maximum;
    rows.truncate(maximum);
    rows.reverse();
    Ok((
        rows.into_iter().map(decode).collect::<Result<Vec<_>>>()?,
        truncated,
    ))
}

macro_rules! bounded_issue_loader {
    ($name:ident, nullable $table:ident, $row:ty, $result:ty, $maximum:ident, $decode:path) => {
        fn $name(
            connection: &mut DbConnection,
            project_id: &str,
            issue_id: &str,
        ) -> Result<(Vec<$result>, bool)> {
            let rows = $table::table
                .filter($table::project_id.eq(project_id))
                .filter($table::issue_id.eq(Some(issue_id)))
                .order(($table::created_at.desc(), $table::id.desc()))
                .limit(i64::try_from($maximum + 1)?)
                .select(<$row>::as_select())
                .load::<$row>(connection)?;
            decode_bounded_rows(rows, $maximum, $decode)
        }
    };
    ($name:ident, required $table:ident, $row:ty, $result:ty, $maximum:ident, $decode:path) => {
        fn $name(
            connection: &mut DbConnection,
            project_id: &str,
            issue_id: &str,
        ) -> Result<(Vec<$result>, bool)> {
            let rows = $table::table
                .filter($table::project_id.eq(project_id))
                .filter($table::issue_id.eq(issue_id))
                .order(($table::created_at.desc(), $table::id.desc()))
                .limit(i64::try_from($maximum + 1)?)
                .select(<$row>::as_select())
                .load::<$row>(connection)?;
            decode_bounded_rows(rows, $maximum, $decode)
        }
    };
}

bounded_issue_loader!(
    load_reports_for_issue,
    nullable sos_reports,
    SosReportRow,
    SosReport,
    MAX_SOS_DETAIL_REPORTS,
    SosReportRow::decode
);

fn load_attempts_for_issue(
    connection: &mut DbConnection,
    project_id: &str,
    issue_id: &str,
) -> Result<(Vec<crate::sos::SosTriageAttempt>, bool)> {
    let rows = sos_triage_attempts::table
        .inner_join(sos_reports::table.on(sos_triage_attempts::report_id.eq(sos_reports::id)))
        .filter(sos_triage_attempts::project_id.eq(project_id))
        .filter(sos_reports::project_id.eq(project_id))
        .filter(sos_reports::issue_id.eq(Some(issue_id)))
        .order((
            sos_triage_attempts::created_at.desc(),
            sos_triage_attempts::id.desc(),
        ))
        .limit(i64::try_from(MAX_SOS_DETAIL_ATTEMPTS + 1)?)
        .select(SosTriageAttemptRow::as_select())
        .load::<SosTriageAttemptRow>(connection)?;
    decode_bounded_rows(rows, MAX_SOS_DETAIL_ATTEMPTS, SosTriageAttemptRow::decode)
}

bounded_issue_loader!(
    load_lifecycle_for_issue,
    required sos_issue_lifecycle,
    SosLifecycleRow,
    SosIssueLifecycleEvent,
    MAX_SOS_DETAIL_LIFECYCLE_EVENTS,
    SosLifecycleRow::decode
);

fn load_registry_issues(
    connection: &mut DbConnection,
    project_id: Option<&str>,
) -> Result<Vec<SosIssue>> {
    let mut query = sos_issues::table.into_boxed::<DbBackend>();
    if let Some(project_id) = project_id {
        query = query.filter(sos_issues::project_id.eq(project_id));
    }
    query
        .order((
            sos_issues::project_id.asc(),
            sos_issues::created_at.asc(),
            sos_issues::id.asc(),
        ))
        .select(SosIssueRow::as_select())
        .load::<SosIssueRow>(connection)?
        .into_iter()
        .map(|row| issue_from_row(connection, row))
        .collect()
}

macro_rules! registry_loader {
    ($name:ident, $table:ident, $row:ty, $result:ty, $decode:path) => {
        fn $name(connection: &mut DbConnection, project_id: Option<&str>) -> Result<Vec<$result>> {
            let mut query = $table::table.into_boxed::<DbBackend>();
            if let Some(project_id) = project_id {
                query = query.filter($table::project_id.eq(project_id));
            }
            query
                .order((
                    $table::project_id.asc(),
                    $table::created_at.asc(),
                    $table::id.asc(),
                ))
                .select(<$row>::as_select())
                .load::<$row>(connection)?
                .into_iter()
                .map($decode)
                .collect()
        }
    };
}

registry_loader!(
    load_registry_reports,
    sos_reports,
    SosReportRow,
    SosReport,
    SosReportRow::decode
);
registry_loader!(
    load_registry_attempts,
    sos_triage_attempts,
    SosTriageAttemptRow,
    SosTriageAttempt,
    SosTriageAttemptRow::decode
);
registry_loader!(
    load_registry_lifecycle,
    sos_issue_lifecycle,
    SosLifecycleRow,
    SosIssueLifecycleEvent,
    SosLifecycleRow::decode
);

fn load_recovery_page(
    connection: &mut DbConnection,
    now: DateTime<Utc>,
) -> Result<Option<Vec<(String, String)>>> {
    let untriaged_count = untriaged_report_count(connection)?;
    if untriaged_count == 0 {
        return Ok(None);
    }
    let offset = recovery_page_offset(untriaged_count, now)?;
    Ok(Some(load_untriaged_page(connection, offset)?))
}

fn untriaged_report_count(connection: &mut DbConnection) -> Result<usize> {
    let count = sos_reports::table
        .filter(sos_reports::issue_id.is_null())
        .select(count_star())
        .first::<i64>(connection)?;
    usize::try_from(count).context("negative untriaged SOS report count")
}

fn recovery_page_offset(untriaged_count: usize, now: DateTime<Utc>) -> Result<i64> {
    let scan_limit = MAX_READY_SOS_REPORTS * READY_SCAN_MULTIPLIER;
    let page_count = i64::try_from(untriaged_count.div_ceil(scan_limit))?;
    let page = now.timestamp().div_euclid(60).rem_euclid(page_count);
    page.checked_mul(i64::try_from(scan_limit)?)
        .context("SOS recovery offset overflow")
}

fn load_untriaged_page(
    connection: &mut DbConnection,
    offset: i64,
) -> Result<Vec<(String, String)>> {
    let scan_limit = i64::try_from(MAX_READY_SOS_REPORTS * READY_SCAN_MULTIPLIER)?;
    Ok(sos_reports::table
        .filter(sos_reports::issue_id.is_null())
        .order((sos_reports::created_at.asc(), sos_reports::id.asc()))
        .offset(offset)
        .limit(scan_limit)
        .select((sos_reports::project_id, sos_reports::id))
        .load::<(String, String)>(connection)?)
}

fn collect_ready_reports(
    connection: &mut DbConnection,
    rows: Vec<(String, String)>,
    now: DateTime<Utc>,
) -> Result<Vec<(String, String)>> {
    let mut ready = Vec::new();
    for (project_id, report_id) in rows {
        if report_is_ready(connection, &project_id, &report_id, now)? {
            ready.push((project_id, report_id));
            if ready.len() == MAX_READY_SOS_REPORTS {
                break;
            }
        }
    }
    Ok(ready)
}

fn report_is_ready(
    connection: &mut DbConnection,
    project_id: &str,
    report_id: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let (attempt_count, last_attempt) = sos_triage_attempts::table
        .filter(sos_triage_attempts::project_id.eq(project_id))
        .filter(sos_triage_attempts::report_id.eq(report_id))
        .select((count_star(), max(sos_triage_attempts::created_at)))
        .first::<(i64, Option<String>)>(connection)?;
    let cooldown = sos_retry_cooldown(u64::try_from(attempt_count)?);
    let last_attempt = last_attempt.as_deref().map(parse_time).transpose()?;
    Ok(last_attempt.is_none_or(|last| now.signed_duration_since(last) >= cooldown))
}
