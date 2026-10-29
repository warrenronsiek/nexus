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
