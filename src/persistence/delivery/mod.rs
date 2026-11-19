// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
// @entrypoint Store::delivery_command
mod projection;
mod queries;
mod remediation;
mod rows;
use crate::{
    delivery::*,
    persistence::{backend::DbConnection, schema::delivery_observations, Store},
    work_history::WorkActor,
};
use anyhow::{bail, Result};
use chrono::Utc;
use diesel::prelude::*;
use rows::ObservationRow;
impl Store {
    pub(crate) fn remediation_notices(
        &mut self,
        project: &str,
        agent: &str,
        session: &str,
    ) -> Result<Vec<RemediationNotice>> {
        remediation::pending_notices(&mut self.connection, project, agent, session)
    }

    pub fn delivery_command(
        &mut self,
        project: &str,
        actor: &WorkActor,
        action: &DeliveryAction,
    ) -> Result<DeliveryResponse> {
        if project.is_empty() || project.len() > 512 {
            bail!(DeliveryRejection::Invalid("project identity".into()))
        }
        match action {
            DeliveryAction::Record { observations } => {
                self.transaction(|connection| record(connection, project, actor, observations, &[]))
            }
            DeliveryAction::Remediate { request } => self.transaction(|connection| {
                remediation::request(connection, project, actor, request)
            }),
            DeliveryAction::Acknowledge { id } => self
                .transaction(|connection| remediation::acknowledge(connection, project, actor, id)),
            DeliveryAction::Import { observations } => {
                let inputs = observations
                    .iter()
                    .map(|v| v.input.clone())
                    .collect::<Vec<_>>();
                self.transaction(|connection| {
                    record(connection, project, actor, &inputs, observations)
                })
            }
            _ => queries::read(&mut self.connection, project, actor, action),
        }
    }
}
pub(super) fn record(
    connection: &mut DbConnection,
    project: &str,
    actor: &WorkActor,
    inputs: &[DeliveryObservationInput],
    imports: &[DeliveryObservation],
) -> Result<DeliveryResponse> {
    validate_batch(inputs, imports)?;
    actor
        .validate()
        .map_err(|error| DeliveryRejection::Invalid(error.to_string()))?;
    let mut journal = DeliveryJournal {
        connection,
        project,
        actor,
        imports,
    };
    let mut inserted = 0;
    let mut replayed = 0;
    for input in inputs {
        if journal.append(input)? {
            inserted += 1;
        } else {
            replayed += 1;
        }
    }
    Ok(DeliveryResponse::Recorded { inserted, replayed })
}

struct DeliveryJournal<'a> {
    connection: &'a mut DbConnection,
    project: &'a str,
    actor: &'a WorkActor,
    imports: &'a [DeliveryObservation],
}
impl DeliveryJournal<'_> {
    fn append(&mut self, input: &DeliveryObservationInput) -> Result<bool> {
        input
            .validate()
            .map_err(|error| DeliveryRejection::Invalid(error.to_string()))?;
        let payload = serde_json::to_string(input)?;
        let hash = blake3::hash(payload.as_bytes()).to_hex().to_string();
        let existing = delivery_observations::table
            .find((self.project, &input.id))
            .select(delivery_observations::content_hash)
            .first::<String>(self.connection)
            .optional()?;
        match existing {
            Some(existing) if existing == hash => Ok(false),
            Some(_) => Err(DeliveryRejection::Conflict(input.id.clone()).into()),
            None => {
                let row = self.new_row(input, payload, hash)?;
                self.publish(&row)?;
                Ok(true)
            }
        }
    }
    fn new_row(
        &mut self,
        input: &DeliveryObservationInput,
        payload: String,
        hash: String,
    ) -> Result<ObservationRow> {
        let subject = input.body.subject();
        let ordinal = delivery_observations::table
            .filter(delivery_observations::project_id.eq(self.project))
            .select(diesel::dsl::max(delivery_observations::ordinal))
            .first::<Option<i64>>(self.connection)?
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| DeliveryRejection::Invalid("delivery ordinal exhausted".into()))?;
        let source = self
            .imports
            .iter()
            .find(|old| old.input.id == input.id)
            .map(|old| {
                crate::record_source::RecordSource::retained(
                    old.source.as_ref(),
                    &old.project_id,
                    old.recorded_at,
                    &old.provenance,
                )
            });
        Ok(ObservationRow {
            project_id: self.project.into(),
            id: input.id.clone(),
            ordinal,
            subject_kind: subject
                .as_ref()
                .map(|value| value.kind.as_str())
                .unwrap_or("watch")
                .into(),
            subject_id: subject
                .as_ref()
                .map(|value| value.id.clone())
                .unwrap_or_default(),
            occurred_at: input.occurred_at.to_rfc3339(),
            recorded_at: Utc::now().to_rfc3339(),
            payload_json: payload,
            content_hash: hash,
            provenance_json: serde_json::to_string(self.actor)?,
            source_json: source.as_ref().map(serde_json::to_string).transpose()?,
        })
    }
    fn publish(&mut self, row: &ObservationRow) -> Result<()> {
        projection::apply(self.connection, self.project, &row.observation()?)?;
        diesel::insert_into(delivery_observations::table)
            .values(row)
            .execute(self.connection)?;
        use crate::persistence::schema::projects;
        diesel::insert_into(projects::table)
            .values((
                projects::project_id.eq(self.project),
                projects::first_seen_at.eq(&row.recorded_at),
                projects::last_seen_at.eq(&row.recorded_at),
            ))
            .on_conflict(projects::project_id)
            .do_update()
            .set(projects::last_seen_at.eq(&row.recorded_at))
            .execute(self.connection)?;
        Ok(())
    }
}
fn validate_batch(
    inputs: &[DeliveryObservationInput],
    imports: &[DeliveryObservation],
) -> Result<()> {
    if inputs.is_empty() || inputs.len() > 64 {
        bail!(DeliveryRejection::Invalid(
            "batch must contain 1..64 observations".into()
        ));
    }
    let bytes = if imports.is_empty() {
        serde_json::to_vec(inputs)?
    } else {
        serde_json::to_vec(imports)?
    };
    if bytes.len() > 768 * 1024 {
        bail!(DeliveryRejection::Invalid(
            "delivery batch exceeds 768 KiB".into()
        ));
    }
    for observation in imports {
        validate_original(observation)
            .map_err(|error| DeliveryRejection::Invalid(error.to_string()))?;
    }
    Ok(())
}

fn validate_original(observation: &DeliveryObservation) -> Result<()> {
    crate::config::canonical_project_id(&observation.project_id)
        .map_err(|_| anyhow::anyhow!("invalid original delivery project identity"))?;
    observation.provenance.validate()?;
    if let Some(source) = &observation.source {
        crate::config::canonical_project_id(&source.project_id)
            .map_err(|_| anyhow::anyhow!("invalid source delivery project identity"))?;
        source.provenance.validate()?;
    }
    Ok(())
}
pub(super) fn rebuild_snapshot(connection: &mut DbConnection) -> Result<()> {
    clear_projections(connection)?;
    let mut after_project = String::new();
    let mut after_ordinal = 0;
    loop {
        let rows = journal_page(connection, &after_project, after_ordinal)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            replay_row(connection, &row)?;
            after_project = row.project_id;
            after_ordinal = row.ordinal;
        }
    }
    Ok(())
}

fn clear_projections(connection: &mut DbConnection) -> Result<()> {
    use crate::persistence::schema::{
        delivery_features, delivery_origins, delivery_presence, delivery_records,
        delivery_relations, delivery_targets,
    };
    diesel::delete(delivery_targets::table)
        .execute(connection)
        .and_then(|_| diesel::delete(delivery_features::table).execute(connection))
        .and_then(|_| diesel::delete(delivery_origins::table).execute(connection))
        .and_then(|_| diesel::delete(delivery_presence::table).execute(connection))
        .and_then(|_| diesel::delete(delivery_relations::table).execute(connection))
        .and_then(|_| diesel::delete(delivery_records::table).execute(connection))
        .map(|_| ())
        .map_err(Into::into)
}

fn journal_page(
    connection: &mut DbConnection,
    project: &str,
    ordinal: i64,
) -> Result<Vec<ObservationRow>> {
    Ok(delivery_observations::table
        .filter(
            delivery_observations::project_id
                .gt(project)
                .or(delivery_observations::project_id
                    .eq(project)
                    .and(delivery_observations::ordinal.gt(ordinal))),
        )
        .order((
            delivery_observations::project_id.asc(),
            delivery_observations::ordinal.asc(),
        ))
        .limit(32)
        .select(ObservationRow::as_select())
        .load(connection)?)
}

fn replay_row(connection: &mut DbConnection, row: &ObservationRow) -> Result<()> {
    let observation = row.observation()?;
    observation.input.validate()?;
    validate_original(&observation)?;
    validate_journal_indexes(row, &observation)?;
    if !matches!(
        observation.input.body,
        DeliveryEvent::WatchRegistered { .. } | DeliveryEvent::WatchStopped { .. }
    ) {
        projection::apply(connection, &row.project_id, &observation)?;
    }
    Ok(())
}

fn validate_journal_indexes(row: &ObservationRow, observation: &DeliveryObservation) -> Result<()> {
    if blake3::hash(row.payload_json.as_bytes()).to_hex().as_str() != row.content_hash {
        bail!("corrupt delivery observation hash");
    }
    let subject = observation.input.body.subject();
    if row.id != observation.input.id
        || row.subject_kind
            != subject
                .as_ref()
                .map(|value| value.kind.as_str())
                .unwrap_or("watch")
        || row.subject_id
            != subject
                .as_ref()
                .map(|value| value.id.as_str())
                .unwrap_or("")
        || row.occurred_at != observation.input.occurred_at.to_rfc3339()
    {
        bail!("corrupt delivery observation indexes");
    }
    Ok(())
}
