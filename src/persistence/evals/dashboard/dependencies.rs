// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::{grouping, RecordCache};
use crate::evals::{EvalBody, EvalRecordInput, ModelAssignment};
use crate::persistence::schema::eval_records;
use anyhow::Result;
use diesel::prelude::*;
use std::collections::BTreeSet;

impl RecordCache<'_> {
    /// Batch the run/definition/model graph without changing bounded admission order in get.
    pub(super) fn prefetch_dependencies(&mut self, records: &[EvalRecordInput]) -> Result<()> {
        self.payload_page.clear();
        let mut frontier = records.to_vec();
        let mut requested = BTreeSet::new();
        let mut bytes = 0;
        for _ in 0..3 {
            let ids: Vec<_> = frontier
                .iter()
                .flat_map(|record| dependencies(&record.body))
                .filter(|id| !self.records.contains_key(id) && !requested.contains(id))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .take(256)
                .collect();
            if ids.is_empty() {
                break;
            }
            requested.extend(ids.iter().cloned());
            let rows = eval_records::table
                .filter(eval_records::project_id.eq(self.project))
                .filter(eval_records::id.eq_any(ids))
                .order(eval_records::id.asc())
                .limit(256)
                .select((eval_records::id, eval_records::payload_json))
                .load::<(String, String)>(self.connection)?;
            frontier.clear();
            for (id, payload) in rows {
                bytes += id.len() + payload.len();
                if bytes <= 4 * 1024 * 1024 {
                    frontier.push(serde_json::from_str(&payload)?);
                    self.payload_page.insert(id, payload);
                }
            }
        }
        Ok(())
    }
}

fn dependencies(body: &EvalBody) -> Vec<String> {
    let mut ids: Vec<_> = grouping::execution_parent(body)
        .map(str::to_owned)
        .into_iter()
        .collect();
    match body {
        EvalBody::Run(run) => ids.extend([run.suite_id.clone(), run.target_id.clone()]),
        EvalBody::Measurement(value) => ids.push(value.metric_id.clone()),
        EvalBody::Grade(value) => ids.push(value.grader_id.clone()),
        EvalBody::Target(target) => {
            for component in &target.components {
                ids.extend(component.repository_id.clone());
                if let ModelAssignment::Fixed { model_id } = &component.model {
                    ids.push(model_id.clone());
                }
            }
        }
        _ => {}
    }
    ids
}
