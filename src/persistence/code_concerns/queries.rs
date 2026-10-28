// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use super::{counts::Population, rows::*};
use crate::code_concerns::*;
use crate::persistence::backend::{DbBackend, DbConnection};
use crate::persistence::schema::{
    code_concerns as c, concern_areas as a, concern_attributions as l, concern_encounters as e,
    concern_observations as o,
};
use anyhow::Result;
use diesel::dsl::{case_when, count, max, sum};
use diesel::prelude::*;

pub(super) fn read(
    connection: &mut DbConnection,
    project: &str,
    action: &ConcernAction,
) -> Result<ConcernResponse> {
    match action {
        ConcernAction::List { query, .. } => list(connection, project, query),
        ConcernAction::Dashboard { query, .. } => dashboard(connection, project, query),
        ConcernAction::Get { concern_id, .. } => detail(connection, project, concern_id),
        ConcernAction::History {
            concern_id,
            after,
            limit,
            ..
        } => {
            let (observations, next_cursor) = history(
                connection,
                project,
                Some(concern_id),
                after.as_deref(),
                *limit,
            )?;
            Ok(ConcernResponse::History {
                concern_id: concern_id.clone(),
                session_titles: Vec::new(),
                observations,
                next_cursor,
            })
        }
        ConcernAction::Export { after, limit, .. } => {
            let (observations, next_cursor) =
                history(connection, project, None, after.as_deref(), *limit)?;
            Ok(ConcernResponse::Export {
                observations,
                next_cursor,
            })
        }
        ConcernAction::Dump { .. } => dump(connection, project),
        _ => unreachable!("mutations dispatched separately"),
    }
}
pub(super) fn lookup(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
) -> Result<Option<ConcernRow>> {
    Ok(c::table
        .find((project, id))
        .select(ConcernRow::as_select())
        .first(connection)
        .optional()?)
}
fn offset(query: &ConcernQuery) -> Result<i64> {
    let result = query
        .after
        .as_deref()
        .unwrap_or("0")
        .parse::<i64>()
        .map_err(|_| ConcernRejection::Invalid("invalid concern page cursor".into()))?;
    if !(0..=10_000_000).contains(&result) {
        return Err(ConcernRejection::Invalid("invalid concern page cursor".into()).into());
    }
    Ok(result)
}
fn limit(query: &ConcernQuery) -> i64 {
    i64::from(query.limit.unwrap_or(24).clamp(1, 64))
}
fn selected_summaries(
    connection: &mut DbConnection,
    project: &str,
    query: &ConcernQuery,
) -> Result<(Vec<ConcernSummary>, Option<String>, usize)> {
    let pop = Population {
        project,
        query,
        concern: None,
        area: None,
    };
    let total = pop.counts(connection)?.concerns;
    let (ids, next) = selected_ids(connection, &pop)?;
    let items = ids
        .into_iter()
        .map(|id| scoped_summary(connection, &pop, &id))
        .collect::<Result<_>>()?;
    Ok((items, next, total))
}
fn scoped_summary(
    connection: &mut DbConnection,
    pop: &Population<'_>,
    id: &str,
) -> Result<ConcernSummary> {
    let row = lookup(connection, pop.project, id)?
        .ok_or_else(|| anyhow::anyhow!("missing concern projection"))?;
    let mut summary = row.summary()?;
    summary.counts = Population {
        project: pop.project,
        query: pop.query,
        concern: Some(&row.id),
        area: None,
    }
    .counts(connection)?;
    Ok(summary)
}
fn selected_ids(
    connection: &mut DbConnection,
    pop: &Population<'_>,
) -> Result<(Vec<String>, Option<String>)> {
    let project = pop.project;
    let query = pop.query;
    let candidates = e::table
        .filter(e::project_id.eq(project))
        .filter(e::event_id.eq_any(pop.query().select(e::event_id)))
        .group_by(e::concern_id)
        .select(e::concern_id)
        .into_boxed();
    let candidates = match query.sort {
        ConcernSort::Recent => candidates.order((max(e::observed_at).desc(), e::concern_id.asc())),
        ConcernSort::WorkSessions => candidates.order((
            count(e::work_session_id).aggregate_distinct().desc(),
            e::concern_id.asc(),
        )),
        ConcernSort::Returns => candidates.order((
            sum(e::returned_after_resolution).desc(),
            e::concern_id.asc(),
        )),
    };
    let start = offset(query)?;
    let mut ids = candidates
        .offset(start)
        .limit(limit(query) + 1)
        .load::<String>(connection)?;
    let next = (ids.len() > limit(query) as usize).then(|| (start + limit(query)).to_string());
    ids.truncate(limit(query) as usize);
    Ok((ids, next))
}
fn list(
    connection: &mut DbConnection,
    project: &str,
    query: &ConcernQuery,
) -> Result<ConcernResponse> {
    let (items, next_cursor, total) = selected_summaries(connection, project, query)?;
    let mut response = ConcernResponse::Page {
        project_id: project.into(),
        query: query.clone(),
        items,
        next_cursor,
        total,
        partial: false,
    };
    while serde_json::to_vec(&response)?.len() > 768 * 1024 {
        let ConcernResponse::Page {
            items,
            next_cursor,
            partial,
            ..
        } = &mut response
        else {
            unreachable!()
        };
        items.pop();
        *next_cursor = Some((offset(query)? + items.len() as i64).to_string());
        *partial = true;
    }
    Ok(response)
}
fn detail(connection: &mut DbConnection, project: &str, id: &str) -> Result<ConcernResponse> {
    let Some(row) = lookup(connection, project, id)? else {
        return Ok(ConcernResponse::Detail { concern: None });
    };
    let (history, next_cursor) = history(connection, project, Some(id), None, Some(8))?;
    Ok(ConcernResponse::Detail {
        concern: Some(Box::new(ConcernDetail {
            session_titles: Vec::new(),
            summary: row.summary()?,
            partial: next_cursor.is_some(),
            history,
            next_cursor,
        })),
    })
}
pub(super) fn history(
    connection: &mut DbConnection,
    project: &str,
    id: Option<&str>,
    after: Option<&str>,
    limit: Option<u32>,
) -> Result<(Vec<ConcernObservation>, Option<String>)> {
    let size = limit.unwrap_or(8).clamp(1, 32) as usize;
    let cursor = HistoryCursor::read(project, id, after)?;
    let rows = history_rows(connection, project, &cursor, size)?;
    history_page(rows, &cursor, size)
}
fn history_rows(
    connection: &mut DbConnection,
    project: &str,
    cursor: &HistoryCursor,
    size: usize,
) -> Result<Vec<ObservationRow>> {
    let query = history_population(connection, project, cursor)?;
    let query = history_position(query, cursor)?;
    Ok(query
        .limit((size + 1) as i64)
        .select(ObservationRow::as_select())
        .load::<ObservationRow>(connection)?)
}
fn history_population<'a>(
    connection: &mut DbConnection,
    project: &'a str,
    cursor: &HistoryCursor,
) -> Result<o::BoxedQuery<'a, DbBackend>> {
    let mut query = o::table
        .filter(o::project_id.eq(project))
        .filter(o::recorded_at.le(super::counts::stamp(cursor.cutoff)))
        .into_boxed();
    if let Some(id) = cursor.concern.as_deref() {
        let canonical = lookup(connection, project, id)?
            .map(|row| row.canonical_id)
            .unwrap_or_else(|| id.into());
        query = query.filter(
            o::concern_id.eq_any(
                c::table
                    .filter(c::project_id.eq(project))
                    .filter(c::canonical_id.eq(canonical))
                    .select(c::id),
            ),
        );
    }
    Ok(query)
}
fn history_position<'a>(
    mut query: o::BoxedQuery<'a, DbBackend>,
    cursor: &'a HistoryCursor,
) -> Result<o::BoxedQuery<'a, DbBackend>> {
    if let Some(after_id) = &cursor.id {
        query = if cursor.concern.is_some() {
            let time = cursor
                .time
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing concern history cursor time"))?;
            query.filter(
                o::effective_time
                    .lt(time)
                    .or(o::effective_time.eq(time).and(o::id.lt(after_id))),
            )
        } else {
            query.filter(o::id.gt(after_id))
        };
    }
    Ok(if cursor.concern.is_some() {
        query.order((o::effective_time.desc(), o::id.desc()))
    } else {
        query.order(o::id.asc())
    })
}
fn history_page(
    mut rows: Vec<ObservationRow>,
    cursor: &HistoryCursor,
    size: usize,
) -> Result<(Vec<ConcernObservation>, Option<String>)> {
    let next = if rows.len() > size {
        let row = &rows[size - 1];
        Some(serde_json::to_string(&HistoryCursor {
            project: cursor.project.clone(),
            concern: cursor.concern.clone(),
            cutoff: cursor.cutoff,
            time: Some(row.effective_time.clone()),
            id: Some(row.id.clone()),
        })?)
    } else {
        None
    };
    rows.truncate(size);
    Ok((
        rows.iter()
            .map(ObservationRow::observation)
            .collect::<Result<_>>()?,
        next,
    ))
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryCursor {
    project: String,
    concern: Option<String>,
    cutoff: chrono::DateTime<chrono::Utc>,
    time: Option<String>,
    id: Option<String>,
}
impl HistoryCursor {
    fn read(project: &str, id: Option<&str>, after: Option<&str>) -> Result<Self> {
        let cursor = after
            .map(serde_json::from_str::<HistoryCursor>)
            .transpose()
            .map_err(|_| ConcernRejection::Invalid("invalid concern history cursor".into()))?
            .unwrap_or_else(|| HistoryCursor {
                project: project.into(),
                concern: id.map(str::to_owned),
                cutoff: chrono::Utc::now(),
                time: None,
                id: None,
            });
        if cursor.project != project || cursor.concern.as_deref() != id {
            return Err(ConcernRejection::Invalid("concern cursor scope mismatch".into()).into());
        }
        Ok(cursor)
    }
}
fn dump(connection: &mut DbConnection, project: &str) -> Result<ConcernResponse> {
    let mut observations = Vec::new();
    let mut after = None;
    loop {
        let (page, next) = history(connection, project, None, after.as_deref(), Some(32))?;
        observations.extend(page);
        match next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    Ok(ConcernResponse::Dump {
        project_id: project.into(),
        observations,
    })
}
fn area_ids(connection: &mut DbConnection, pop: &Population<'_>) -> Result<Vec<String>> {
    let base = l::table
        .inner_join(
            e::table.on(e::project_id
                .eq(l::project_id)
                .and(e::event_id.eq(l::event_id))),
        )
        .filter(l::project_id.eq(pop.project))
        .filter(e::event_id.eq_any(pop.query().select(e::event_id)))
        .group_by(l::area_id)
        .select(l::area_id)
        .into_boxed();
    let sorted = match pop.query.sort {
        ConcernSort::Recent => base.order((max(e::observed_at).desc(), l::area_id.asc())),
        ConcernSort::WorkSessions => base.order((
            count(e::work_session_id).aggregate_distinct().desc(),
            l::area_id.asc(),
        )),
        ConcernSort::Returns => {
            // An encounter may carry both roles for one area; rank the same unique
            // return events counted by the area projection, not attribution rows.
            base.order((
                count(case_when(e::returned_after_resolution.eq(1), e::event_id))
                    .aggregate_distinct()
                    .desc(),
                l::area_id.asc(),
            ))
        }
    };
    Ok(sorted
        .offset(offset(pop.query)?)
        .limit(limit(pop.query) + 1)
        .load(connection)?)
}
fn dashboard(
    connection: &mut DbConnection,
    project: &str,
    query: &ConcernQuery,
) -> Result<ConcernResponse> {
    let pop = Population {
        project,
        query,
        concern: None,
        area: None,
    };
    let counts = pop.counts(connection)?;
    let mut ids = area_ids(connection, &pop)?;
    let next_cursor = (ids.len() > limit(query) as usize)
        .then(|| (offset(query).unwrap_or(0) + limit(query)).to_string());
    ids.truncate(limit(query) as usize);
    let areas = ids
        .into_iter()
        .map(|id| area_summary(connection, &pop, &id))
        .collect::<Result<_>>()?;
    let mut daily = pop.daily(connection)?;
    let partial = daily.len() > 366;
    daily.truncate(366);
    daily.reverse();
    let mut notices=vec!["Reported problems are observations, not a defect rate or proof of causation; overlapping areas are not additive.".into(),"Window uses source occurrence time when known, otherwise observation time; unknown source timing is counted explicitly.".into()];
    if partial {
        notices.push("Daily chart limited to366recent days; aggregate counts cover the full selected population.".into())
    }
    Ok(ConcernResponse::Dashboard {
        project_id: project.into(),
        query: query.clone(),
        counts,
        areas,
        daily,
        next_cursor,
        partial,
        notices,
    })
}
fn area_summary(
    connection: &mut DbConnection,
    pop: &Population<'_>,
    id: &str,
) -> Result<ConcernAreaSummary> {
    let row = a::table
        .find((pop.project, id))
        .select(AreaRow::as_select())
        .first::<AreaRow>(connection)?;
    let area_pop = Population {
        project: pop.project,
        query: pop.query,
        concern: None,
        area: Some(id),
    };
    Ok(ConcernAreaSummary {
        area: serde_json::from_str(&row.area_json)?,
        counts: area_pop.counts(connection)?,
        open_concern_count: open_concerns(connection, &area_pop)?,
        last_observed_at: last_observed(connection, &area_pop)?,
    })
}
fn open_concerns(connection: &mut DbConnection, pop: &Population<'_>) -> Result<usize> {
    Ok(pop
        .query()
        .filter(
            e::concern_id.eq_any(
                c::table
                    .filter(c::project_id.eq(pop.project))
                    .filter(c::status.eq("open"))
                    .select(c::id),
            ),
        )
        .select(count(e::concern_id).aggregate_distinct())
        .first::<i64>(connection)? as usize)
}
fn last_observed(
    connection: &mut DbConnection,
    pop: &Population<'_>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    Ok(pop
        .query()
        .select(max(e::observed_at))
        .first::<Option<String>>(connection)?
        .map(|value| value.parse())
        .transpose()?)
}
