// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::ingestion::{index, refresh};
use super::rows::{ObservationRow, SubjectRow};
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{work_links, work_observations, work_subjects};
use crate::work_history::*;
use anyhow::{bail, Context, Result};
use diesel::prelude::*;

pub(crate) fn rebuild_snapshot(connection: &mut DbConnection) -> Result<()> {
    diesel::delete(work_links::table).execute(connection)?;
    diesel::delete(work_subjects::table).execute(connection)?;
    rebuild_indexes(connection)?;
    refresh_subjects(connection)
}
fn rebuild_indexes(connection: &mut DbConnection) -> Result<()> {
    let mut after: Option<(String, String)> = None;
    loop {
        let mut query = work_observations::table.into_boxed();
        if let Some((project, id)) = &after {
            query = query.filter(
                work_observations::project_id
                    .gt(project)
                    .or(work_observations::project_id
                        .eq(project)
                        .and(work_observations::id.gt(id))),
            );
        }
        let rows = query
            .order((
                work_observations::project_id.asc(),
                work_observations::id.asc(),
            ))
            .limit(16)
            .select(ObservationRow::as_select())
            .load::<ObservationRow>(connection)?;
        if rows.is_empty() {
            break;
        }
        for row in &rows {
            row.validate_identity()?;
            index(connection, row)?;
        }
        let last = rows.last().context("missing work import batch")?;
        after = Some((last.project_id.clone(), last.id.clone()));
    }
    Ok(())
}
impl ObservationRow {
    fn validate_identity(&self) -> Result<()> {
        let observation = self.observation()?;
        observation.validate()?;
        let identity_matches = observation.input.id == self.id
            && observation.input.subject.id() == self.subject_id
            && observation.input.subject.scope() == self.subject_scope
            && observation.input.body.family() == self.kind;
        let encoding_matches = observation.input.occurred_at.to_rfc3339() == self.occurred_at
            && blake3::hash(self.payload_json.as_bytes()).to_hex().as_str() == self.content_hash;
        if !identity_matches || !encoding_matches {
            bail!("invalid portable work observation identity");
        }
        Ok(())
    }
}
fn refresh_subjects(connection: &mut DbConnection) -> Result<()> {
    let mut cursor: Option<(String, String, String)> = None;
    loop {
        let mut query = work_subjects::table.into_boxed();
        if let Some((project, scope, id)) = &cursor {
            query = query.filter(
                work_subjects::project_id
                    .gt(project)
                    .or(work_subjects::project_id.eq(project).and(
                        work_subjects::scope
                            .gt(scope)
                            .or(work_subjects::scope.eq(scope).and(work_subjects::id.gt(id))),
                    )),
            );
        }
        let rows = query
            .order((
                work_subjects::project_id.asc(),
                work_subjects::scope.asc(),
                work_subjects::id.asc(),
            ))
            .limit(16)
            .select(SubjectRow::as_select())
            .load::<SubjectRow>(connection)?;
        if rows.is_empty() {
            break;
        }
        for row in &rows {
            let subject = if row.scope == "session" {
                WorkSubject::Session { id: row.id.clone() }
            } else {
                WorkSubject::Activity { id: row.id.clone() }
            };
            refresh(connection, &row.project_id, &subject)?;
        }
        let last = rows.last().context("missing work subject batch")?;
        cursor = Some((last.project_id.clone(), last.scope.clone(), last.id.clone()));
    }
    Ok(())
}
