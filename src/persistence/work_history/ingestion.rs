// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::rows::*;
use crate::persistence::backend::DbConnection;
use crate::persistence::lookup;
use crate::persistence::schema::{work_links, work_observations, work_subjects};
use crate::persistence::Store;
use crate::work_history::*;
use anyhow::{bail, Result};
use chrono::Utc;
use diesel::prelude::*;
use std::collections::{HashMap, HashSet};

impl Store {
    pub(super) fn record_work(
        &mut self,
        project: &str,
        actor: &WorkActor,
        inputs: &[WorkObservationInput],
        imports: &[WorkObservation],
    ) -> Result<WorkResponse> {
        let publication = Publication::prepare(project, actor, inputs, imports)?;
        self.transaction(|connection| publication.publish(connection))
    }
}

struct Publication<'a> {
    project: &'a str,
    inputs: &'a [WorkObservationInput],
    pending: HashMap<&'a str, &'a WorkObservationInput>,
    rows: Vec<ObservationRow>,
    historical: bool,
}
impl<'a> Publication<'a> {
    fn prepare(
        project: &'a str,
        actor: &WorkActor,
        inputs: &'a [WorkObservationInput],
        imports: &[WorkObservation],
    ) -> Result<Self> {
        validate_batch(project, actor, inputs, imports)?;
        let rows = inputs
            .iter()
            .map(|input| prepare_row(project, actor, input, imports))
            .collect::<Result<_>>()?;
        Ok(Self {
            project,
            inputs,
            pending: inputs
                .iter()
                .map(|input| (input.id.as_str(), input))
                .collect(),
            rows,
            historical: !imports.is_empty(),
        })
    }
    fn publish(self, connection: &mut DbConnection) -> Result<WorkResponse> {
        let mut inserted = 0;
        let mut receipts = Vec::new();
        let mut subjects = HashSet::new();
        for (input, row) in self.inputs.iter().zip(&self.rows) {
            let (receipt, is_new) = self.publish_one(connection, input, row)?;
            receipts.push(receipt);
            if is_new {
                subjects.insert(input.subject.clone());
                inserted += 1;
            }
        }
        for subject in subjects {
            refresh(connection, self.project, &subject)?;
        }
        Ok(WorkResponse::Recorded {
            inserted,
            replayed: self.inputs.len() - inserted,
            receipts,
        })
    }
    fn publish_one(
        &self,
        connection: &mut DbConnection,
        input: &WorkObservationInput,
        row: &ObservationRow,
    ) -> Result<(WorkReceipt, bool)> {
        match lookup::optional::<ObservationRow, _, _>(
            connection,
            work_observations::table,
            (self.project, &input.id),
        )? {
            Some(existing) => existing.replay(row).map(|receipt| (receipt, false)),
            None => self
                .insert(connection, input, row)
                .map(|receipt| (receipt, true)),
        }
    }
    fn insert(
        &self,
        connection: &mut DbConnection,
        input: &WorkObservationInput,
        row: &ObservationRow,
    ) -> Result<WorkReceipt> {
        self.check_relationships(connection, input)?;
        diesel::insert_into(work_observations::table)
            .values(row)
            .execute(connection)?;
        index(connection, row)?;
        row.receipt()
    }
    fn check_relationships(
        &self,
        connection: &mut DbConnection,
        input: &WorkObservationInput,
    ) -> Result<()> {
        if self.historical {
            return Ok(());
        }
        check_correction(connection, self.project, input, &self.pending)?;
        check_known_parent_cycle(connection, self.project, input)
    }
}
fn validate_batch(
    project: &str,
    actor: &WorkActor,
    inputs: &[WorkObservationInput],
    imports: &[WorkObservation],
) -> Result<()> {
    if project.is_empty() || project.len() > 512 || inputs.is_empty() || inputs.len() > 64 {
        return Err(WorkRejection::Invalid("project or batch size".into()).into());
    }
    actor
        .validate()
        .map_err(|error| WorkRejection::Invalid(error.to_string()))?;
    imports
        .iter()
        .try_for_each(|observation| observation.validate())
        .map_err(|error| WorkRejection::Invalid(error.to_string()))?;
    validate_inputs(inputs).map_err(|error| WorkRejection::Invalid(error.to_string()).into())
}
fn validate_inputs(inputs: &[WorkObservationInput]) -> Result<()> {
    let mut identities = HashSet::new();
    for input in inputs {
        input.validate()?;
        if !identities.insert(&input.id) {
            bail!("duplicate event in batch");
        }
    }
    Ok(())
}
fn prepare_row(
    project: &str,
    actor: &WorkActor,
    input: &WorkObservationInput,
    imports: &[WorkObservation],
) -> Result<ObservationRow> {
    let payload = serde_json::to_string(input)?;
    let source = imports
        .iter()
        .find(|old| old.input.id == input.id)
        .map(|old| {
            old.source.clone().unwrap_or_else(|| WorkSource {
                project_id: old.project_id.clone(),
                recorded_at: old.recorded_at,
                provenance: old.provenance.clone(),
            })
        });
    Ok(ObservationRow {
        project_id: project.into(),
        id: input.id.clone(),
        subject_scope: input.subject.scope().into(),
        subject_id: input.subject.id().into(),
        kind: input.body.family().into(),
        occurred_at: input.occurred_at.to_rfc3339(),
        recorded_at: Utc::now().to_rfc3339(),
        content_hash: blake3::hash(payload.as_bytes()).to_hex().to_string(),
        payload_json: payload,
        provenance_json: serde_json::to_string(actor)?,
        source_json: source.as_ref().map(serde_json::to_string).transpose()?,
    })
}

fn check_correction(
    connection: &mut DbConnection,
    project: &str,
    input: &WorkObservationInput,
    pending: &HashMap<&str, &WorkObservationInput>,
) -> Result<()> {
    let mut current = input.clone();
    let mut seen = HashSet::from([input.id.clone()]);
    while let Some(correction) = &current.correction {
        if !seen.insert(correction.replaces.clone()) || seen.len() > 1024 {
            return Err(WorkRejection::Invalid("correction ancestry cycle or limit".into()).into());
        }
        let Some(target) = correction_target(connection, project, &correction.replaces, pending)?
        else {
            break;
        };
        if target.subject != input.subject || target.body.family() != input.body.family() {
            return Err(WorkRejection::Invalid(
                "correction must retain subject and event family".into(),
            )
            .into());
        }
        current = target;
    }
    Ok(())
}
fn correction_target(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
    pending: &HashMap<&str, &WorkObservationInput>,
) -> Result<Option<WorkObservationInput>> {
    match pending.get(id) {
        Some(input) => Ok(Some((*input).clone())),
        None => lookup::optional::<ObservationRow, _, _>(
            connection,
            work_observations::table,
            (project, id),
        )?
        .map(|row| row.observation().map(|observation| observation.input))
        .transpose(),
    }
}

fn check_known_parent_cycle(
    connection: &mut DbConnection,
    project: &str,
    input: &WorkObservationInput,
) -> Result<()> {
    let source_known = diesel::select(diesel::dsl::exists(work_subjects::table.find((
        project,
        input.subject.scope(),
        input.subject.id(),
    ))))
    .get_result::<bool>(connection)?;
    if !source_known {
        return Ok(());
    }
    for (relation, target) in input.body.references() {
        if relation != WorkRelation::Parent {
            continue;
        }
        let Some(mut current) = target.subject() else {
            continue;
        };
        let mut seen = HashSet::new();
        for _ in 0..64 {
            if current == input.subject {
                return Err(WorkRejection::Invalid("known structural parent cycle".into()).into());
            }
            if !seen.insert(current.clone()) {
                break;
            }
            let (links, _) = super::projection::active_links(connection, project, &current)?;
            let parent = links
                .into_iter()
                .find(|link| link.relation == "parent")
                .and_then(|link| link.target.subject());
            let Some(parent) = parent else {
                break;
            };
            current = parent;
        }
    }
    Ok(())
}

pub(super) fn index(connection: &mut DbConnection, row: &ObservationRow) -> Result<()> {
    let observation = row.observation()?;
    let input = &observation.input;
    let placeholder = SubjectRow {
        project_id: row.project_id.clone(),
        scope: row.subject_scope.clone(),
        id: row.subject_id.clone(),
        title: row.subject_id.clone(),
        kind: row.subject_scope.clone(),
        repository: None,
        host_session_id: Some(observation.provenance.session_id),
        environment: None,
        outcome: None,
        occurred_at: row.occurred_at.clone(),
        recorded_at: row.recorded_at.clone(),
    };
    diesel::insert_into(work_subjects::table)
        .values(&placeholder)
        .on_conflict_do_nothing()
        .execute(connection)?;
    for (relation, target) in input.body.references() {
        let (target_scope, target_id) = match &target {
            WorkReference::Session { id } => ("session", id.clone()),
            WorkReference::Activity { id } => ("activity", id.clone()),
            WorkReference::Eval { id } => ("eval", id.clone()),
            WorkReference::Revision {
                repository, commit, ..
            } => ("revision", format!("{repository}@{commit}")),
            WorkReference::External { provider, id, .. } => {
                ("external", format!("{provider}:{id}"))
            }
        };
        let link = LinkRow {
            project_id: row.project_id.clone(),
            event_id: input.id.clone(),
            source_scope: input.subject.scope().into(),
            source_id: input.subject.id().into(),
            relation: relation.as_str().into(),
            target_scope: target_scope.into(),
            target_id,
            target_json: serde_json::to_string(&target)?,
        };
        diesel::insert_into(work_links::table)
            .values(&link)
            .on_conflict_do_nothing()
            .execute(connection)?;
    }
    Ok(())
}

pub(super) fn refresh(
    connection: &mut DbConnection,
    project: &str,
    subject: &WorkSubject,
) -> Result<()> {
    let Some(detail) = super::projection::detail(connection, project, subject)? else {
        return Ok(());
    };
    let summary = detail.summary;
    let outcome = summary
        .outcome
        .map(serde_json::to_value)
        .transpose()?
        .and_then(|value| value.as_str().map(str::to_owned));
    let source_time = source_window_time(connection, project, subject, summary.started_at)?;
    diesel::update(work_subjects::table.find((project, subject.scope(), subject.id())))
        .set((
            work_subjects::title.eq(summary.title),
            work_subjects::kind.eq(summary.kind),
            work_subjects::repository.eq(summary.context.repository),
            work_subjects::host_session_id.eq(summary.context.host_session_id),
            work_subjects::environment.eq(summary.context.environment),
            work_subjects::outcome.eq(outcome),
            work_subjects::occurred_at.eq(source_time),
        ))
        .execute(connection)?;
    Ok(())
}
fn source_window_time(
    connection: &mut DbConnection,
    project: &str,
    subject: &WorkSubject,
    start: Option<chrono::DateTime<Utc>>,
) -> Result<String> {
    if let Some(time) = start {
        return Ok(time.to_rfc3339());
    }
    Ok(work_observations::table
        .filter(work_observations::project_id.eq(project))
        .filter(work_observations::subject_scope.eq(subject.scope()))
        .filter(work_observations::subject_id.eq(subject.id()))
        .order((
            work_observations::recorded_at.asc(),
            work_observations::id.asc(),
        ))
        .select(work_observations::occurred_at)
        .first(connection)?)
}
