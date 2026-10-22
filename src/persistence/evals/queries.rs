// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
// @entrypoint Store::list_eval_records
// @entrypoint Store::why_eval_record
use super::rows::RecordRow;
use crate::evals::*;
use crate::persistence::backend::DbBackend;
use crate::persistence::schema::{eval_dimensions, eval_links, eval_records};
use crate::persistence::Store;
use anyhow::{bail, Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: String,
    cutoff: DateTime<Utc>,
}

impl Cursor {
    fn decode(value: &str) -> Result<Self> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(value)
            .context("invalid eval cursor")?;
        serde_json::from_slice(&bytes).context("invalid eval cursor")
    }
    fn encode(&self) -> Result<String> {
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?))
    }
}

impl Store {
    pub fn get_eval_record(&mut self, project: &str, id: &str) -> Result<Option<EvalRecord>> {
        let mut record = super::find(&mut self.connection, project, id)?
            .map(|r| r.record())
            .transpose()?;
        if let Some(record) = &mut record {
            record.lifecycle =
                super::lifecycle::projection(&mut self.connection, project, &record.input)?;
        }
        Ok(record)
    }

    pub fn list_eval_records(&mut self, project: &str, query: &EvalQuery) -> Result<EvalPage> {
        let page = PageSelection::new(query)?;
        let mut rows = selection(project, query, &page)
            .order(eval_records::id.asc())
            .limit((page.limit + 1) as i64)
            .select(RecordRow::as_select())
            .load::<RecordRow>(&mut self.connection)?;
        let next_cursor = page.next_cursor(&mut rows)?;
        let records = rows
            .into_iter()
            .map(|row| header(&mut self.connection, project, row))
            .collect::<Result<_>>()?;
        Ok(EvalPage {
            records,
            next_cursor,
        })
    }

    pub fn why_eval_record(&mut self, project: &str, id: &str, limit: u32) -> Result<EvalHistory> {
        let limit = limit.clamp(1, 64) as usize;
        let mut queue = VecDeque::from([id.to_owned()]);
        let mut seen = HashSet::new();
        let mut records = Vec::new();
        let mut bytes = 0;
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let record = self
                .get_eval_record(project, &id)?
                .with_context(|| format!("unknown eval record: {id}"))?;
            bytes += serde_json::to_vec(&record)?.len();
            if records.len() >= limit || bytes > 768 * 1024 {
                return Ok(EvalHistory {
                    records,
                    truncated: true,
                });
            }
            for link in &record.links {
                if !seen.contains(&link.target_id) {
                    queue.push_back(link.target_id.clone());
                }
            }
            queue.extend(corrections(&mut self.connection, project, &id)?);
            records.push(record);
        }
        Ok(EvalHistory {
            records,
            truncated: false,
        })
    }
}

fn corrections(
    connection: &mut crate::persistence::backend::DbConnection,
    project: &str,
    id: &str,
) -> Result<Vec<String>> {
    Ok(eval_links::table
        .filter(eval_links::project_id.eq(project))
        .filter(eval_links::target_id.eq(id))
        .filter(eval_links::relation.eq_any([
            "supersedes",
            "regrades",
            "verified_artifact",
            "lifecycle_subject",
            "measured_subject",
            "remeasures",
        ]))
        .order(eval_links::source_id.asc())
        .select(eval_links::source_id)
        .distinct()
        .limit(65)
        .load(connection)?)
}

struct PageSelection {
    limit: usize,
    cutoff: DateTime<Utc>,
    after: Option<String>,
}
impl PageSelection {
    fn new(query: &EvalQuery) -> Result<Self> {
        for value in [&query.repository, &query.model, &query.search, &query.after]
            .into_iter()
            .flatten()
        {
            if value.len() > 1024 {
                bail!("eval query filter exceeds 1024 bytes");
            }
        }
        let cursor = query.after.as_deref().map(Cursor::decode).transpose()?;
        Ok(Self {
            limit: query.limit.unwrap_or(32).clamp(1, 64) as usize,
            cutoff: cursor.as_ref().map_or_else(Utc::now, |c| c.cutoff),
            after: cursor.map(|c| c.after),
        })
    }
    fn next_cursor(&self, rows: &mut Vec<RecordRow>) -> Result<Option<String>> {
        if rows.len() <= self.limit {
            return Ok(None);
        }
        rows.truncate(self.limit);
        rows.last()
            .map(|r| {
                Cursor {
                    after: r.id.clone(),
                    cutoff: self.cutoff,
                }
                .encode()
            })
            .transpose()
    }
}

fn selection<'a>(
    project: &'a str,
    query: &'a EvalQuery,
    page: &'a PageSelection,
) -> eval_records::BoxedQuery<'a, DbBackend> {
    let mut rows = eval_records::table
        .filter(eval_records::project_id.eq(project))
        .filter(eval_records::recorded_at.le(page.cutoff.to_rfc3339()))
        .into_boxed::<DbBackend>();
    if let Some(kind) = query.kind {
        rows = rows.filter(eval_records::kind.eq(kind.as_str()));
    }
    if let Some(after) = &page.after {
        rows = rows.filter(eval_records::id.gt(after));
    }
    if let Some(search) = &query.search {
        let escaped = search
            .to_lowercase()
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_");
        rows = rows.filter(
            eval_records::search_text
                .like(format!("%{escaped}%"))
                .escape('!'),
        );
    }
    rows = with_dimension(rows, project, "repository", query.repository.as_deref());
    with_dimension(rows, project, "model", query.model.as_deref())
}

fn with_dimension<'a>(
    rows: eval_records::BoxedQuery<'a, DbBackend>,
    project: &'a str,
    dimension: &'a str,
    value: Option<&'a str>,
) -> eval_records::BoxedQuery<'a, DbBackend> {
    match value {
        Some(value) => rows.filter(
            eval_records::id.eq_any(
                eval_dimensions::table
                    .filter(eval_dimensions::project_id.eq(project))
                    .filter(eval_dimensions::dimension.eq(dimension))
                    .filter(eval_dimensions::value.eq(value))
                    .select(eval_dimensions::record_id),
            ),
        ),
        None => rows,
    }
}

fn header(
    connection: &mut crate::persistence::backend::DbConnection,
    project: &str,
    row: RecordRow,
) -> Result<EvalHeader> {
    let dimensions = eval_dimensions::table
        .filter(eval_dimensions::project_id.eq(project))
        .filter(eval_dimensions::record_id.eq(&row.id))
        .order((
            eval_dimensions::dimension.asc(),
            eval_dimensions::value.asc(),
        ))
        .select((eval_dimensions::dimension, eval_dimensions::value))
        .limit(128)
        .load::<(String, String)>(connection)?;
    let body: EvalRecordInput = serde_json::from_str(&row.payload_json)?;
    let value = |name: &str| {
        dimensions
            .iter()
            .find(|(d, _)| d == name)
            .map(|(_, v)| v.clone())
    };
    Ok(EvalHeader {
        id: row.id,
        kind: body.body.kind(),
        title: row.title,
        occurred_at: row.occurred_at.parse()?,
        recorded_at: row.recorded_at.parse()?,
        repository: value("repository"),
        model: value("model"),
    })
}
