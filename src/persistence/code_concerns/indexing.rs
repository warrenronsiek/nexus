// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use super::{duplicates, queries, rows::*};
use crate::code_concerns::*;
use crate::persistence::{
    backend::DbConnection,
    schema::{concern_areas as a, concern_attributions as l, concern_encounters as e},
};
use anyhow::{ensure, Result};
use diesel::prelude::*;

struct IndexedObservation<'a> {
    row: &'a ObservationRow,
    observation: ConcernObservation,
    canonical: String,
}
impl<'a> IndexedObservation<'a> {
    fn load(connection: &mut DbConnection, row: &'a ObservationRow) -> Result<Option<Self>> {
        let observation = row.observation()?;
        Ok(
            queries::lookup(connection, &row.project_id, &row.concern_id)?.map(|record| Self {
                row,
                observation,
                canonical: record.canonical_id,
            }),
        )
    }
    fn install(&self, connection: &mut DbConnection) -> Result<()> {
        self.install_encounter(connection)?;
        self.install_mappings(connection)?;
        duplicates::index(connection, self.row, &self.observation.input.body)
    }
    fn install_encounter(&self, connection: &mut DbConnection) -> Result<()> {
        let Some(encounter) = self.observation.input.body.encounter() else {
            return Ok(());
        };
        let row = self.encounter_row(encounter);
        diesel::insert_into(e::table)
            .values(row)
            .on_conflict_do_nothing()
            .execute(connection)?;
        encounter
            .areas
            .iter()
            .try_for_each(|area| self.install_attribution(connection, area))
    }
    fn install_attribution(
        &self,
        connection: &mut DbConnection,
        attribution: &ConcernAreaAttribution,
    ) -> Result<()> {
        let relation = match attribution.relation {
            ConcernAreaRelation::Affected => "affected",
            ConcernAreaRelation::SuspectedCause => "suspected_cause",
        };
        index_area(connection, self.row, &attribution.area, relation)?;
        attribution
            .memberships
            .iter()
            .try_for_each(|area| index_area(connection, self.row, area, relation))
    }
    fn install_mappings(&self, connection: &mut DbConnection) -> Result<()> {
        if let ConcernEvent::AreaMapped { from, to, .. } = &self.observation.input.body {
            index_area(connection, self.row, from, "mapping")?;
            index_area(connection, self.row, to, "mapping")?;
        }
        Ok(())
    }
    fn encounter_row(&self, encounter: &ConcernEncounter) -> EncounterRow {
        let input = &self.observation.input;
        let source_key = encounter.source.as_ref().map(|s| {
            crate::identity::stable_id(&[
                &s.source,
                &s.repository,
                &s.execution_attempt,
                &s.failure_id,
            ])
        });
        let repository = input
            .context
            .repository
            .clone()
            .or_else(|| encounter.areas.first().map(|v| v.area.repository.clone()))
            .or_else(|| encounter.source.as_ref().map(|v| v.repository.clone()));
        EncounterRow {
            project_id: self.row.project_id.clone(),
            event_id: self.row.id.clone(),
            concern_id: self.canonical.clone(),
            reported_concern_id: self.row.concern_id.clone(),
            source_key,
            work_session_id: input.context.work_session_id.clone(),
            host_session_key: self.host_session_key(),
            source_time: self.row.occurred_at.clone(),
            effective_time: self.row.effective_time.clone(),
            observed_at: self.row.observed_at.clone(),
            effective_day: self.row.effective_time[..10].into(),
            repository,
            returned_after_resolution: 0,
        }
    }
    fn host_session_key(&self) -> Option<String> {
        let actor = self
            .observation
            .source
            .as_ref()
            .map(|source| &source.provenance)
            .unwrap_or(&self.observation.provenance);
        self.observation
            .input
            .context
            .host_session_id
            .as_ref()
            .map(|id| crate::identity::stable_id(&[&actor.agent, id]))
    }
}
pub(super) fn index(connection: &mut DbConnection, row: &ObservationRow) -> Result<()> {
    if let Some(observation) = IndexedObservation::load(connection, row)? {
        observation.install(connection)?;
    }
    Ok(())
}
fn index_area(
    connection: &mut DbConnection,
    row: &ObservationRow,
    area: &ConcernArea,
    relation: &str,
) -> Result<()> {
    let id = ensure_area(connection, row, area)?;
    diesel::insert_into(l::table)
        .values(AttributionRow {
            project_id: row.project_id.clone(),
            event_id: row.id.clone(),
            area_id: id,
            relation: relation.into(),
        })
        .on_conflict_do_nothing()
        .execute(connection)?;
    Ok(())
}
fn ensure_area(
    connection: &mut DbConnection,
    row: &ObservationRow,
    area: &ConcernArea,
) -> Result<String> {
    let id = area.identity();
    let existing = a::table
        .find((&row.project_id, &id))
        .select(AreaRow::as_select())
        .first::<AreaRow>(connection)
        .optional()?;
    match existing {
        Some(existing) => check_area(&existing, area)?,
        None => insert_area(connection, &row.project_id, &id, area)?,
    };
    Ok(id)
}
fn check_area(existing: &AreaRow, area: &ConcernArea) -> Result<()> {
    let prior: ConcernArea = serde_json::from_str(&existing.area_json)?;
    ensure!(
        prior.repository == area.repository
            && prior.kind == area.kind
            && prior.key == area.key
            && prior.discriminator == area.discriminator,
        ConcernRejection::Conflict(format!("area identity {}", existing.id))
    );
    Ok(())
}
fn insert_area(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
    area: &ConcernArea,
) -> Result<()> {
    let mut resolved = area.clone();
    resolved.id = Some(id.into());
    let row = AreaRow {
        project_id: project.into(),
        id: id.into(),
        repository: area.repository.clone(),
        kind: area.kind.as_str().into(),
        area_key: area.key.clone(),
        area_json: serde_json::to_string(&resolved)?,
    };
    diesel::insert_into(a::table)
        .values(row)
        .execute(connection)?;
    Ok(())
}
