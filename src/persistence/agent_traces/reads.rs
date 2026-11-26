// @feature agent-traces
// @feature persistence
// @spec docs/features/agent-traces.md
// @spec docs/features/persistence.md
mod events;

use super::rows::InvocationRow;
use crate::agent_traces::{
    canonical_time, TraceApplicationScope, TraceFilter, TraceOutcome, TracePage, TraceQuery,
    TraceReadResponse, TraceSummary, TraceTimestamp, TraceUsage, TraceVersionSelector,
};
use crate::persistence::backend::{DbBackend, DbConnection};
use crate::persistence::models::parse_time;
use crate::persistence::schema::{
    trace_batches as batch, trace_events as event, trace_invocations as call,
};
use anyhow::{ensure, Context, Result};
use diesel::dsl::{case_when, count, count_star, max, min};
use diesel::prelude::*;

const DEFAULT_PAGE_ITEMS: usize = 32;
const SCOPE_LIMIT: usize = 128;

pub(super) fn read(
    connection: &mut DbConnection,
    project: &str,
    query: TraceQuery,
) -> Result<TraceReadResponse> {
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
            let reference = events::Selection {
                project,
                application: &application_id,
                environment: &environment,
                trace: &trace_id,
                event: event_id.as_deref(),
            };
            let content = if include_payload {
                events::Content::WithPayload
            } else {
                events::Content::Metadata
            };
            events::read(connection, reference, content, limit, cursor.as_deref())
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
            limit: limit.unwrap_or(DEFAULT_PAGE_ITEMS),
            offset,
        })
    }

    fn records<T>(&self, mut rows: Vec<T>) -> (Vec<T>, TracePage) {
        let truncated = rows.len() > self.limit;
        rows.truncate(self.limit);
        let next_cursor = truncated.then(|| format!("offset:{}", self.offset + rows.len() as i64));
        (rows, TracePage::new(next_cursor))
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

#[derive(Queryable)]
struct TraceSummaryRow {
    application_id: String,
    environment: String,
    trace_id: String,
    occurred_at: Option<String>,
    agent_id: Option<String>,
    conversation_id: Option<String>,
}

impl TraceSummaryRow {
    fn decode(self) -> Result<TraceSummary> {
        let time = self.occurred_at.context("missing trace summary time")?;
        Ok(TraceSummary {
            application_id: self.application_id,
            environment: self.environment,
            trace_id: self.trace_id,
            occurred_at: parse_time(&time)?.into(),
            agent_id: self.agent_id,
            conversation_id: self.conversation_id,
        })
    }
}

fn list(
    connection: &mut DbConnection,
    project: &str,
    filter: TraceFilter,
) -> Result<TraceReadResponse> {
    let page = Page::parse(filter.limit, filter.cursor.as_deref())?;
    ensure!(
        filter.kind.is_none()
            && filter.name.is_none()
            && matches!(filter.version, TraceVersionSelector::Any {})
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
        .load::<TraceSummaryRow>(connection)?;
    let (rows, page) = page.records(rows);
    let traces = rows
        .into_iter()
        .map(TraceSummaryRow::decode)
        .collect::<Result<_>>()?;
    Ok(TraceReadResponse::List { traces, page })
}

// Both the invocation list and grouped usage query use these identical predicates.
macro_rules! filter_calls {
    ($query:expr, $filter:expr) => {{
        let filter = $filter;
        let mut query = filter_scope!($query, call, filter);
        if let Some(value) = &filter.name {
            query = query.filter(call::name.eq(value));
        }
        query = match &filter.version {
            TraceVersionSelector::Any {} => query,
            TraceVersionSelector::Exact { value } => query.filter(call::version.eq(value)),
            TraceVersionSelector::Unversioned {} => query.filter(call::version.is_null()),
        };
        if let Some(value) = filter.kind {
            query = query.filter(call::kind.eq(value.as_str()));
        }
        if let Some(value) = filter.outcome {
            query = query.filter(call::outcome.eq(value.as_str()));
        }
        query
    }};
}

fn calls(
    connection: &mut DbConnection,
    project: &str,
    filter: TraceFilter,
) -> Result<TraceReadResponse> {
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
    let (rows, page) = page.records(rows);
    let calls = rows
        .into_iter()
        .map(InvocationRow::decode)
        .collect::<Result<_>>()?;
    Ok(TraceReadResponse::Calls { calls, page })
}

#[derive(Queryable)]
struct UsageRow {
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

impl UsageRow {
    fn decode(self) -> Result<TraceUsage> {
        Ok(TraceUsage {
            application_id: self.application_id,
            environment: self.environment,
            agent_id: self.agent_id,
            kind: self.kind.parse()?,
            name: self.name,
            version: self.version,
            calls: self.calls,
            succeeded: self.succeeded,
            failed: self.failed,
            incomplete: self.incomplete,
            canceled: self.canceled,
            unknown: self.unknown,
        })
    }
}

fn usage(
    connection: &mut DbConnection,
    project: &str,
    filter: TraceFilter,
) -> Result<TraceReadResponse> {
    let page = Page::parse(filter.limit, filter.cursor.as_deref())?;
    let rows = usage_rows(connection, project, &filter, &page)?;
    let (rows, page) = page.records(rows);
    let usage = rows
        .into_iter()
        .map(UsageRow::decode)
        .collect::<Result<_>>()?;
    Ok(TraceReadResponse::Usage { usage, page })
}

fn usage_rows(
    connection: &mut DbConnection,
    project: &str,
    filter: &TraceFilter,
    page: &Page,
) -> Result<Vec<UsageRow>> {
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
        .load::<UsageRow>(connection)?)
}

#[derive(Queryable)]
struct ApplicationScopeRow {
    application_id: String,
    environment: String,
    agent_id: String,
}

impl From<ApplicationScopeRow> for TraceApplicationScope {
    fn from(row: ApplicationScopeRow) -> Self {
        Self {
            application_id: row.application_id,
            environment: row.environment,
            agent_id: row.agent_id,
        }
    }
}

fn status(
    connection: &mut DbConnection,
    project: &str,
    application: Option<&str>,
    environment: Option<&str>,
) -> Result<TraceReadResponse> {
    let (pending_batches, oldest_pending_at) =
        pending_stats(connection, project, application, environment)?;
    let (scopes, scopes_truncated) =
        discover_scopes(connection, project, application, environment)?;
    Ok(TraceReadResponse::Status {
        ok: true,
        pending_batches,
        oldest_pending_at,
        scopes,
        scopes_truncated,
    })
}

fn pending_stats(
    connection: &mut DbConnection,
    project: &str,
    application: Option<&str>,
    environment: Option<&str>,
) -> Result<(i64, Option<TraceTimestamp>)> {
    let mut pending = batch::table
        .filter(batch::project_id.eq(project))
        .filter(batch::projected.eq(false))
        .select((count_star(), min(batch::received_at)))
        .into_boxed::<DbBackend>();
    if let Some(value) = application {
        pending = pending.filter(batch::application_id.eq(value));
    }
    if let Some(value) = environment {
        pending = pending.filter(batch::environment.eq(value));
    }
    let (count, oldest) = pending.first::<(i64, Option<String>)>(connection)?;
    let oldest = oldest
        .as_deref()
        .map(parse_time)
        .transpose()?
        .map(Into::into);
    Ok((count, oldest))
}

fn discover_scopes(
    connection: &mut DbConnection,
    project: &str,
    application: Option<&str>,
    environment: Option<&str>,
) -> Result<(Vec<TraceApplicationScope>, bool)> {
    let mut scopes = event::table
        .filter(event::project_id.eq(project))
        .select((event::application_id, event::environment, event::agent_id))
        .distinct()
        .into_boxed::<DbBackend>();
    if let Some(value) = application {
        scopes = scopes.filter(event::application_id.eq(value));
    }
    if let Some(value) = environment {
        scopes = scopes.filter(event::environment.eq(value));
    }
    let mut rows = scopes
        .order((
            event::application_id.asc(),
            event::environment.asc(),
            event::agent_id.asc(),
        ))
        .limit(SCOPE_LIMIT as i64 + 1)
        .load::<ApplicationScopeRow>(connection)?;
    let truncated = rows.len() > SCOPE_LIMIT;
    rows.truncate(SCOPE_LIMIT);
    Ok((rows.into_iter().map(Into::into).collect(), truncated))
}
