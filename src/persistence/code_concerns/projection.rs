// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use super::{
    counts::{stamp, status_name, Population},
    queries,
    rows::*,
};
use crate::code_concerns::*;
use crate::persistence::{
    backend::DbConnection,
    schema::{
        code_concerns as c, concern_areas as a, concern_attributions as l, concern_encounters as e,
        concern_observations as o,
    },
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use diesel::prelude::*;

struct Lifecycle {
    resolution: Option<DateTime<Utc>>,
    known_encounter: Option<DateTime<Utc>>,
    unknown_encounter: Option<DateTime<Utc>>,
    status: ConcernStatus,
    qualification: Option<String>,
}
impl Lifecycle {
    fn observe(&mut self, input: &ConcernObservationInput) -> bool {
        let time = input.occurred_at.unwrap_or(input.observed_at);
        match &input.body {
            ConcernEvent::Resolved { .. } => {
                self.resolve(time);
            }
            ConcernEvent::Reopened { .. } => {
                let returned = self.status == ConcernStatus::Resolved;
                self.status = ConcernStatus::Open;
                return returned;
            }
            _ if input.body.encounter().is_some() => {
                return self.encounter(input);
            }
            _ => (),
        }
        false
    }
    fn resolve(&mut self, time: DateTime<Utc>) {
        self.resolution = Some(time);
        self.status = ConcernStatus::Resolved;
        self.qualification = match (self.known_encounter, self.unknown_encounter) {
            (Some(encounter), _) if encounter == time => {
                Some("same_time_recurrence_order_unknown".into())
            }
            (_, Some(encounter)) if encounter >= time => Some("unknown_recurrence_order".into()),
            _ => None,
        };
    }
    fn encounter(&mut self, input: &ConcernObservationInput) -> bool {
        match input.occurred_at {
            Some(time) => self.known_encounter = Some(time),
            None => self.unknown_encounter = Some(input.observed_at),
        }
        if let Some(resolution) = self.resolution {
            match input.occurred_at {
                Some(time) if time > resolution => {
                    let returned = self.status == ConcernStatus::Resolved;
                    self.status = ConcernStatus::Open;
                    return returned;
                }
                Some(time) if time == resolution => {
                    self.qualification = Some("same_time_recurrence_order_unknown".into())
                }
                None if input.observed_at >= resolution => {
                    self.qualification = Some("unknown_recurrence_order".into())
                }
                _ => (),
            }
        }
        false
    }
}
pub(super) fn refresh(connection: &mut DbConnection, project: &str, id: &str) -> Result<()> {
    if let Some(summary) = canonical_summary(connection, project, id)? {
        refresh_canonical(connection, project, summary)?;
    }
    Ok(())
}
fn canonical_summary(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
) -> Result<Option<ConcernSummary>> {
    queries::lookup(connection, project, id)?
        .filter(|row| row.canonical_id == row.id)
        .map(|row| row.summary())
        .transpose()
}
fn refresh_canonical(
    connection: &mut DbConnection,
    project: &str,
    mut summary: ConcernSummary,
) -> Result<()> {
    let id = summary.id.clone();
    project_lifecycle(connection, project, &id, &mut summary)?;
    project_context(connection, project, &id, &mut summary)?;
    write_summary(connection, project, &summary)?;
    refresh_aliases(connection, project, &summary)
}
fn project_lifecycle(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
    summary: &mut ConcernSummary,
) -> Result<()> {
    let lifecycle = read_lifecycle(connection, project, id)?;
    summary.status = lifecycle.status;
    summary.resolved_at = lifecycle.resolution;
    summary.lifecycle_qualification = lifecycle.qualification;
    Ok(())
}
fn read_lifecycle(connection: &mut DbConnection, project: &str, id: &str) -> Result<Lifecycle> {
    let mut lifecycle = Lifecycle {
        resolution: None,
        known_encounter: None,
        unknown_encounter: None,
        status: ConcernStatus::Open,
        qualification: None,
    };
    diesel::update(
        e::table
            .filter(e::project_id.eq(project))
            .filter(e::concern_id.eq(id)),
    )
    .set(e::returned_after_resolution.eq(0))
    .execute(connection)?;
    let mut cursor = None;
    loop {
        let rows = history_batch(connection, project, id, cursor.as_ref())?;
        if rows.is_empty() {
            break;
        }
        for event in &rows {
            apply_lifecycle(connection, project, &mut lifecycle, event)?;
        }
        let last = rows.last().unwrap();
        cursor = Some((last.effective_time.clone(), last.id.clone()));
    }
    Ok(lifecycle)
}
fn history_batch(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
    cursor: Option<&(String, String)>,
) -> Result<Vec<ObservationRow>> {
    let mut query = o::table
        .filter(o::project_id.eq(project))
        .filter(
            o::concern_id.eq_any(
                c::table
                    .filter(c::project_id.eq(project))
                    .filter(c::canonical_id.eq(id))
                    .select(c::id),
            ),
        )
        .into_boxed();
    if let Some((time, event)) = cursor {
        query = query.filter(
            o::effective_time
                .gt(time)
                .or(o::effective_time.eq(time).and(o::id.gt(event))),
        )
    }
    Ok(query
        .order((o::effective_time.asc(), o::id.asc()))
        .limit(16)
        .select(ObservationRow::as_select())
        .load::<ObservationRow>(connection)?)
}
fn apply_lifecycle(
    connection: &mut DbConnection,
    project: &str,
    lifecycle: &mut Lifecycle,
    event: &ObservationRow,
) -> Result<()> {
    let observation = event.observation()?;
    if lifecycle.observe(&observation.input) {
        diesel::update(e::table.find((project, &event.id)))
            .set(e::returned_after_resolution.eq(1))
            .execute(connection)?;
    }
    Ok(())
}
fn project_context(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
    summary: &mut ConcernSummary,
) -> Result<()> {
    summary.counts = Population {
        project,
        query: &ConcernQuery::default(),
        concern: Some(id),
        area: None,
    }
    .counts(connection)?;
    summary.last_observed_at = last_observed(connection, project, id)?;
    let areas = areas(connection, project, id)?;
    summary.areas_partial = areas.len() > 32;
    summary.areas = areas.into_iter().take(32).collect();
    Ok(())
}
fn last_observed(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
) -> Result<Option<DateTime<Utc>>> {
    e::table
        .filter(e::project_id.eq(project))
        .filter(e::concern_id.eq(id))
        .select(diesel::dsl::max(e::observed_at))
        .first::<Option<String>>(connection)?
        .map(|v| v.parse().map_err(Into::into))
        .transpose()
}
fn areas(connection: &mut DbConnection, project: &str, id: &str) -> Result<Vec<ConcernArea>> {
    let areas = a::table
        .filter(a::project_id.eq(project))
        .filter(
            a::id.eq_any(
                l::table
                    .filter(l::project_id.eq(project))
                    .filter(
                        l::event_id.eq_any(
                            e::table
                                .filter(e::project_id.eq(project))
                                .filter(e::concern_id.eq(id))
                                .select(e::event_id),
                        ),
                    )
                    .select(l::area_id),
            ),
        )
        .order(a::id.asc())
        .limit(33)
        .select(a::area_json)
        .load::<String>(connection)?;
    areas
        .into_iter()
        .map(|value| serde_json::from_str(&value).map_err(Into::into))
        .collect()
}
fn write_summary(
    connection: &mut DbConnection,
    project: &str,
    summary: &ConcernSummary,
) -> Result<()> {
    let updated = summary.last_observed_at.unwrap_or(summary.created_at);
    diesel::update(c::table.find((project, &summary.id)))
        .set((
            c::status.eq(status_name(summary.status)),
            c::last_observed_at.eq(stamp(updated)),
            c::work_session_count.eq(summary.counts.work_sessions as i64),
            c::returns_count.eq(summary.counts.returns_after_resolution as i64),
            c::summary_json.eq(serde_json::to_string(summary)?),
        ))
        .execute(connection)?;
    Ok(())
}
fn refresh_aliases(
    connection: &mut DbConnection,
    project: &str,
    canonical: &ConcernSummary,
) -> Result<()> {
    let mut after = String::new();
    loop {
        let rows = c::table
            .filter(c::project_id.eq(project))
            .filter(c::canonical_id.eq(&canonical.id))
            .filter(c::id.ne(&canonical.id))
            .filter(c::id.gt(&after))
            .order(c::id.asc())
            .limit(16)
            .select(ConcernRow::as_select())
            .load::<ConcernRow>(connection)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            after = row.id.clone();
            refresh_alias(connection, project, &row, canonical)?;
        }
    }
    Ok(())
}
fn refresh_alias(
    connection: &mut DbConnection,
    project: &str,
    row: &ConcernRow,
    canonical: &ConcernSummary,
) -> Result<()> {
    let mut summary = row.summary()?;
    summary.canonical_id = canonical.id.clone();
    summary.status = ConcernStatus::Consolidated;
    summary.counts = canonical.counts.clone();
    summary.areas = canonical.areas.clone();
    summary.areas_partial = canonical.areas_partial;
    summary.last_observed_at = canonical.last_observed_at;
    summary.lifecycle_qualification = Some("consolidated_counts_are_canonical".into());
    diesel::update(c::table.find((project, &row.id)))
        .set((
            c::status.eq("consolidated"),
            c::summary_json.eq(serde_json::to_string(&summary)?),
        ))
        .execute(connection)?;
    Ok(())
}
