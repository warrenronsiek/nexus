// @feature papercuts
// @feature persistence
// @spec docs/features/papercuts.md
// @spec docs/features/persistence.md
use super::super::models::{parse_time, EventRow};
use super::super::schema::{
    events, papercut_duplicate_reviews, papercut_review_attempts, papercut_upvotes, papercuts,
};
use super::super::Store;
use super::records::{
    load_papercut_row, validate_project, DuplicateReviewRow, PapercutReviewAttemptRow, PapercutRow,
    PapercutUpvoteRow,
};
use crate::coordination::domain::{query_window_limit, RecordWindow};
use crate::papercuts::{
    Papercut, PapercutActor, PapercutLifecycleEvent, PapercutLifecycleTransition, PapercutRegistry,
    PapercutReviewCandidate, PapercutStatus,
};
use crate::persistence::backend::DbConnection;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::str::FromStr;

pub(super) const MAX_PAPERCUT_REVIEW_CANDIDATES: usize = 64;

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = papercuts)]
struct PapercutReviewCandidateRow {
    id: String,
    project_id: String,
    title: String,
    description: String,
    status: String,
    consolidated_into: Option<String>,
    created_at: String,
    updated_at: String,
}

struct RegistryRows {
    papercuts: Vec<PapercutRow>,
    upvotes: Vec<PapercutUpvoteRow>,
    reviews: Vec<DuplicateReviewRow>,
    attempts: Vec<PapercutReviewAttemptRow>,
    lifecycle_events: Vec<PapercutLifecycleEvent>,
}

struct SingleParentRows {
    upvotes: Vec<PapercutUpvoteRow>,
    attempts: Vec<PapercutReviewAttemptRow>,
}

type VoteMetrics = (u64, Option<DateTime<Utc>>);
type VoteMetricsByPapercut = BTreeMap<String, VoteMetrics>;

const LIFECYCLE_EVENT_KINDS: [&str; 3] = [
    "papercut_resolved",
    "papercut_reopened",
    "papercut_consolidated",
];

#[derive(Deserialize)]
struct LifecyclePayload {
    papercut_id: String,
    #[serde(default)]
    resolution: Option<String>,
    #[serde(default)]
    canonical_id: Option<String>,
    #[serde(default)]
    rationale: Option<String>,
    actor: PapercutActor,
}

impl Store {
    pub fn papercut_registry(&mut self, project_id: Option<&str>) -> Result<PapercutRegistry> {
        if let Some(project_id) = project_id {
            validate_project(project_id)?;
        }
        load_registry(&mut self.connection, project_id)
    }

    pub fn open_papercuts(&mut self, project_id: &str) -> Result<Vec<Papercut>> {
        Ok(self.open_papercuts_window(project_id)?.items)
    }

    pub(crate) fn open_papercuts_window(
        &mut self,
        project_id: &str,
    ) -> Result<RecordWindow<Papercut>> {
        validate_project(project_id)?;
        load_open_papercuts(&mut self.connection, project_id)
    }

    pub fn papercut_by_id(&mut self, project_id: &str, papercut_id: &str) -> Result<Papercut> {
        load_papercut(&mut self.connection, project_id, papercut_id)
    }

    pub(crate) fn papercut_review_candidates(
        &mut self,
        project_id: &str,
        source_id: &str,
    ) -> Result<Vec<PapercutReviewCandidate>> {
        validate_project(project_id)?;
        load_review_candidates(&mut self.connection, project_id, source_id)
    }
}

fn load_review_candidates(
    connection: &mut DbConnection,
    project_id: &str,
    source_id: &str,
) -> Result<Vec<PapercutReviewCandidate>> {
    let rows = papercuts::table
        .filter(papercuts::project_id.eq(project_id))
        .filter(papercuts::status.eq("open"))
        .filter(papercuts::id.ne(source_id))
        .order((papercuts::updated_at.desc(), papercuts::id.asc()))
        .limit(i64::try_from(MAX_PAPERCUT_REVIEW_CANDIDATES)?)
        .select(PapercutReviewCandidateRow::as_select())
        .load::<PapercutReviewCandidateRow>(connection)?;
    rows.into_iter()
        .map(|row| {
            let metrics = bounded_vote_metrics(connection, project_id, &row.id)?;
            row.into_domain(metrics)
        })
        .collect()
}

impl PapercutReviewCandidateRow {
    fn into_domain(
        self,
        (upvote_count, last_upvoted_at): VoteMetrics,
    ) -> Result<PapercutReviewCandidate> {
        Ok(PapercutReviewCandidate {
            id: self.id,
            project_id: self.project_id,
            title: self.title,
            description: self.description,
            status: PapercutStatus::from_str(&self.status).map_err(anyhow::Error::msg)?,
            consolidated_into: self.consolidated_into,
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
            upvote_count,
            last_upvoted_at,
        })
    }
}

fn bounded_vote_metrics(
    connection: &mut DbConnection,
    project_id: &str,
    papercut_id: &str,
) -> Result<VoteMetrics> {
    let (count, latest) = papercut_upvotes::table
        .filter(papercut_upvotes::project_id.eq(project_id))
        .filter(papercut_upvotes::papercut_id.eq(papercut_id))
        .select((
            diesel::dsl::count_star(),
            diesel::dsl::max(papercut_upvotes::created_at),
        ))
        .first::<(i64, Option<String>)>(connection)?;
    Ok((
        u64::try_from(count).context("papercut upvote count exceeds u64")?,
        latest.as_deref().map(parse_time).transpose()?,
    ))
}

pub(super) fn load_papercut(
    connection: &mut DbConnection,
    project_id: &str,
    papercut_id: &str,
) -> Result<Papercut> {
    let row = load_papercut_row(connection, project_id, papercut_id)?;
    let metrics = bounded_vote_metrics(connection, project_id, papercut_id)?;
    row.into_domain(metrics)
}

fn load_open_papercuts(
    connection: &mut DbConnection,
    project_id: &str,
) -> Result<RecordWindow<Papercut>> {
    let rows = papercuts::table
        .filter(papercuts::project_id.eq(project_id))
        .filter(papercuts::status.eq("open"))
        .order((papercuts::updated_at.desc(), papercuts::id.asc()))
        .limit(query_window_limit(
            crate::coordination::api::MAX_OPEN_PAPERCUT_RESULTS,
        ))
        .select(PapercutRow::as_select())
        .load::<PapercutRow>(connection)?;
    let rows = RecordWindow::bounded(rows, crate::coordination::api::MAX_OPEN_PAPERCUT_RESULTS);
    let papercut_ids = rows
        .items
        .iter()
        .map(|row| row.id.as_str())
        .collect::<Vec<_>>();
    if papercut_ids.is_empty() {
        return Ok(RecordWindow {
            items: Vec::new(),
            truncated: rows.truncated,
        });
    }
    let metrics = papercut_upvotes::table
        .inner_join(
            papercuts::table.on(papercut_upvotes::papercut_id
                .eq(papercuts::id)
                .and(papercut_upvotes::project_id.eq(papercuts::project_id))),
        )
        .filter(papercut_upvotes::project_id.eq(project_id))
        .filter(papercut_upvotes::papercut_id.eq_any(papercut_ids))
        .filter(papercuts::project_id.eq(project_id))
        .filter(papercuts::status.eq("open"))
        .group_by(papercut_upvotes::papercut_id)
        .select((
            papercut_upvotes::papercut_id,
            diesel::dsl::count_star(),
            diesel::dsl::max(papercut_upvotes::created_at),
        ))
        .load::<(String, i64, Option<String>)>(connection)?
        .into_iter()
        .map(|(papercut_id, count, latest)| {
            Ok((
                papercut_id,
                (
                    u64::try_from(count).context("papercut upvote count exceeds u64")?,
                    latest.as_deref().map(parse_time).transpose()?,
                ),
            ))
        })
        .collect::<Result<VoteMetricsByPapercut>>()?;
    Ok(RecordWindow {
        items: papercuts_from_metrics(rows.items, metrics)?,
        truncated: rows.truncated,
    })
}

fn load_registry(
    connection: &mut DbConnection,
    project_id: Option<&str>,
) -> Result<PapercutRegistry> {
    let papercuts = load_papercut_rows(connection, project_id)?;
    let single_parent = load_single_parent_rows(connection, project_id)?;
    let mut review_query = papercut_duplicate_reviews::table.into_boxed();
    if let Some(project_id) = project_id {
        review_query = review_query.filter(papercut_duplicate_reviews::project_id.eq(project_id));
    }
    let reviews = review_query
        .order((
            papercut_duplicate_reviews::project_id.asc(),
            papercut_duplicate_reviews::created_at.asc(),
            papercut_duplicate_reviews::id.asc(),
        ))
        .select(DuplicateReviewRow::as_select())
        .load::<DuplicateReviewRow>(connection)?;
    let rows = RegistryRows {
        papercuts,
        upvotes: single_parent.upvotes,
        reviews,
        attempts: single_parent.attempts,
        lifecycle_events: load_lifecycle_events(connection, project_id)?,
    };
    registry_from_rows(rows)
}

fn load_papercut_rows(
    connection: &mut DbConnection,
    project_id: Option<&str>,
) -> Result<Vec<PapercutRow>> {
    let mut query = papercuts::table.into_boxed();
    if let Some(project_id) = project_id {
        query = query.filter(papercuts::project_id.eq(project_id));
    }
    query
        .order((
            papercuts::project_id.asc(),
            papercuts::created_at.asc(),
            papercuts::id.asc(),
        ))
        .select(PapercutRow::as_select())
        .load::<PapercutRow>(connection)
        .map_err(Into::into)
}

fn load_single_parent_rows(
    connection: &mut DbConnection,
    project_id: Option<&str>,
) -> Result<SingleParentRows> {
    let mut upvote_query = papercut_upvotes::table.into_boxed();
    if let Some(project_id) = project_id {
        upvote_query = upvote_query.filter(papercut_upvotes::project_id.eq(project_id));
    }
    let upvotes = upvote_query
        .order((
            papercut_upvotes::project_id.asc(),
            papercut_upvotes::created_at.asc(),
            papercut_upvotes::id.asc(),
        ))
        .select(PapercutUpvoteRow::as_select())
        .load::<PapercutUpvoteRow>(connection)?;

    let mut attempt_query = papercut_review_attempts::table.into_boxed();
    if let Some(project_id) = project_id {
        attempt_query = attempt_query.filter(papercut_review_attempts::project_id.eq(project_id));
    }
    let attempts = attempt_query
        .order((
            papercut_review_attempts::project_id.asc(),
            papercut_review_attempts::created_at.asc(),
            papercut_review_attempts::id.asc(),
        ))
        .select(PapercutReviewAttemptRow::as_select())
        .load::<PapercutReviewAttemptRow>(connection)?;

    Ok(SingleParentRows { upvotes, attempts })
}

fn load_lifecycle_events(
    connection: &mut DbConnection,
    project_id: Option<&str>,
) -> Result<Vec<PapercutLifecycleEvent>> {
    let mut query = events::table
        .filter(events::kind.eq_any(LIFECYCLE_EVENT_KINDS))
        .into_boxed();
    if let Some(project_id) = project_id {
        query = query.filter(events::project_id.eq(project_id));
    }
    query
        .order((
            events::project_id.asc(),
            events::created_at.asc(),
            events::id.asc(),
        ))
        .select(EventRow::as_select())
        .load::<EventRow>(connection)?
        .into_iter()
        .map(lifecycle_event_from_row)
        .collect()
}

fn lifecycle_event_from_row(row: EventRow) -> Result<PapercutLifecycleEvent> {
    let payload = serde_json::from_str::<LifecyclePayload>(&row.payload_json)
        .with_context(|| format!("decode {} event {}", row.kind, row.id))?;
    let transition = match row.kind.as_str() {
        "papercut_resolved" => PapercutLifecycleTransition::Resolved {
            resolution: payload
                .resolution
                .context("papercut_resolved event has no resolution")?,
        },
        "papercut_reopened" => PapercutLifecycleTransition::Reopened,
        "papercut_consolidated" => PapercutLifecycleTransition::Consolidated {
            canonical_id: payload
                .canonical_id
                .context("papercut_consolidated event has no canonical id")?,
            rationale: payload
                .rationale
                .context("papercut_consolidated event has no rationale")?,
        },
        kind => bail!("unknown papercut lifecycle event kind {kind}"),
    };
    Ok(PapercutLifecycleEvent {
        id: row.id,
        project_id: row.project_id,
        papercut_id: payload.papercut_id,
        transition,
        actor: payload.actor,
        created_at: parse_time(&row.created_at)?,
    })
}

fn registry_from_rows(rows: RegistryRows) -> Result<PapercutRegistry> {
    let papercuts = papercuts_from_rows(rows.papercuts, &rows.upvotes)?;
    let upvotes = rows
        .upvotes
        .into_iter()
        .map(PapercutUpvoteRow::into_domain)
        .collect::<Result<Vec<_>>>()?;
    let duplicate_reviews = rows
        .reviews
        .into_iter()
        .map(DuplicateReviewRow::into_domain)
        .collect::<Result<Vec<_>>>()?;
    let review_attempts = rows
        .attempts
        .into_iter()
        .map(PapercutReviewAttemptRow::into_domain)
        .collect::<Result<Vec<_>>>()?;
    Ok(PapercutRegistry {
        generated_at: Utc::now(),
        truncated: false,
        papercuts,
        upvotes,
        duplicate_reviews,
        review_attempts,
        lifecycle_events: rows.lifecycle_events,
    })
}

fn papercuts_from_rows(
    rows: Vec<PapercutRow>,
    upvotes: &[PapercutUpvoteRow],
) -> Result<Vec<Papercut>> {
    papercuts_from_metrics(rows, vote_metrics_by_papercut(upvotes)?)
}

fn papercuts_from_metrics(
    rows: Vec<PapercutRow>,
    mut metrics: VoteMetricsByPapercut,
) -> Result<Vec<Papercut>> {
    rows.into_iter()
        .map(|row| {
            let row_metrics = metrics.remove(&row.id).unwrap_or((0, None));
            row.into_domain(row_metrics)
        })
        .collect()
}

fn vote_metrics_by_papercut(rows: &[PapercutUpvoteRow]) -> Result<VoteMetricsByPapercut> {
    let mut metrics = BTreeMap::new();
    for row in rows {
        let entry = metrics
            .entry(row.papercut_id.clone())
            .or_insert((0_u64, None));
        entry.0 += 1;
        let created_at = parse_time(&row.created_at)?;
        if entry.1.is_none_or(|latest| created_at > latest) {
            entry.1 = Some(created_at);
        }
    }
    Ok(metrics)
}
