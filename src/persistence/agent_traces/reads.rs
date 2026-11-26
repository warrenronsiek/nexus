// @feature agent-traces
// @feature persistence
// @spec docs/features/agent-traces.md
// @spec docs/features/persistence.md
// @boundary trace-read-json: dynamic content is assembled only for explicitly requested raw trace detail.
use super::rows::{EventRow, InvocationRow};
use crate::agent_traces::{canonical_time, TraceFilter, TraceOutcome, TraceQuery};
use crate::persistence::backend::{DbBackend, DbConnection};
use crate::persistence::schema::{
    trace_batches as batch, trace_events as event, trace_invocations as call,
};
use anyhow::{ensure, Context, Result};
use diesel::dsl::{case_when, count, count_star, max, min};
use diesel::prelude::*;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

const MAX_EVENT_PAGE_BYTES: usize = 768 * 1024;
const SCOPE_LIMIT: usize = 128;

pub(super) fn read(
    connection: &mut DbConnection,
    project: &str,
    query: TraceQuery,
) -> Result<Value> {
    match query {
        TraceQuery::List { filter } => list(connection, project, filter),
        TraceQuery::Calls { filter } => calls(connection, project, filter),
        TraceQuery::Usage { filter } => usage(connection, project, filter),
        TraceQuery::Get {
            application_id,
            environment,
            trace_id,
            event_id,
            include_payload,
            limit,
            cursor,
        } => {
            let page = Page::parse(limit, cursor.as_deref())?;
            let reference = EventSelection {
                project,
                application: &application_id,
                environment: &environment,
                trace: &trace_id,
                event: event_id.as_deref(),
            };
            let selection = if include_payload {
                TraceContent::WithPayload
            } else {
                TraceContent::Metadata
            };
            get(connection, reference, selection, page)
        }
        TraceQuery::Status {
            application_id,
            environment,
        } => status(
            connection,
            project,
            application_id.as_deref(),
            environment.as_deref(),
        ),
    }
}

struct Page {
    limit: usize,
    offset: i64,
}

impl Page {
    fn parse(limit: Option<usize>, cursor: Option<&str>) -> Result<Self> {
        let offset = cursor
            .map(|value| {
                value
                    .strip_prefix("offset:")
                    .context("invalid trace cursor")?
                    .parse::<i64>()
                    .context("invalid trace cursor")
            })
            .transpose()?
            .unwrap_or(0);
        ensure!(offset >= 0, "invalid trace cursor");
        Ok(Self {
            limit: limit.unwrap_or(32),
            offset,
        })
    }

    fn finish(&self, key: &str, rows: &[impl Serialize], truncated: bool) -> Result<Value> {
        let next_cursor = truncated.then(|| format!("offset:{}", self.offset + rows.len() as i64));
        let mut response = json!({"ok": true, "next_cursor": next_cursor, "truncated": truncated});
        response[key] = serde_json::to_value(rows)?;
        Ok(response)
    }

    fn records(&self, key: &str, mut rows: Vec<impl Serialize>) -> Result<Value> {
        let truncated = rows.len() > self.limit;
        rows.truncate(self.limit);
        self.finish(key, &rows, truncated)
    }

    fn query_limit(&self) -> i64 {
        self.limit as i64 + 1
    }
}

// Both grouped and ordinary queries apply the same application, agent and time filters.
macro_rules! filter_scope {
    ($query:expr, $table:ident, $filter:expr) => {{
        let mut query = $query;
        let filter = $filter;
        if let Some(value) = &filter.application_id {
            query = query.filter($table::application_id.eq(value));
        }
        if let Some(value) = &filter.environment {
            query = query.filter($table::environment.eq(value));
        }
        if let Some(value) = &filter.agent_id {
            query = query.filter($table::agent_id.eq(value));
        }
        if let Some(value) = &filter.since {
            query = query.filter($table::occurred_at.ge(canonical_time(value)?));
        }
        if let Some(value) = &filter.until {
            query = query.filter($table::occurred_at.lt(canonical_time(value)?));
        }
        query
    }};
}

#[derive(Queryable, Serialize)]
struct TraceSummary {
    application_id: String,
    environment: String,
    trace_id: String,
    occurred_at: Option<String>,
    agent_id: Option<String>,
    conversation_id: Option<String>,
}

fn list(connection: &mut DbConnection, project: &str, filter: TraceFilter) -> Result<Value> {
    let page = Page::parse(filter.limit, filter.cursor.as_deref())?;
    ensure!(
        filter.kind.is_none()
            && filter.name.is_none()
            && filter.version.is_none()
            && !filter.unversioned
            && filter.outcome.is_none(),
        "filter by capability on the calls or usage action"
    );
    let query = event::table
        .filter(event::project_id.eq(project))
        .group_by((event::application_id, event::environment, event::trace_id))
        .select((
            event::application_id,
            event::environment,
            event::trace_id,
            min(event::occurred_at),
            case_when(
                min(event::agent_id).eq(max(event::agent_id)),
                min(event::agent_id),
            )
            .otherwise(None::<String>),
            min(event::conversation_id),
        ))
        .into_boxed::<DbBackend>();
    let rows = filter_scope!(query, event, &filter)
        .order((
            min(event::occurred_at).desc(),
            event::application_id.asc(),
            event::environment.asc(),
            event::trace_id.asc(),
        ))
        .offset(page.offset)
        .limit(page.query_limit())
        .load::<TraceSummary>(connection)?;
    page.records("traces", rows)
}

struct EventSelection<'a> {
    project: &'a str,
    application: &'a str,
    environment: &'a str,
    trace: &'a str,
    event: Option<&'a str>,
}

enum TraceContent {
    Metadata,
    WithPayload,
}

#[derive(Queryable)]
struct EventHead {
    id: String,
    event_json: String,
    payload_bytes: i64,
    payload_expired: bool,
}

fn get(
    connection: &mut DbConnection,
    selection: EventSelection<'_>,
    content: TraceContent,
    page: Page,
) -> Result<Value> {
    let heads = event_heads(connection, &selection, &page)?;
    let kept = event_page_len(&heads, &selection, &content, page.limit);
    let payloads = match content {
        TraceContent::Metadata => HashMap::new(),
        TraceContent::WithPayload => load_payloads(connection, &heads[..kept])?,
    };
    let views = heads[..kept]
        .iter()
        .map(|head| event_view(head, &selection, &content, &payloads))
        .collect::<Result<Vec<_>>>()?;
    page.finish("events", &views, kept < heads.len())
}

fn event_heads(
    connection: &mut DbConnection,
    selection: &EventSelection<'_>,
    page: &Page,
) -> Result<Vec<EventHead>> {
    let mut query = event::table
        .filter(event::project_id.eq(selection.project))
        .filter(event::application_id.eq(selection.application))
        .filter(event::environment.eq(selection.environment))
        .filter(event::trace_id.eq(selection.trace))
        .select((
            event::id,
            event::event_json,
            event::payload_bytes,
            event::payload_expired,
        ))
        .into_boxed::<DbBackend>();
    if let Some(value) = selection.event {
        query = query.filter(event::id.eq(EventRow::key(
            selection.project,
            selection.application,
            selection.environment,
            value,
        )?));
    }
    Ok(query
        .order((
            event::source_sequence.is_null().asc(),
            event::source_sequence.asc(),
            event::occurred_at.asc(),
            event::event_id.asc(),
        ))
        .offset(page.offset)
        .limit(page.query_limit())
        .load(connection)?)
}

fn event_page_len(
    heads: &[EventHead],
    selection: &EventSelection<'_>,
    content: &TraceContent,
    limit: usize,
) -> usize {
    let envelope_bytes = 2 * (selection.application.len() + selection.environment.len()) + 96;
    let mut bytes = 128;
    heads
        .iter()
        .take(limit)
        .take_while(|head| {
            let payload_bytes = match content {
                TraceContent::Metadata => 0,
                TraceContent::WithPayload => head.payload_bytes as usize,
            };
            bytes += head.event_json.len() + payload_bytes + envelope_bytes;
            bytes <= MAX_EVENT_PAGE_BYTES
        })
        .count()
}

fn load_payloads(
    connection: &mut DbConnection,
    heads: &[EventHead],
) -> Result<HashMap<String, String>> {
    Ok(event::table
        .filter(
            event::id.eq_any(
                heads
                    .iter()
                    .filter(|head| head.payload_bytes > 0)
                    .map(|head| &head.id),
            ),
        )
        .select((event::id, event::payload_json))
        .load::<(String, Option<String>)>(connection)?
        .into_iter()
        .filter_map(|(id, payload)| payload.map(|payload| (id, payload)))
        .collect())
}

fn event_view(
    head: &EventHead,
    selection: &EventSelection<'_>,
    content: &TraceContent,
    payloads: &HashMap<String, String>,
) -> Result<Value> {
    let mut view: Value = serde_json::from_str(&head.event_json)?;
    view["application_id"] = json!(selection.application);
    view["environment"] = json!(selection.environment);
    let payload = payloads.get(&head.id);
    let expired_during_read =
        matches!(content, TraceContent::WithPayload) && payload.is_none() && head.payload_bytes > 0;
    let expired = head.payload_expired || expired_during_read;
    view["has_payload"] = json!(!expired && head.payload_bytes > 0);
    view["payload_state"] = json!(match (expired, head.payload_bytes > 0) {
        (true, _) => "expired",
        (false, true) => "available",
        _ => "not_captured",
    });
    if let Some(value) = payload {
        view["payload"] = serde_json::from_str(value)?;
    }
    Ok(view)
}

// Both the invocation list and grouped usage query use these identical predicates.
macro_rules! filter_calls {
    ($query:expr, $filter:expr) => {{
        let filter = $filter;
        let mut query = filter_scope!($query, call, filter);
        if let Some(value) = &filter.name {
            query = query.filter(call::name.eq(value));
        }
        if let Some(value) = &filter.version {
            query = query.filter(call::version.eq(value));
        } else if filter.unversioned {
            query = query.filter(call::version.is_null());
        }
        if let Some(value) = filter.kind {
            query = query.filter(call::kind.eq(value.as_str()));
        }
        if let Some(value) = filter.outcome {
            query = query.filter(call::outcome.eq(value.as_str()));
        }
        query
    }};
}

fn calls(connection: &mut DbConnection, project: &str, filter: TraceFilter) -> Result<Value> {
    let page = Page::parse(filter.limit, filter.cursor.as_deref())?;
    let query = call::table
        .filter(call::project_id.eq(project))
        .into_boxed::<DbBackend>();
    let rows = filter_calls!(query, &filter)
        .order((call::occurred_at.desc(), call::id.asc()))
        .limit(page.query_limit())
        .offset(page.offset)
        .select(InvocationRow::as_select())
        .load::<InvocationRow>(connection)?;
    page.records("calls", rows)
}

#[derive(Queryable, Serialize)]
struct Usage {
    application_id: String,
    environment: String,
    agent_id: String,
    kind: String,
    name: Option<String>,
    version: Option<String>,
    calls: i64,
    succeeded: i64,
    failed: i64,
    incomplete: i64,
    canceled: i64,
    unknown: i64,
}

fn usage(connection: &mut DbConnection, project: &str, filter: TraceFilter) -> Result<Value> {
    let page = Page::parse(filter.limit, filter.cursor.as_deref())?;
    let rows = usage_rows(connection, project, &filter, &page)?;
    page.records("usage", rows)
}

fn usage_rows(
    connection: &mut DbConnection,
    project: &str,
    filter: &TraceFilter,
    page: &Page,
) -> Result<Vec<Usage>> {
    let grouping = (
        call::application_id,
        call::environment,
        call::agent_id,
        call::kind,
        call::name,
        call::version,
    );
    let unsuccessful = [
        TraceOutcome::Failed,
        TraceOutcome::ValidationFailed,
        TraceOutcome::TimedOut,
        TraceOutcome::Rejected,
    ];
    let query = call::table
        .filter(call::project_id.eq(project))
        .group_by(grouping)
        .select((
            call::application_id,
            call::environment,
            call::agent_id,
            call::kind,
            call::name,
            call::version,
            count_star(),
            count(case_when(
                call::outcome.eq(TraceOutcome::Succeeded.as_str()),
                call::id,
            )),
            count(case_when(
                call::outcome.eq_any(unsuccessful.map(TraceOutcome::as_str)),
                call::id,
            )),
            count(case_when(call::finished_event_id.is_null(), call::id)),
            count(case_when(
                call::outcome.eq(TraceOutcome::Canceled.as_str()),
                call::id,
            )),
            count(case_when(
                call::finished_event_id.is_not_null().and(
                    call::outcome
                        .is_null()
                        .or(call::outcome.eq(TraceOutcome::Unknown.as_str())),
                ),
                call::id,
            )),
        ))
        .into_boxed::<DbBackend>();
    Ok(filter_calls!(query, filter)
        .order((count_star().desc(), grouping))
        .limit(page.query_limit())
        .offset(page.offset)
        .load::<Usage>(connection)?)
}

#[derive(Queryable, Serialize)]
struct ApplicationScope {
    application_id: String,
    environment: String,
    agent_id: String,
}

fn status(
    connection: &mut DbConnection,
    project: &str,
    application: Option<&str>,
    environment: Option<&str>,
) -> Result<Value> {
    let mut pending = batch::table
        .filter(batch::project_id.eq(project))
        .filter(batch::projected.eq(false))
        .select((count_star(), min(batch::received_at)))
        .into_boxed::<DbBackend>();
    let mut scopes = event::table
        .filter(event::project_id.eq(project))
        .select((event::application_id, event::environment, event::agent_id))
        .distinct()
        .into_boxed::<DbBackend>();
    if let Some(value) = application {
        pending = pending.filter(batch::application_id.eq(value));
        scopes = scopes.filter(event::application_id.eq(value));
    }
    if let Some(value) = environment {
        pending = pending.filter(batch::environment.eq(value));
        scopes = scopes.filter(event::environment.eq(value));
    }
    let (pending_batches, oldest_pending_at) =
        pending.first::<(i64, Option<String>)>(connection)?;
    let mut scopes = scopes
        .order((
            event::application_id.asc(),
            event::environment.asc(),
            event::agent_id.asc(),
        ))
        .limit(SCOPE_LIMIT as i64 + 1)
        .load::<ApplicationScope>(connection)?;
    let scopes_truncated = scopes.len() > SCOPE_LIMIT;
    scopes.truncate(SCOPE_LIMIT);
    Ok(json!({
        "ok": true,
        "pending_batches": pending_batches,
        "oldest_pending_at": oldest_pending_at,
        "scopes": scopes,
        "scopes_truncated": scopes_truncated,
    }))
}
