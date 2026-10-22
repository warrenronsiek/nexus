// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use crate::evals::{AssessmentSubject, EvalBody, EvalRecordInput, LifecycleSubject, Trial};
use crate::persistence::backend::DbConnection;
use anyhow::{bail, ensure, Context, Result};
use std::collections::{BTreeMap, HashSet};

#[derive(PartialEq)]
enum Scope<'a> {
    Trial(&'a str),
    Attempt(&'a str),
    Assessment(&'a AssessmentSubject),
    Measurement(&'a str, &'a AssessmentSubject),
    Selection(&'a str),
    Lifecycle(&'a LifecycleSubject),
}

struct Lineage<'a> {
    parent: Option<&'a str>,
    scope: Scope<'a>,
}

impl<'a> Lineage<'a> {
    fn of(body: &'a EvalBody) -> Option<Self> {
        match body {
            EvalBody::Attempt(value) => Some(Self {
                parent: value.parent_id.as_deref(),
                scope: Scope::Trial(&value.trial_id),
            }),
            EvalBody::Call(value) => Some(Self {
                parent: value.parent_id.as_deref(),
                scope: Scope::Attempt(&value.attempt_id),
            }),
            EvalBody::Grade(value) => Some(Self {
                parent: value.replaces.as_deref(),
                scope: Scope::Assessment(&value.subject),
            }),
            EvalBody::Measurement(value) => Some(Self {
                parent: value.replaces.as_deref(),
                scope: Scope::Measurement(&value.metric_id, &value.subject),
            }),
            EvalBody::Selection(value) => Some(Self {
                parent: value.previous_id.as_deref(),
                scope: Scope::Selection(&value.scope),
            }),
            EvalBody::Lifecycle(value) => Some(Self {
                parent: value.previous_id.as_deref(),
                scope: Scope::Lifecycle(&value.subject),
            }),
            _ => None,
        }
    }
}

struct RecordedGraph<'a, 'b> {
    connection: &'a mut DbConnection,
    project: &'a str,
    pending: &'a BTreeMap<&'b str, &'b EvalRecordInput>,
}

enum Visit {
    Enter(String),
    Leave(String),
}

#[derive(Clone, Copy)]
enum Ancestry {
    Revision,
    Predecessor,
}

impl Ancestry {
    fn of(body: &EvalBody) -> Option<Self> {
        if body.revision().is_some() {
            Some(Self::Revision)
        } else {
            Lineage::of(body).map(|_| Self::Predecessor)
        }
    }

    fn parents(self, body: &EvalBody) -> Result<Vec<String>> {
        match self {
            Self::Revision => Ok(body
                .revision()
                .context("parent must be a revision")?
                .parents
                .clone()),
            Self::Predecessor => Ok(Lineage::of(body)
                .context("parent must belong to a temporal lineage")?
                .parent
                .map(str::to_owned)
                .into_iter()
                .collect()),
        }
    }
}

impl RecordedGraph<'_, '_> {
    fn get(&mut self, id: &str) -> Result<EvalRecordInput> {
        if let Some(input) = self.pending.get(id) {
            return Ok((*input).clone());
        }
        Ok(super::find(self.connection, self.project, id)?
            .with_context(|| format!("missing project-local eval reference: {id}"))?
            .record()?
            .input)
    }

    fn validate_case_membership(&mut self, trial: &Trial) -> Result<()> {
        let EvalBody::Run(run) = self.get(&trial.run_id)?.body else {
            bail!("trial must reference a run");
        };
        let EvalBody::Suite(suite) = self.get(&run.suite_id)?.body else {
            bail!("run must reference a suite");
        };
        ensure!(
            suite
                .cases
                .iter()
                .any(|member| member.case_id == trial.case_id),
            "trial case is not a member of its run's suite"
        );
        Ok(())
    }

    fn validate_ancestry(&mut self, id: &str, ancestry: Ancestry) -> Result<()> {
        let mut active = HashSet::new();
        let mut finished = HashSet::new();
        let mut stack = vec![Visit::Enter(id.to_owned())];
        while let Some(visit) = stack.pop() {
            match visit {
                Visit::Enter(id) => {
                    if finished.contains(&id) {
                        continue;
                    }
                    ensure!(active.insert(id.clone()), "eval ancestry forms a cycle");
                    ensure!(
                        active.len() + finished.len() <= 4096,
                        "eval ancestry exceeds 4096 records"
                    );
                    let record = self.get(&id)?;
                    let parents = ancestry.parents(&record.body)?;
                    stack.push(Visit::Leave(id));
                    stack.extend(parents.into_iter().rev().map(Visit::Enter));
                }
                Visit::Leave(id) => {
                    active.remove(&id);
                    finished.insert(id);
                }
            }
        }
        Ok(())
    }
}

pub(super) fn validate_links(
    connection: &mut DbConnection,
    project: &str,
    input: &EvalRecordInput,
    pending: &BTreeMap<&str, &EvalRecordInput>,
) -> Result<()> {
    let mut graph = RecordedGraph {
        connection,
        project,
        pending,
    };
    validate_component_assessment(&mut graph, &input.body)?;
    if let Some(lineage) = Lineage::of(&input.body) {
        if let Some(parent_id) = lineage.parent {
            let parent = graph.get(parent_id)?;
            ensure!(
                Lineage::of(&parent.body).is_some_and(|parent| parent.scope == lineage.scope),
                "eval parent belongs to a different lineage scope"
            );
        }
    }
    if let EvalBody::Trial(trial) = &input.body {
        graph.validate_case_membership(trial)?;
    }
    if let Some(ancestry) = Ancestry::of(&input.body) {
        graph.validate_ancestry(&input.id, ancestry)?;
    }
    Ok(())
}

fn validate_component_assessment(graph: &mut RecordedGraph<'_, '_>, body: &EvalBody) -> Result<()> {
    let Some(subject) = body.assessment_subject() else {
        return Ok(());
    };
    let AssessmentSubject::Component {
        attempt_id,
        instance,
        call_ids,
    } = subject
    else {
        return Ok(());
    };
    for id in call_ids {
        let EvalBody::Call(call) = graph.get(id)?.body else {
            bail!("component assessment must pin calls");
        };
        ensure!(
            call.attempt_id == *attempt_id && call.instance == *instance,
            "component assessment call belongs to a different attempt or instance"
        );
    }
    Ok(())
}
