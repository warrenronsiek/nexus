// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::{grouping, RecordCache};
use crate::evals::{EvalBody, EvalRecordInput};
use crate::persistence::schema::eval_records;
use anyhow::Result;
use diesel::prelude::*;
use std::collections::BTreeSet;

impl RecordCache<'_> {
    /// Prefetch is an IO buffer; dependency admission still occurs in the original get order.
    pub(super) fn prefetch_dependencies(&mut self, records: &[EvalRecordInput]) -> Result<()> {
        self.payload_page.clear();
        let ids: BTreeSet<_> = records
            .iter()
            .flat_map(|record| match &record.body {
                EvalBody::Run(run) => vec![run.suite_id.as_str(), run.target_id.as_str()],
                body => grouping::execution_parent(body).into_iter().collect(),
            })
            .collect();
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| !self.records.contains_key(*id))
            .take(64)
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let rows = eval_records::table
            .filter(eval_records::project_id.eq(self.project))
            .filter(eval_records::id.eq_any(ids))
            .order(eval_records::id.asc())
            .limit(64)
            .select((eval_records::id, eval_records::payload_json))
            .load::<(String, String)>(self.connection)?;
        let mut bytes = 0;
        for (id, payload) in rows {
            bytes += id.len() + payload.len();
            if bytes <= 4 * 1024 * 1024 {
                self.payload_page.insert(id, payload);
            }
        }
        Ok(())
    }
}
