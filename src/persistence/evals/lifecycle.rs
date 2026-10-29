// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::rows::RecordRow;
use crate::evals::{EvalBody, EvalRecordInput, LifecycleFrontier, LifecycleProjection};
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{eval_links, eval_records};
use anyhow::{bail, Result};
use diesel::dsl::{exists, not};
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
    let subjects = eval_links::table
        .filter(eval_links::project_id.eq(project))
        .filter(eval_links::relation.eq("lifecycle_subject"))
        .filter(eval_links::target_id.eq(&input.id))
        .select(eval_links::source_id);
    let predecessors = eval_links::table
        .filter(eval_links::project_id.eq(project))
        .filter(eval_links::relation.eq("previous_lifecycle"))
        .filter(eval_links::target_id.eq(eval_records::id));
    let mut rows = eval_records::table
        .filter(eval_records::project_id.eq(project))
        .filter(eval_records::id.eq_any(subjects))
        .filter(not(exists(predecessors)))
        .order((eval_records::recorded_at.asc(), eval_records::id.asc()))
        .limit(65)
        .select(RecordRow::as_select())
        .load::<RecordRow>(connection)?;
    let truncated = rows.len() > 64;
    rows.truncate(64);
    let frontiers = rows.into_iter().map(frontier).collect::<Result<_>>()?;
    Ok(Some(LifecycleProjection {
        initial_state,
        frontiers,
        truncated,
    }))
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
