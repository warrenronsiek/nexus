// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::RecordCache;
use crate::evals::EvalRecordInput;
use crate::persistence::batch::{complete_groups, GroupBounds};
use crate::persistence::schema::eval_dimensions;
use anyhow::Result;
use diesel::prelude::*;

impl RecordCache<'_> {
    pub(super) fn prefetch_dimensions(&mut self, records: &[EvalRecordInput]) -> Result<()> {
        self.dimension_page.clear();
        let rows = eval_dimensions::table
            .filter(eval_dimensions::project_id.eq(self.project))
            .filter(eval_dimensions::record_id.eq_any(records.iter().map(|record| &record.id)))
            .order((
                eval_dimensions::record_id.asc(),
                eval_dimensions::dimension.asc(),
                eval_dimensions::value.asc(),
            ))
            .limit(1025)
            .select((
                eval_dimensions::record_id,
                eval_dimensions::dimension,
                eval_dimensions::value,
            ))
            .load::<(String, String, String)>(self.connection)?;
        self.cache_dimension_page(records, rows)
    }
    fn cache_dimension_page(
        &mut self,
        records: &[EvalRecordInput],
        rows: Vec<(String, String, String)>,
    ) -> Result<()> {
        self.dimension_page = complete_groups(
            rows.into_iter()
                .map(|(id, dimension, value)| (id, (dimension, value)))
                .collect(),
            records.iter().map(|record| record.id.clone()),
            GroupBounds {
                rows: 1025,
                bytes: 1024 * 1024,
                items: 129,
            },
        )?
        .into_iter()
        .collect();
        Ok(())
    }
    pub(super) fn dimensions(&mut self, id: &str) -> Result<Vec<(String, String)>> {
        let mut values = match self.dimension_page.get(id) {
            Some(values) => values.clone(),
            None => eval_dimensions::table
                .filter(eval_dimensions::project_id.eq(self.project))
                .filter(eval_dimensions::record_id.eq(id))
                .order((
                    eval_dimensions::dimension.asc(),
                    eval_dimensions::value.asc(),
                ))
                .limit(129)
                .select((eval_dimensions::dimension, eval_dimensions::value))
                .load::<(String, String)>(self.connection)?,
        };
        if values.len() > 128 {
            values.truncate(128);
            self.truncated = true;
        }
        Ok(values)
    }
}
