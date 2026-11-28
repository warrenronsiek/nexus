// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::{series, RecordCache, Selection, SOURCE_LIMIT};
use crate::evals::*;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
struct GroupProjection {
    summary: EvalDashboardGroup,
    runs: BTreeSet<String>,
    series: BTreeMap<String, EvalDashboardSeries>,
}
pub(super) struct DashboardProjection<'a> {
    selected: Selection<'a>,
    groups: BTreeMap<String, GroupProjection>,
    runs: BTreeMap<String, EvalRecordInput>,
    result_count: usize,
    unresolved: usize,
    run_context: BTreeMap<String, Option<GroupContext>>,
}
impl<'a> DashboardProjection<'a> {
    pub(super) fn new(selected: Selection<'a>) -> Self {
        Self {
            selected,
            groups: BTreeMap::new(),
            runs: BTreeMap::new(),
            result_count: 0,
            unresolved: 0,
            run_context: BTreeMap::new(),
        }
    }
    pub(super) fn observe(
        &mut self,
        cache: &mut RecordCache<'_>,
        record: &EvalRecordInput,
        superseded: &BTreeSet<String>,
    ) -> Result<()> {
        let Some(run) = resolve_run(cache, record)? else {
            self.unresolved += 1;
            return Ok(());
        };
        let Some(summary) = self.resolve_summary(cache, record, &run)? else {
            self.unresolved += 1;
            return Ok(());
        };
        let group = self
            .groups
            .entry(summary.id.clone())
            .or_insert_with(|| GroupProjection {
                summary,
                runs: BTreeSet::new(),
                series: BTreeMap::new(),
            });
        group.observe_run(&run);
        if record.body.assessment_subject().is_some() {
            if !superseded.contains(&record.id) {
                group.summary.result_count += 1;
                self.result_count += 1;
            }
            group.summary.latest_at = group.summary.latest_at.max(record.occurred_at);
            if self.selected.includes(&group.summary) {
                series::observe(
                    cache,
                    &mut group.series,
                    record,
                    &run,
                    superseded.contains(&record.id),
                )?;
            }
        }
        self.runs.entry(run.id.clone()).or_insert(run);
        Ok(())
    }

    fn resolve_summary(
        &mut self,
        cache: &mut RecordCache<'_>,
        record: &EvalRecordInput,
        run: &EvalRecordInput,
    ) -> Result<Option<EvalDashboardGroup>> {
        let EvalBody::Run(body) = &run.body else {
            unreachable!("resolved run has a run body");
        };
        let context = match self.run_context.entry(run.id.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(group_context(cache, body)?)
            }
        };
        context
            .as_ref()
            .map(|context| group_summary(cache, record, run, body, context))
            .transpose()
    }

    pub(super) fn project_run_states(&mut self, cache: &mut RecordCache<'_>) -> Result<()> {
        let runs: Vec<_> = self.runs.values().collect();
        let mut states = BTreeMap::new();
        for page in runs.chunks(64) {
            let ids: Vec<_> = page.iter().map(|run| run.id.as_str()).collect();
            let batch =
                super::super::lifecycle::dashboard_states(cache.connection, cache.project, &ids)?;
            for run in page {
                let state = dashboard_run_state(
                    cache,
                    run,
                    batch
                        .as_ref()
                        .and_then(|rows| rows.get(&run.id).map(Vec::as_slice)),
                )?;
                states.insert(run.id.as_str(), state);
            }
        }
        for group in self.groups.values_mut() {
            for run in &group.runs {
                *group
                    .summary
                    .statuses
                    .entry(states[run.as_str()].clone())
                    .or_default() += 1;
            }
        }
        Ok(())
    }

    pub(super) fn finish(mut self, scanned: usize, truncated: bool) -> Result<EvalDashboard> {
        let mut view = EvalDashboard {
            groups: vec![],
            selected_group_id: match self.selected {
                Selection::Group(id) => id.map(str::to_owned),
                Selection::Agent(_) => None,
            },
            selected_agent: match self.selected {
                Selection::Agent(agent) => Some(agent.to_owned()),
                _ => None,
            },
            series: vec![],
            run_count: self.runs.len(),
            result_count: self.result_count,
            scanned_records: scanned,
            source_limit: SOURCE_LIMIT,
            truncated,
            notices: vec![],
        };
        let selected_runs = self.collect_groups(&mut view);
        view.groups
            .sort_by(|a, b| b.latest_at.cmp(&a.latest_at).then(a.id.cmp(&b.id)));
        disclose_bounds(&mut view, truncated, self.unresolved);
        if let Selection::Agent(_) = self.selected {
            view.run_count = selected_runs.len();
            view.result_count = view
                .groups
                .iter()
                .filter(|group| self.selected.includes(group))
                .map(|group| group.result_count)
                .sum();
        }
        series::bound_output(&mut view)?;
        view.notices.push("Series separate exact metric/grader revisions and assessment scope; units/names do not establish compatibility. Numeric chart values are optional presentation approximations; exact text remains authoritative. No optimization direction is inferred.".into());
        Ok(view)
    }
    fn collect_groups(&mut self, view: &mut EvalDashboard) -> BTreeSet<String> {
        let mut selected_runs = BTreeSet::new();
        let mut series_by_id = BTreeMap::new();
        for (_, group) in std::mem::take(&mut self.groups) {
            if self.selected.includes(&group.summary) {
                selected_runs.extend(group.runs);
                for (id, series) in group.series {
                    merge_series(&mut series_by_id, id, series);
                }
            }
            view.groups.push(group.summary);
        }
        view.series = series_by_id.into_values().collect();
        selected_runs
    }
}
fn disclose_bounds(view: &mut EvalDashboard, truncated: bool, unresolved: usize) {
    if truncated {
        view.notices.push("source_limit: most recently received run/assessment records or their dependencies exceeded 4096 records / 8 MiB; coverage and current-frontier counts are partial".into());
    }
    if unresolved > 0 {
        view.truncated = true;
        view.notices.push(format!("{0} source records could not resolve their run/definition context within the bounded dependency view",unresolved));
    }
    if view.selected_group_id.is_some() && view.series.is_empty() {
        view.notices
            .push("No result series in the selected captured group".into());
    }
}

fn merge_series(
    series: &mut BTreeMap<String, EvalDashboardSeries>,
    id: String,
    incoming: EvalDashboardSeries,
) {
    match series.entry(id) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(incoming);
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            entry.get_mut().points.extend(incoming.points);
            entry.get_mut().superseded_count += incoming.superseded_count;
        }
    }
}

impl GroupProjection {
    fn observe_run(&mut self, run: &EvalRecordInput) {
        let EvalBody::Run(body) = &run.body else {
            return;
        };
        insert_unique(&mut self.summary.suite_revision_ids, &body.suite_id);
        insert_unique(&mut self.summary.target_revision_ids, &body.target_id);
        if self.runs.insert(run.id.clone()) {
            self.summary.run_count += 1;
        }
    }
}

fn insert_unique(values: &mut Vec<String>, id: &str) {
    if !values.iter().any(|value| value == id) {
        values.push(id.into());
        values.sort();
    }
}
fn dashboard_run_state(
    cache: &mut RecordCache<'_>,
    run: &EvalRecordInput,
    frontiers: Option<&[ReportedState]>,
) -> Result<String> {
    if let Some(frontiers) = frontiers {
        let EvalBody::Run(body) = &run.body else {
            unreachable!("state requires a run");
        };
        return match frontiers {
            [] => state_name(body.state),
            [state] => state_name(*state),
            states if states.len() > 64 => {
                cache.truncated = true;
                Ok("unknown".into())
            }
            _ => Ok("conflicted".into()),
        };
    }
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
    state_name(state)
}

fn state_name(state: ReportedState) -> Result<String> {
    Ok(serde_json::to_value(state)?
        .as_str()
        .unwrap_or("unknown")
        .into())
}
fn group_summary(
    cache: &mut RecordCache<'_>,
    record: &EvalRecordInput,
    run: &EvalRecordInput,
    body: &Run,
    context: &GroupContext,
) -> Result<EvalDashboardGroup> {
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
    let label = group_label(context, &models);
    Ok(EvalDashboardGroup {
        id: blake3::hash(&key).to_hex().to_string(),
        label,
        agent: context.agent.clone(),
        agent_declared: context.agent_declared,
        suite_identity: context.suite_identity.clone(),
        suite_revision_ids: vec![body.suite_id.clone()],
        target_identity: context.target_identity.clone(),
        target_revision_ids: vec![body.target_id.clone()],
        repository,
        models,
        components: context.components.clone(),
        run_count: 0,
        result_count: 0,
        latest_at: run.occurred_at,
        statuses: BTreeMap::new(),
    })
}
struct GroupContext {
    agent: String,
    agent_declared: bool,
    suite_name: String,
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
        (EvalBody::Suite(suite_body), EvalBody::Target(target_body)) => {
            let (agent, agent_declared) = target_agent(target_body);
            Ok(Some(GroupContext {
                suite_name: suite_name(&suite.title, &suite_body.revision.identity, &agent),
                agent,
                agent_declared,
                suite_identity: suite_body.revision.identity.clone(),
                target_identity: target_body.revision.identity.clone(),
                components: component_labels(cache, &target_body.components)?,
            }))
        }
        _ => Ok(None),
    }
}
/// A declared agent wins; a single-component target is named by that component; otherwise the
/// target identity is the most specific stable name available.
fn target_agent(target: &TargetRevision) -> (String, bool) {
    if let Some(agent) = &target.agent {
        return (agent.clone(), true);
    }
    match target.components.as_slice() {
        [component] => (component.id.clone(), false),
        _ => (target.revision.identity.clone(), false),
    }
}
/// Generated suite titles such as `suite/part-0` only repeat the identity's tail, so the
/// identity (without a redundant agent prefix) distinguishes otherwise identical groups.
fn suite_name(title: &str, identity: &str, agent: &str) -> String {
    if !identity.ends_with(title) {
        return title.to_owned();
    }
    identity
        .strip_prefix(agent)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(identity)
        .to_owned()
}
fn group_label(context: &GroupContext, models: &[String]) -> String {
    let mut label = format!("{} · {}", context.agent, context.suite_name);
    if let Some((model, rest)) = models.split_first() {
        label.push_str(" · ");
        label.push_str(model);
        if !rest.is_empty() {
            label.push_str(&format!(" +{}", rest.len()));
        }
    }
    label
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
