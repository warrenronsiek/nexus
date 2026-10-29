// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::RecordCache;
use crate::evals::EvalRecordInput;
use crate::persistence::schema::eval_dimensions;
use anyhow::Result;
use diesel::prelude::*;
use std::collections::BTreeMap;

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
        let incomplete = (rows.len() == 1025).then(|| rows.last().unwrap().0.clone());
        let mut groups: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for (id, dimension, value) in rows {
            groups.entry(id).or_default().push((dimension, value));
        }
        if incomplete.is_none() {
            for record in records {
                groups.entry(record.id.clone()).or_default();
            }
        }
        let mut bytes = 0;
        for (id, mut values) in groups {
            if incomplete.as_ref() == Some(&id) {
                continue;
            }
            values.truncate(129);
            bytes += serde_json::to_vec(&(&id, &values))?.len();
            if bytes <= 1024 * 1024 {
                self.dimension_page.insert(id, values);
            }
        }
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
