// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::{series, RecordCache, SOURCE_LIMIT};
use crate::evals::*;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
struct GroupProjection {
    summary: EvalDashboardGroup,
    runs: BTreeSet<String>,
    series: BTreeMap<String, EvalDashboardSeries>,
}
pub(super) struct DashboardProjection<'a> {
    selected: Option<&'a str>,
    groups: BTreeMap<String, GroupProjection>,
    runs: BTreeSet<String>,
    result_count: usize,
    unresolved: usize,
}
impl<'a> DashboardProjection<'a> {
    pub(super) fn new(selected: Option<&'a str>) -> Self {
        Self {
            selected,
            groups: BTreeMap::new(),
            runs: BTreeSet::new(),
            result_count: 0,
            unresolved: 0,
        }
    }
    pub(super) fn observe(
        &mut self,
        cache: &mut RecordCache<'_>,
        record: &EvalRecordInput,
        superseded: &BTreeSet<String>,
    ) -> Result<()> {
        let Some((run, summary)) = resolve_group(cache, record)? else {
            self.unresolved += 1;
            return Ok(());
        };
        self.runs.insert(run.id.clone());
        let group = self
            .groups
            .entry(summary.id.clone())
            .or_insert_with(|| GroupProjection {
                summary,
                runs: BTreeSet::new(),
                series: BTreeMap::new(),
            });
        group.observe_run(cache, &run)?;
        if record.body.assessment_subject().is_some() {
            if !superseded.contains(&record.id) {
                group.summary.result_count += 1;
                self.result_count += 1;
            }
            group.summary.latest_at = group.summary.latest_at.max(record.occurred_at);
            if self.selected == Some(group.summary.id.as_str()) {
                series::observe(
                    cache,
                    &mut group.series,
                    record,
                    &run,
                    superseded.contains(&record.id),
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn finish(self, scanned: usize, truncated: bool) -> Result<EvalDashboard> {
        let mut view = EvalDashboard {
            groups: vec![],
            selected_group_id: self.selected.map(str::to_owned),
            series: vec![],
            run_count: self.runs.len(),
            result_count: self.result_count,
            scanned_records: scanned,
            source_limit: SOURCE_LIMIT,
            truncated,
            notices: vec![],
        };
        for (_, group) in self.groups {
            if self.selected == Some(group.summary.id.as_str()) {
                view.series = group.series.into_values().collect();
            }
            view.groups.push(group.summary);
        }
        view.groups
            .sort_by(|a, b| b.latest_at.cmp(&a.latest_at).then(a.id.cmp(&b.id)));
        if truncated {
            view.notices.push("source_limit: most recently received run/assessment records or their dependencies exceeded 4096 records / 8 MiB; coverage and current-frontier counts are partial".into());
        }
        if self.unresolved > 0 {
            view.truncated = true;
            view.notices.push(format!("{0} source records could not resolve their run/definition context within the bounded dependency view",self.unresolved));
        }
        if view.selected_group_id.is_some() && view.series.is_empty() {
            view.notices
                .push("No result series in the selected captured group".into());
        }
        series::bound_output(&mut view)?;
        view.notices.push("Series separate exact metric/grader revisions and assessment scope; units/names do not establish compatibility. Numeric chart values are optional presentation approximations; exact text remains authoritative. No optimization direction is inferred.".into());
        Ok(view)
    }
}
impl GroupProjection {
    fn observe_run(&mut self, cache: &mut RecordCache<'_>, run: &EvalRecordInput) -> Result<()> {
        let EvalBody::Run(body) = &run.body else {
            return Ok(());
        };
        insert_unique(&mut self.summary.suite_revision_ids, &body.suite_id);
        insert_unique(&mut self.summary.target_revision_ids, &body.target_id);
        if self.runs.insert(run.id.clone()) {
            self.summary.run_count += 1;
            let state = run_state(cache, run)?;
            *self.summary.statuses.entry(state).or_default() += 1;
        }
        Ok(())
    }
}
fn insert_unique(values: &mut Vec<String>, id: &str) {
    if !values.iter().any(|value| value == id) {
        values.push(id.into());
        values.sort();
    }
}
fn run_state(cache: &mut RecordCache<'_>, run: &EvalRecordInput) -> Result<String> {
    let lifecycle = super::super::lifecycle::projection(cache.connection, cache.project, run)?;
    let Some(lifecycle) = lifecycle else {
        return Ok("unknown".into());
    };
    if lifecycle.truncated {
        cache.truncated = true;
        return Ok("unknown".into());
    }
    if lifecycle.frontiers.len() > 1 {
        return Ok("conflicted".into());
    }
    let state = lifecycle
        .frontiers
        .first()
        .map_or(lifecycle.initial_state, |event| event.state);
    Ok(serde_json::to_value(state)?
        .as_str()
        .unwrap_or("unknown")
        .into())
}
fn resolve_group(
    cache: &mut RecordCache<'_>,
    record: &EvalRecordInput,
) -> Result<Option<(EvalRecordInput, EvalDashboardGroup)>> {
    let Some(run) = resolve_run(cache, record)? else {
        return Ok(None);
    };
    Ok(group_summary(cache, record, &run)?.map(|summary| (run, summary)))
}
fn group_summary(
    cache: &mut RecordCache<'_>,
    record: &EvalRecordInput,
    run: &EvalRecordInput,
) -> Result<Option<EvalDashboardGroup>> {
    let EvalBody::Run(body) = &run.body else {
        return Ok(None);
    };
    let Some(context) = group_context(cache, body)? else {
        return Ok(None);
    };
    let dimensions = cache.dimensions(&record.id)?;
    let repository: Vec<_> = dimensions
        .iter()
        .filter(|(kind, _)| kind == "repository")
        .map(|(_, value)| value.clone())
        .collect();
    let models: Vec<_> = dimensions
        .iter()
        .filter(|(kind, _)| kind == "model")
        .map(|(_, value)| value.clone())
        .collect();
    let key = serde_json::to_vec(&(
        &context.suite_identity,
        &context.target_identity,
        &context.components,
        &repository,
        &models,
    ))?;
    Ok(Some(EvalDashboardGroup {
        id: blake3::hash(&key).to_hex().to_string(),
        label: context.label,
        suite_identity: context.suite_identity,
        suite_revision_ids: vec![body.suite_id.clone()],
        target_identity: context.target_identity,
        target_revision_ids: vec![body.target_id.clone()],
        repository,
        models,
        components: context.components,
        run_count: 0,
        result_count: 0,
        latest_at: run.occurred_at,
        statuses: BTreeMap::new(),
    }))
}
struct GroupContext {
    label: String,
    suite_identity: String,
    target_identity: String,
    components: Vec<String>,
}
fn group_context(cache: &mut RecordCache<'_>, run: &Run) -> Result<Option<GroupContext>> {
    let (Some(suite), Some(target)) = (cache.get(&run.suite_id)?, cache.get(&run.target_id)?)
    else {
        return Ok(None);
    };
    match (&suite.body, &target.body) {
        (EvalBody::Suite(suite_body), EvalBody::Target(target_body)) => Ok(Some(GroupContext {
            label: suite.title.clone(),
            suite_identity: suite_body.revision.identity.clone(),
            target_identity: target_body.revision.identity.clone(),
            components: component_labels(cache, &target_body.components)?,
        })),
        _ => Ok(None),
    }
}
fn component_labels(cache: &mut RecordCache<'_>, components: &[Component]) -> Result<Vec<String>> {
    let mut labels = vec![];
    for component in components {
        let assignment = match &component.model {
            ModelAssignment::Fixed { model_id } => {
                match cache.get(model_id)?.map(|record| record.body) {
                    Some(EvalBody::Model(model)) => format!(
                        "declared {} [{}:{}]",
                        model.label, model.provider, model.requested_model
                    ),
                    _ => format!("declared model {model_id} (context unavailable)"),
                }
            }
            ModelAssignment::Policy { artifact_id } => {
                format!("policy {artifact_id}; observed models listed separately")
            }
            ModelAssignment::Unspecified { .. } => "unspecified model".into(),
        };
        let repository = match component.repository_id.as_deref() {
            Some(id) => match cache.get(id)?.map(|record| record.body) {
                Some(EvalBody::Repository(repo)) => repo.repository,
                _ => "unknown repository".into(),
            },
            None => "no repository".into(),
        };
        labels.push(format!(
            "{} ({}) · {} · {}",
            component.id, component.role, assignment, repository
        ));
    }
    labels.sort();
    Ok(labels)
}

fn resolve_run(
    cache: &mut RecordCache<'_>,
    record: &EvalRecordInput,
) -> Result<Option<EvalRecordInput>> {
    let mut current = record.clone();
    for _ in 0..16 {
        if matches!(current.body, EvalBody::Run(_)) {
            return Ok(Some(current));
        }
        let Some(id) = execution_parent(&current.body) else {
            return Ok(None);
        };
        let Some(next) = cache.get(id)? else {
            return Ok(None);
        };
        current = next;
    }
    cache.truncated = true;
    Ok(None)
}
pub(super) fn execution_parent(body: &EvalBody) -> Option<&str> {
    match body {
        EvalBody::Trial(value) => Some(&value.run_id),
        EvalBody::Attempt(value) => Some(&value.trial_id),
        EvalBody::Call(value) => Some(&value.attempt_id),
        EvalBody::Trace(value) => Some(&value.attempt_id),
        _ => body.assessment_subject().map(|subject| match subject {
            AssessmentSubject::Run { id }
            | AssessmentSubject::Trial { id }
            | AssessmentSubject::Attempt { id }
            | AssessmentSubject::Call { id } => id.as_str(),
            AssessmentSubject::Component { attempt_id, .. } => attempt_id.as_str(),
            AssessmentSubject::Routing { trace_id } => trace_id.as_str(),
        }),
    }
}
