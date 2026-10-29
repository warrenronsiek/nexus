// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
// @boundary chart-numbers: exact metric text is retained; only finite safely roundtripping scalars receive optional presentation numbers.
mod dependencies;
mod dimensions;
mod grouping;
mod series;
use crate::evals::*;
use crate::persistence::{backend::DbConnection, schema::eval_records, Store};
use anyhow::{ensure, Result};
use diesel::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
const SOURCE_LIMIT: usize = 4096;
const SOURCE_BYTES: usize = 8 * 1024 * 1024;
#[cfg(all(test, feature = "postgres"))]
mod tests;
impl Store {
    pub fn eval_dashboard(&mut self, project: &str, group: Option<&str>) -> Result<EvalDashboard> {
        ensure!(
            group.is_none_or(|id| id.len() <= 512),
            "eval dashboard identity exceeds limit"
        );
        project_dashboard(&mut self.connection, project, group)
    }
}
fn project_dashboard(
    connection: &mut DbConnection,
    project: &str,
    selected: Option<&str>,
) -> Result<EvalDashboard> {
    let (records, truncated) = source_records(connection, project)?;
    let scanned_records = records.len();
    let superseded: BTreeSet<_> = records
        .iter()
        .filter_map(|record| match &record.body {
            EvalBody::Measurement(value) => value.replaces.clone(),
            EvalBody::Grade(value) => value.replaces.clone(),
            _ => None,
        })
        .collect();
    let mut cache = RecordCache {
        connection,
        project,
        records: BTreeMap::new(),
        bytes: 0,
        truncated: false,
        dimension_page: BTreeMap::new(),
        payload_page: BTreeMap::new(),
    };
    let mut projection = grouping::DashboardProjection::new(selected);
    for page in records.chunks(64) {
        cache.prefetch_dependencies(page)?;
        cache.prefetch_dimensions(page)?;
        for record in page {
            projection.observe(&mut cache, record, &superseded)?;
        }
    }
    projection.finish(scanned_records, truncated || cache.truncated)
}
fn source_records(
    connection: &mut DbConnection,
    project: &str,
) -> Result<(Vec<EvalRecordInput>, bool)> {
    let mut records = vec![];
    let mut after: Option<(String, String)> = None;
    let mut bytes = 0;
    loop {
        let mut query = eval_records::table
            .filter(eval_records::project_id.eq(project))
            .filter(eval_records::kind.eq_any(["run", "grade", "measurement"]))
            .into_boxed::<crate::persistence::backend::DbBackend>();
        if let Some((time, id)) = &after {
            query = query.filter(eval_records::recorded_at.le(time)).filter(
                eval_records::recorded_at
                    .lt(time)
                    .or(eval_records::id.gt(id)),
            );
        }
        let rows = query
            .order((eval_records::recorded_at.desc(), eval_records::id.asc()))
            .limit(256)
            .select((
                eval_records::id,
                eval_records::recorded_at,
                eval_records::payload_json,
            ))
            .load::<(String, String, String)>(connection)?;
        if rows.is_empty() {
            return Ok((records, false));
        }
        for (id, time, payload) in rows {
            if records.len() == SOURCE_LIMIT || bytes + payload.len() > SOURCE_BYTES {
                return Ok((records, true));
            }
            bytes += payload.len();
            after = Some((time, id));
            records.push(serde_json::from_str(&payload)?);
        }
    }
}
struct RecordCache<'a> {
    connection: &'a mut DbConnection,
    project: &'a str,
    records: BTreeMap<String, Option<EvalRecordInput>>,
    bytes: usize,
    truncated: bool,
    dimension_page: BTreeMap<String, Vec<(String, String)>>,
    payload_page: BTreeMap<String, String>,
}
impl RecordCache<'_> {
    fn get(&mut self, id: &str) -> Result<Option<EvalRecordInput>> {
        if let Some(record) = self.records.get(id) {
            return Ok(record.clone());
        }
        if self.records.len() >= SOURCE_LIMIT || self.bytes >= SOURCE_BYTES {
            self.truncated = true;
            return Ok(None);
        }
        let payload = match self.payload_page.get(id) {
            Some(payload) => Some(payload.clone()),
            None => eval_records::table
                .find((self.project, id))
                .select(eval_records::payload_json)
                .first::<String>(self.connection)
                .optional()?,
        };
        if payload
            .as_ref()
            .is_some_and(|payload| self.bytes + payload.len() > SOURCE_BYTES)
        {
            self.truncated = true;
            return Ok(None);
        }
        let record = payload
            .map(|payload| {
                self.bytes += payload.len();
                serde_json::from_str(&payload)
            })
            .transpose()?;
        self.records.insert(id.into(), record.clone());
        Ok(record)
    }
}
