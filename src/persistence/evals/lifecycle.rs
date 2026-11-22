// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::rows::RecordRow;
use crate::evals::{EvalBody, EvalRecordInput, LifecycleFrontier, LifecycleProjection};
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{eval_links, eval_records};
use anyhow::{bail, Result};
use diesel::prelude::*;

pub(super) fn projection(
    connection: &mut DbConnection,
    project: &str,
    input: &EvalRecordInput,
) -> Result<Option<LifecycleProjection>> {
    let initial_state = match &input.body {
        EvalBody::Run(run) => run.state,
        EvalBody::Trial(trial) => trial.state,
        EvalBody::Attempt(attempt) => attempt.state,
        _ => return Ok(None),
    };
    let rows = frontier_rows(connection, project, &input.id)?;
    Ok(Some(LifecycleProjection {
        initial_state,
        truncated: rows.len() > 64,
        frontiers: rows
            .into_iter()
            .take(64)
            .map(frontier)
            .collect::<Result<_>>()?,
    }))
}

fn frontier_rows(
    connection: &mut DbConnection,
    project: &str,
    subject: &str,
) -> Result<Vec<RecordRow>> {
    let subjects = eval_links::table
        .filter(eval_links::project_id.eq(project))
        .filter(eval_links::relation.eq("lifecycle_subject"))
        .filter(eval_links::target_id.eq(subject))
        .select(eval_links::source_id);
    // A scalar probe prevents an anti join from scanning all links under stale project statistics.
    let predecessors = eval_links::table
        .filter(eval_links::project_id.eq(project))
        .filter(eval_links::relation.eq("previous_lifecycle"))
        .filter(eval_links::target_id.eq(eval_records::id))
        .select(eval_links::target_id);
    Ok(eval_records::table
        .filter(eval_records::project_id.eq(project))
        .filter(eval_records::kind.eq("lifecycle"))
        .filter(eval_records::id.eq_any(subjects))
        .filter(predecessors.single_value().is_null())
        .order((eval_records::recorded_at.asc(), eval_records::id.asc()))
        .limit(65)
        .select(RecordRow::as_select())
        .load::<RecordRow>(connection)?)
}

type DashboardStates = std::collections::BTreeMap<String, Vec<crate::evals::ReportedState>>;

pub(super) fn dashboard_states(
    connection: &mut DbConnection,
    project: &str,
    subjects: &[&str],
) -> Result<Option<DashboardStates>> {
    const ROW_LIMIT: usize = 256;
    let links = eval_links::table
        .filter(eval_links::project_id.eq(project))
        .filter(eval_links::relation.eq("lifecycle_subject"))
        .filter(eval_links::target_id.eq_any(subjects))
        .select((eval_links::target_id, eval_links::source_id))
        .limit(ROW_LIMIT as i64 + 1)
        .load::<(String, String)>(connection)?;
    if links.len() > ROW_LIMIT {
        return Ok(None);
    }
    let ids: Vec<_> = links.iter().map(|(_, id)| id.as_str()).collect();
    let current = batch_current_states(connection, project, &ids)?;
    let mut states: DashboardStates = subjects
        .iter()
        .map(|id| ((*id).to_owned(), vec![]))
        .collect();
    for (subject, source) in links {
        if let Some(state) = current.get(&source) {
            states.entry(subject).or_default().push(*state);
        }
    }
    Ok(Some(states))
}

fn batch_current_states(
    connection: &mut DbConnection,
    project: &str,
    ids: &[&str],
) -> Result<std::collections::BTreeMap<String, crate::evals::ReportedState>> {
    let predecessors = eval_links::table
        .filter(eval_links::project_id.eq(project))
        .filter(eval_links::relation.eq("previous_lifecycle"))
        .filter(eval_links::target_id.eq(eval_records::id))
        .select(eval_links::target_id);
    let payloads = eval_records::table
        .filter(eval_records::project_id.eq(project))
        .filter(eval_records::id.eq_any(ids))
        .filter(eval_records::kind.eq("lifecycle"))
        .filter(predecessors.single_value().is_null())
        .select((eval_records::id, eval_records::payload_json))
        .load::<(String, String)>(connection)?;
    payloads
        .into_iter()
        .map(|(id, payload)| {
            let record: EvalRecordInput = serde_json::from_str(&payload)?;
            let EvalBody::Lifecycle(event) = record.body else {
                bail!("lifecycle projection references a non-lifecycle record");
            };
            Ok((id, event.state))
        })
        .collect()
}

fn frontier(row: RecordRow) -> Result<LifecycleFrontier> {
    let record = row.record()?;
    let EvalBody::Lifecycle(report) = record.input.body else {
        bail!("lifecycle projection references a non-lifecycle record");
    };
    Ok(LifecycleFrontier {
        id: record.input.id,
        state: report.state,
        occurred_at: record.input.occurred_at,
        recorded_at: record.recorded_at,
    })
}
