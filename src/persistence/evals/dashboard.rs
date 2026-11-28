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
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
const SOURCE_LIMIT: usize = 4096;
const SOURCE_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy)]
enum Selection<'a> {
    Group(Option<&'a str>),
    Agent(&'a str),
}
impl Selection<'_> {
    fn includes(self, group: &EvalDashboardGroup) -> bool {
        match self {
            Self::Group(id) => id == Some(group.id.as_str()),
            Self::Agent(agent) => group.agent == agent || group.id == agent,
        }
    }
}
#[cfg(test)]
mod prefetch_tests;
#[cfg(all(test, feature = "postgres"))]
mod tests;
impl Store {
    pub fn eval_dashboard(&mut self, project: &str, group: Option<&str>) -> Result<EvalDashboard> {
        self.eval_dashboard_window(project, group, None)
    }

    pub fn eval_dashboard_window(
        &mut self,
        project: &str,
        group: Option<&str>,
        occurred_after: Option<DateTime<Utc>>,
    ) -> Result<EvalDashboard> {
        ensure!(
            group.is_none_or(|id| id.len() <= 512),
            "eval dashboard identity exceeds limit"
        );
        project_dashboard(
            &mut self.connection,
            project,
            Selection::Group(group),
            occurred_after,
        )
    }
    pub fn eval_agent_dashboard_window(
        &mut self,
        project: &str,
        agent: &str,
        occurred_after: Option<DateTime<Utc>>,
    ) -> Result<EvalDashboard> {
        ensure!(
            !agent.is_empty() && agent.len() <= 512,
            "eval agent identity is empty or oversized"
        );
        project_dashboard(
            &mut self.connection,
            project,
            Selection::Agent(agent),
            occurred_after,
        )
    }
}
fn project_dashboard(
    connection: &mut DbConnection,
    project: &str,
    selected: Selection<'_>,
    occurred_after: Option<DateTime<Utc>>,
) -> Result<EvalDashboard> {
    let (records, truncated) = source_records(connection, project, occurred_after)?;
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
    projection.project_run_states(&mut cache)?;
    projection.finish(scanned_records, truncated || cache.truncated)
}
fn source_records(
    connection: &mut DbConnection,
    project: &str,
    occurred_after: Option<DateTime<Utc>>,
) -> Result<(Vec<EvalRecordInput>, bool)> {
    let mut records = vec![];
    let mut after: Option<(String, String)> = None;
    let mut bytes = 0;
    let cutoff = occurred_after.map(|time| time.to_rfc3339());
    loop {
        let rows = source_page(connection, project, cutoff.as_deref(), after.as_ref())?;
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
type SourceRow = (String, String, String);

fn source_page(
    connection: &mut DbConnection,
    project: &str,
    cutoff: Option<&str>,
    after: Option<&(String, String)>,
) -> Result<Vec<SourceRow>> {
    let mut query = eval_records::table
        .filter(eval_records::project_id.eq(project))
        .filter(eval_records::kind.eq_any(["run", "grade", "measurement"]))
        .into_boxed::<crate::persistence::backend::DbBackend>();
    let query = match cutoff {
        Some(cutoff) => {
            query = query.filter(eval_records::occurred_at.ge(cutoff));
            if let Some((time, id)) = after {
                query = query.filter(
                    eval_records::occurred_at
                        .lt(time)
                        .or(eval_records::occurred_at
                            .eq(time)
                            .and(eval_records::id.lt(id))),
                );
            }
            query
                .order((eval_records::occurred_at.desc(), eval_records::id.desc()))
                .select((
                    eval_records::id,
                    eval_records::occurred_at,
                    eval_records::payload_json,
                ))
        }
        None => {
            if let Some((time, id)) = after {
                query = query.filter(
                    eval_records::recorded_at
                        .lt(time)
                        .or(eval_records::recorded_at
                            .eq(time)
                            .and(eval_records::id.gt(id))),
                );
            }
            query
                .order((eval_records::recorded_at.desc(), eval_records::id.asc()))
                .select((
                    eval_records::id,
                    eval_records::recorded_at,
                    eval_records::payload_json,
                ))
        }
    };
    Ok(query.limit(256).load(connection)?)
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
