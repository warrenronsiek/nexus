// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::{
    paging::{Cursor, Page},
    projection,
    rows::{ObservationRow, SubjectRow},
};
use crate::persistence::backend::{DbBackend, DbConnection};
use crate::persistence::schema::{work_observations, work_subjects};
use crate::work_history::*;
use anyhow::{bail, Result};
use diesel::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SubjectScope {
    Sessions,
    Activities,
}
impl SubjectScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sessions => "session",
            Self::Activities => "activity",
        }
    }
    fn subject(self, id: String) -> WorkSubject {
        match self {
            Self::Sessions => WorkSubject::Session { id },
            Self::Activities => WorkSubject::Activity { id },
        }
    }
}
pub(super) fn subjects(
    connection: &mut DbConnection,
    project: &str,
    query: &WorkQuery,
    scope: SubjectScope,
) -> Result<WorkResponse> {
    let mut page = open_subject_page(project, query, scope)?;
    let mut rows = selection(project, query, &page.cursor, scope)
        .order(work_subjects::id.asc())
        .limit(page.limit + 1)
        .select(SubjectRow::as_select())
        .load::<SubjectRow>(connection)?;
    let more = rows.len() > page.limit as usize;
    rows.truncate(page.limit as usize);
    for row in rows {
        let item = projection::detail(connection, project, &scope.subject(row.id.clone()))?
            .map(|detail| detail.summary);
        if !page.include(row.id, item)? {
            break;
        }
    }
    let result = page.finish(more)?;
    Ok(WorkResponse::Page {
        items: result.items,
        next_cursor: result.next_cursor,
    })
}
fn open_subject_page(
    project: &str,
    query: &WorkQuery,
    scope: SubjectScope,
) -> Result<Page<WorkSummary>> {
    validate_query(query)?;
    if scope == SubjectScope::Sessions && query.kind.is_some() {
        return Err(WorkRejection::Invalid("kind filters require activities search".into()).into());
    }
    let mut identity = query.clone();
    identity.after = None;
    identity.limit = None;
    Page::open(
        query.after.as_deref(),
        format!(
            "subjects:{project}:{}:{}",
            scope.as_str(),
            serde_json::to_string(&identity)?
        ),
        query.limit,
    )
}

fn validate_query(query: &WorkQuery) -> Result<()> {
    if [
        &query.repository,
        &query.host_session_id,
        &query.kind,
        &query.environment,
        &query.search,
    ]
    .into_iter()
    .flatten()
    .any(|text| text.len() > 4096)
    {
        bail!("work query filter exceeds limit");
    }
    Ok(())
}
fn selection<'a>(
    project: &'a str,
    query: &'a WorkQuery,
    cursor: &'a Cursor,
    scope: SubjectScope,
) -> work_subjects::BoxedQuery<'a, DbBackend> {
    let rows = work_subjects::table
        .filter(work_subjects::project_id.eq(project))
        .filter(work_subjects::scope.eq(scope.as_str()))
        .filter(work_subjects::id.gt(&cursor.after))
        .filter(work_subjects::recorded_at.le(cursor.cutoff.to_rfc3339()))
        .into_boxed::<DbBackend>();
    let rows = context_filters(rows, query);
    let rows = report_filters(rows, query);
    window_filters(rows, query)
}
fn context_filters<'a>(
    mut rows: work_subjects::BoxedQuery<'a, DbBackend>,
    query: &'a WorkQuery,
) -> work_subjects::BoxedQuery<'a, DbBackend> {
    if let Some(value) = &query.repository {
        rows = rows.filter(work_subjects::repository.eq(value));
    }
    if let Some(value) = &query.host_session_id {
        rows = rows.filter(work_subjects::host_session_id.eq(value));
    }
    if let Some(value) = &query.environment {
        rows = rows.filter(work_subjects::environment.eq(value));
    }
    rows
}
fn report_filters<'a>(
    mut rows: work_subjects::BoxedQuery<'a, DbBackend>,
    query: &'a WorkQuery,
) -> work_subjects::BoxedQuery<'a, DbBackend> {
    if let Some(value) = &query.kind {
        rows = rows.filter(work_subjects::kind.eq(value));
    }
    if let Some(value) = query.outcome {
        let text = serde_json::to_value(value)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned));
        rows = rows.filter(work_subjects::outcome.eq(text));
    }
    if let Some(value) = &query.search {
        let escaped = value
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_");
        rows = rows.filter(
            work_subjects::title
                .like(format!("%{escaped}%"))
                .escape('!'),
        );
    }
    rows
}
fn window_filters<'a>(
    mut rows: work_subjects::BoxedQuery<'a, DbBackend>,
    query: &'a WorkQuery,
) -> work_subjects::BoxedQuery<'a, DbBackend> {
    if let Some(value) = &query.occurred_after {
        rows = rows.filter(work_subjects::occurred_at.ge(value.to_rfc3339()));
    }
    if let Some(value) = &query.occurred_before {
        rows = rows.filter(work_subjects::occurred_at.lt(value.to_rfc3339()));
    }
    rows
}
pub(super) fn events(
    connection: &mut DbConnection,
    project: &str,
    subject: Option<&WorkSubject>,
    after: Option<&str>,
    limit: Option<u32>,
) -> Result<WorkResponse> {
    let mut page = Page::open(after, format!("events:{project}:{subject:?}"), limit)?;
    let mut rows = event_rows(connection, project, subject, &page)?;
    let more = rows.len() > page.limit as usize;
    rows.truncate(page.limit as usize);
    for row in rows {
        let observation = row.observation()?;
        if !page.include(row.id, Some(observation))? {
            break;
        }
    }
    let result = page.finish(more)?;
    Ok(WorkResponse::Events {
        observations: result.items,
        next_cursor: result.next_cursor,
    })
}
fn event_rows(
    connection: &mut DbConnection,
    project: &str,
    subject: Option<&WorkSubject>,
    page: &Page<WorkObservation>,
) -> Result<Vec<ObservationRow>> {
    let mut rows = work_observations::table
        .filter(work_observations::project_id.eq(project))
        .filter(work_observations::recorded_at.le(page.cursor.cutoff.to_rfc3339()))
        .filter(work_observations::id.gt(&page.cursor.after))
        .into_boxed::<DbBackend>();
    if let Some(subject) = subject {
        rows = rows
            .filter(work_observations::subject_scope.eq(subject.scope()))
            .filter(work_observations::subject_id.eq(subject.id()));
    }
    Ok(rows
        .order(work_observations::id.asc())
        .limit(page.limit + 1)
        .select(ObservationRow::as_select())
        .load(connection)?)
}
