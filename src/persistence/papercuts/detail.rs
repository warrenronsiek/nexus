// @feature papercuts
// @feature persistence
// @spec docs/features/papercuts.md
// @spec docs/features/persistence.md
use super::super::schema::{papercut_upvotes, papercuts};
use super::super::Store;
use super::records::{load_canonical_papercut, validate_project, PapercutUpvoteRow};
use super::registry::load_papercut;
use crate::coordination::domain::query_window_limit;
use crate::papercuts::{PapercutDetail, PapercutReport, PapercutReportKind};
use crate::persistence::backend::DbConnection;
use anyhow::Result;
use diesel::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_PAPERCUT_DETAIL_REPORTS: usize = 64;
/// Keeps an encoded detail inside one 1 MiB response frame.
pub(super) const MAX_PAPERCUT_DETAIL_BYTES: usize = 768 * 1024;

/// The creating report of a papercut: its text and the timestamp its occurrence shares.
struct Origin {
    title: String,
    description: String,
    created_at: String,
}

impl Store {
    /// Resolves consolidation to the canonical papercut and returns its newest bounded
    /// occurrences in chronological order.
    pub fn papercut_detail(
        &mut self,
        project_id: &str,
        papercut_id: &str,
    ) -> Result<PapercutDetail> {
        validate_project(project_id)?;
        let connection = &mut self.connection;
        let canonical = load_canonical_papercut(connection, project_id, papercut_id)?;
        let papercut = load_papercut(connection, project_id, &canonical.id)?;
        let (reports, truncated) = load_reports(connection, project_id, &canonical.id)?;
        bound_detail_bytes(PapercutDetail {
            papercut,
            reports,
            truncated,
        })
    }
}

fn load_reports(
    connection: &mut DbConnection,
    project_id: &str,
    canonical_id: &str,
) -> Result<(Vec<PapercutReport>, bool)> {
    let (occurrences, truncated) = newest_occurrences(connection, project_id, canonical_id)?;
    let origins = report_origins(connection, project_id, &occurrences)?;
    let reports = occurrences
        .into_iter()
        .map(|row| occurrence_report(row, &origins))
        .collect::<Result<Vec<_>>>()?;
    Ok((reports, truncated))
}

fn newest_occurrences(
    connection: &mut DbConnection,
    project_id: &str,
    canonical_id: &str,
) -> Result<(Vec<PapercutUpvoteRow>, bool)> {
    let mut rows = papercut_upvotes::table
        .filter(papercut_upvotes::project_id.eq(project_id))
        .filter(papercut_upvotes::papercut_id.eq(canonical_id))
        .order((
            papercut_upvotes::created_at.desc(),
            papercut_upvotes::id.desc(),
        ))
        .limit(query_window_limit(MAX_PAPERCUT_DETAIL_REPORTS))
        .select(PapercutUpvoteRow::as_select())
        .load::<PapercutUpvoteRow>(connection)?;
    let truncated = rows.len() > MAX_PAPERCUT_DETAIL_REPORTS;
    rows.truncate(MAX_PAPERCUT_DETAIL_REPORTS);
    rows.reverse();
    Ok((rows, truncated))
}

/// Consolidation moves a duplicate's occurrences to its canonical papercut while retaining
/// `reported_papercut_id`, so the reported papercuts supply each creating report's text.
fn report_origins(
    connection: &mut DbConnection,
    project_id: &str,
    occurrences: &[PapercutUpvoteRow],
) -> Result<BTreeMap<String, Origin>> {
    let reported_ids = occurrences
        .iter()
        .map(|row| row.reported_papercut_id.as_str())
        .collect::<BTreeSet<_>>();
    Ok(papercuts::table
        .filter(papercuts::project_id.eq(project_id))
        .filter(papercuts::id.eq_any(reported_ids))
        .select((
            papercuts::id,
            papercuts::title,
            papercuts::description,
            papercuts::created_at,
        ))
        .load::<(String, String, String, String)>(connection)?
        .into_iter()
        .map(|(id, title, description, created_at)| {
            let origin = Origin {
                title,
                description,
                created_at,
            };
            (id, origin)
        })
        .collect())
}

fn occurrence_report(
    row: PapercutUpvoteRow,
    origins: &BTreeMap<String, Origin>,
) -> Result<PapercutReport> {
    let origin = origins
        .get(&row.reported_papercut_id)
        .filter(|origin| origin.created_at == row.created_at);
    let upvote = row.into_domain()?;
    Ok(PapercutReport {
        kind: if origin.is_some() {
            PapercutReportKind::Report
        } else {
            PapercutReportKind::Encounter
        },
        title: origin.map(|origin| origin.title.clone()),
        description: origin.map(|origin| origin.description.clone()),
        id: upvote.id,
        reported_papercut_id: upvote.reported_papercut_id,
        note: upvote.note,
        reporter_agent: upvote.reporter_agent,
        reporter_session_id: upvote.reporter_session_id,
        reporter_model_id: upvote.reporter_model_id,
        created_at: upvote.created_at,
    })
}

/// JSON escaping can expand control characters, so the encoded detail is bounded directly by
/// removing the oldest occurrences first.
fn bound_detail_bytes(mut detail: PapercutDetail) -> Result<PapercutDetail> {
    while !detail.reports.is_empty()
        && serde_json::to_vec(&detail)?.len() > MAX_PAPERCUT_DETAIL_BYTES
    {
        detail.reports.remove(0);
        detail.truncated = true;
    }
    Ok(detail)
}
