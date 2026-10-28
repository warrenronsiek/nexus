// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use super::{ingestion, projection, rows::*};
use crate::persistence::{
    backend::DbConnection,
    schema::{
        code_concerns as c, concern_areas as a, concern_attributions as l, concern_encounters as e,
        concern_observations as o,
    },
};
use anyhow::{bail, Result};
use diesel::prelude::*;

pub(crate) fn rebuild_snapshot(connection: &mut DbConnection) -> Result<()> {
    clear_projections(connection)?;
    let projects = o::table
        .select(o::project_id)
        .distinct()
        .load::<String>(connection)?;
    for project in projects {
        reindex_known(connection, &project)?;
    }
    Ok(())
}
fn clear_projections(connection: &mut DbConnection) -> Result<()> {
    super::duplicates::clear_index(connection)?;
    diesel::delete(l::table).execute(connection)?;
    diesel::delete(e::table).execute(connection)?;
    diesel::delete(a::table).execute(connection)?;
    diesel::delete(c::table).execute(connection)?;
    Ok(())
}
pub(super) fn reindex_known(connection: &mut DbConnection, project: &str) -> Result<()> {
    replay_observations(connection, project, RebuildPhase::Declarations)?;
    replay_observations(connection, project, RebuildPhase::Evidence)?;
    refresh_canonical(connection, project)
}
#[derive(Clone, Copy)]
enum RebuildPhase {
    Declarations,
    Evidence,
}
impl RebuildPhase {
    fn rows(
        self,
        connection: &mut DbConnection,
        project: &str,
        after: &str,
    ) -> Result<Vec<ObservationRow>> {
        let mut query = o::table
            .filter(o::project_id.eq(project))
            .filter(o::id.gt(after))
            .into_boxed();
        if matches!(self, Self::Declarations) {
            query = query.filter(o::kind.eq("reported"));
        }
        Ok(query
            .order(o::id.asc())
            .limit(16)
            .select(ObservationRow::as_select())
            .load::<ObservationRow>(connection)?)
    }
    fn apply(self, connection: &mut DbConnection, row: &mut ObservationRow) -> Result<()> {
        validate_row(row)?;
        match self {
            Self::Declarations => {
                ingestion::declare(connection, row, super::publication::Origin::Imported)
                    .map(|_| ())
            }
            Self::Evidence => ingestion::index(connection, row),
        }
    }
}
fn replay_observations(
    connection: &mut DbConnection,
    project: &str,
    phase: RebuildPhase,
) -> Result<()> {
    let mut after = String::new();
    loop {
        let rows = phase.rows(connection, project, &after)?;
        if rows.is_empty() {
            break;
        }
        for mut row in rows {
            after = row.id.clone();
            phase.apply(connection, &mut row)?;
        }
    }
    Ok(())
}
fn refresh_canonical(connection: &mut DbConnection, project: &str) -> Result<()> {
    let mut after = String::new();
    loop {
        let ids = c::table
            .filter(c::project_id.eq(project))
            .filter(c::id.eq(c::canonical_id))
            .filter(c::id.gt(&after))
            .order(c::id.asc())
            .limit(16)
            .select(c::id)
            .load::<String>(connection)?;
        if ids.is_empty() {
            break;
        }
        for id in ids {
            after = id.clone();
            projection::refresh(connection, project, &id)?;
        }
    }
    Ok(())
}
fn validate_row(row: &ObservationRow) -> Result<()> {
    let observation = row.observation()?;
    observation.validate()?;
    if observation.input.id != row.id
        || observation.input.concern_id != row.concern_id
        || observation.input.body.kind() != row.kind
        || blake3::hash(row.payload_json.as_bytes()).to_hex().as_str() != row.content_hash
        || row.occurred_at != observation.input.occurred_at.map(super::counts::stamp)
        || row.observed_at != super::counts::stamp(observation.input.observed_at)
        || row.effective_time
            != super::counts::stamp(
                observation
                    .input
                    .occurred_at
                    .unwrap_or(observation.input.observed_at),
            )
        || row.payload_json.len() > 16 * 1024
    {
        bail!("invalid portable concern identity")
    }
    Ok(())
}
