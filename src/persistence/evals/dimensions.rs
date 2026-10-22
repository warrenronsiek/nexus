// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::rows::DimensionRow;
use crate::evals::*;
use crate::persistence::backend::{DbBackend, DbConnection};
use crate::persistence::schema::eval_dimensions;
use anyhow::{bail, Result};
use diesel::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum Scope {
    Both,
    Repository,
}
type Source = (String, Scope);

pub(super) fn publish(
    connection: &mut DbConnection,
    project: &str,
    input: &EvalRecordInput,
    pending: &BTreeMap<&str, &EvalRecordInput>,
) -> Result<()> {
    let mut walk = DimensionWalk::new(project, input, pending);
    walk.collect(connection)?;
    for (dimension, value) in walk.dimensions {
        diesel::insert_into(eval_dimensions::table)
            .values(DimensionRow {
                project_id: project.into(),
                record_id: input.id.clone(),
                dimension,
                value,
            })
            .execute(connection)?;
    }
    Ok(())
}

struct DimensionWalk<'a, 'b> {
    project: &'a str,
    pending: &'a BTreeMap<&'b str, &'b EvalRecordInput>,
    queue: Vec<Source>,
    seen: HashSet<Source>,
    dimensions: BTreeSet<(String, String)>,
}
impl<'a, 'b> DimensionWalk<'a, 'b> {
    fn new(
        project: &'a str,
        input: &EvalRecordInput,
        pending: &'a BTreeMap<&'b str, &'b EvalRecordInput>,
    ) -> Self {
        Self {
            project,
            pending,
            queue: vec![(input.id.clone(), Scope::Both)],
            seen: HashSet::new(),
            dimensions: BTreeSet::new(),
        }
    }
    fn collect(&mut self, connection: &mut DbConnection) -> Result<()> {
        while let Some((id, scope)) = self.queue.pop() {
            self.visit(connection, id, scope)?;
        }
        Ok(())
    }
    fn visit(&mut self, connection: &mut DbConnection, id: String, scope: Scope) -> Result<()> {
        if !self.seen.insert((id.clone(), scope)) {
            return Ok(());
        }
        if self.seen.len() > 4096 {
            bail!("eval dimension ancestry exceeds 4096 records");
        }
        match self.pending.get(id.as_str()).copied() {
            Some(record) => self.observe_record(record, scope),
            None => self.observe_stored(connection, &id, scope)?,
        }
        Ok(())
    }
    fn observe_record(&mut self, record: &EvalRecordInput, scope: Scope) {
        self.dimensions.extend(
            own(&record.body)
                .into_iter()
                .filter(|(d, _)| scope == Scope::Both || d == "repository"),
        );
        self.queue
            .extend(sources(&record.body).into_iter().map(|(id, child_scope)| {
                (
                    id,
                    if scope == Scope::Repository {
                        scope
                    } else {
                        child_scope
                    },
                )
            }));
    }
    fn observe_stored(
        &mut self,
        connection: &mut DbConnection,
        id: &str,
        scope: Scope,
    ) -> Result<()> {
        let mut query = eval_dimensions::table
            .filter(eval_dimensions::project_id.eq(self.project))
            .filter(eval_dimensions::record_id.eq(id))
            .into_boxed::<DbBackend>();
        if scope == Scope::Repository {
            query = query.filter(eval_dimensions::dimension.eq("repository"));
        }
        self.dimensions.extend(
            query
                .select((eval_dimensions::dimension, eval_dimensions::value))
                .load::<(String, String)>(connection)?,
        );
        Ok(())
    }
}

fn own(body: &EvalBody) -> Vec<(String, String)> {
    match body {
        EvalBody::Repository(v) => vec![("repository".into(), v.repository.clone())],
        EvalBody::Model(v) => vec![("model".into(), v.label.clone())],
        EvalBody::Artifact(v) => v
            .backings
            .iter()
            .filter_map(|b| match b {
                ArtifactBacking::GitFile { repository, .. }
                | ArtifactBacking::GitLfs { repository, .. } => {
                    Some(("repository".into(), repository.clone()))
                }
                ArtifactBacking::StoredBlob { .. } => None,
            })
            .collect(),
        _ => vec![],
    }
}

fn source(id: &str) -> Source {
    (id.into(), Scope::Both)
}
fn repository(id: &str) -> Source {
    (id.into(), Scope::Repository)
}

fn sources(body: &EvalBody) -> Vec<Source> {
    let mut result = definitions(body);
    result.extend(execution(body));
    result.extend(assessments(body));
    result
}

fn definitions(body: &EvalBody) -> Vec<Source> {
    match body {
        EvalBody::Target(v) => v
            .components
            .iter()
            .flat_map(|component| {
                let mut refs: Vec<_> = component
                    .repository_id
                    .iter()
                    .map(|id| repository(id))
                    .collect();
                if let ModelAssignment::Fixed { model_id } = &component.model {
                    refs.push(source(model_id));
                }
                refs
            })
            .collect(),
        EvalBody::Case(v) => v.inputs.iter().map(|id| repository(id)).collect(),
        EvalBody::Suite(v) => v
            .cases
            .iter()
            .map(|member| repository(&member.case_id))
            .collect(),
        _ => vec![],
    }
}

fn execution(body: &EvalBody) -> Vec<Source> {
    match body {
        EvalBody::Experiment(v) => vec![source(&v.baseline_id), source(&v.candidate_id)],
        EvalBody::Run(v) => vec![source(&v.target_id)],
        EvalBody::Trial(v) => vec![source(&v.run_id)],
        EvalBody::Attempt(v) => vec![source(&v.trial_id)],
        EvalBody::Lifecycle(v) => vec![source(v.subject.record_id())],
        EvalBody::Call(v) => vec![repository(&v.attempt_id), source(&v.model_id)],
        EvalBody::Trace(v) => v
            .call_id
            .as_ref()
            .map_or_else(|| vec![repository(&v.attempt_id)], |id| vec![source(id)]),
        _ => vec![],
    }
}

fn assessments(body: &EvalBody) -> Vec<Source> {
    match body {
        EvalBody::Grade(v) => vec![source(match &v.subject {
            AssessmentSubject::Trial { id }
            | AssessmentSubject::Attempt { id }
            | AssessmentSubject::Call { id } => id,
            AssessmentSubject::Component {
                attempt_id,
                call_ids,
                ..
            } => {
                let mut sources = vec![repository(attempt_id)];
                sources.extend(call_ids.iter().map(|id| source(id)));
                return sources;
            }
            AssessmentSubject::Routing { trace_id } => trace_id,
        })],
        EvalBody::Comparison(v) => v
            .baseline_ids
            .iter()
            .chain(&v.candidate_ids)
            .map(|id| source(id))
            .collect(),
        EvalBody::Decision(v) => vec![source(&v.subject_id)],
        EvalBody::Selection(v) => v.revision_id.iter().map(|id| source(id)).collect(),
        EvalBody::Verification(v) => vec![repository(&v.artifact_id)],
        _ => vec![],
    }
}
