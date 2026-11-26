// @feature agent-traces
// @feature persistence
// @spec docs/features/agent-traces.md
// @spec docs/features/persistence.md
// @boundary trace-read-payload: authorized reads reattach the captured JSON content.
use super::{DbBackend, DbConnection, DEFAULT_PAGE_ITEMS};
use crate::agent_traces::{
    TraceEvent, TraceEventView, TracePage, TracePayloadState, TraceReadResponse,
};
use crate::persistence::agent_traces::EventRow;
use crate::persistence::schema::trace_events as event;
use anyhow::{ensure, Context, Result};
use diesel::prelude::*;
use std::collections::HashMap;

const MAX_EVENT_PAGE_BYTES: usize = 768 * 1024;

pub(super) struct Selection<'a> {
    pub project: &'a str,
    pub application: &'a str,
    pub environment: &'a str,
    pub trace: &'a str,
    pub event: Option<&'a str>,
}

pub(super) enum Content {
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

#[derive(Queryable)]
struct Anchor {
    sequence: Option<i64>,
    occurred_at: String,
    event_id: String,
}

pub(super) fn read(
    connection: &mut DbConnection,
    selection: Selection<'_>,
    content: Content,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Result<TraceReadResponse> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_ITEMS);
    let heads = event_heads(connection, &selection, limit, cursor)?;
    let kept = event_page_len(&heads, &selection, &content, limit);
    let visible = &heads[..kept];
    let page = event_page(visible, heads.len())?;
    let payloads = match content {
        Content::Metadata => HashMap::new(),
        Content::WithPayload => load_payloads(connection, visible)?,
    };
    let events = visible
        .iter()
        .map(|head| event_view(head, &selection, &content, &payloads))
        .collect::<Result<Vec<_>>>()?;
    Ok(TraceReadResponse::Events { events, page })
}

fn event_page(visible: &[EventHead], fetched: usize) -> Result<TracePage> {
    let cursor = if visible.len() < fetched {
        let last = visible
            .last()
            .context("trace event exceeds the read response budget")?;
        Some(format!("event:{}", last.id))
    } else {
        None
    };
    Ok(TracePage::new(cursor))
}

fn scoped_events<'a>(selection: &Selection<'a>) -> Result<event::BoxedQuery<'a, DbBackend>> {
    let mut query = event::table
        .filter(event::project_id.eq(selection.project))
        .filter(event::application_id.eq(selection.application))
        .filter(event::environment.eq(selection.environment))
        .filter(event::trace_id.eq(selection.trace))
        .into_boxed::<DbBackend>();
    if let Some(value) = selection.event {
        query = query.filter(event::id.eq(EventRow::key(
            selection.project,
            selection.application,
            selection.environment,
            value,
        )?));
    }
    Ok(query)
}

/// Resolve an immutable ordering anchor in the exact trace scope. Payload expiry
/// cannot remove anchor rows from the journal.
fn find_anchor(
    connection: &mut DbConnection,
    selection: &Selection<'_>,
    cursor: &str,
) -> Result<Anchor> {
    let id = cursor
        .strip_prefix("event:")
        .context("invalid trace event cursor")?;
    ensure!(
        id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid trace event cursor"
    );
    scoped_events(selection)?
        .filter(event::id.eq(id))
        .select((event::source_sequence, event::occurred_at, event::event_id))
        .first(connection)
        .optional()?
        .context("trace event cursor is outside the requested trace")
}

fn event_heads(
    connection: &mut DbConnection,
    selection: &Selection<'_>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Vec<EventHead>> {
    let anchor = cursor
        .map(|cursor| find_anchor(connection, selection, cursor))
        .transpose()?;
    let query = seek(scoped_events(selection)?, anchor.as_ref());
    Ok(query
        .select((
            event::id,
            event::event_json,
            event::payload_bytes,
            event::payload_expired,
        ))
        .order((
            event::source_sequence.is_null().asc(),
            event::source_sequence.asc(),
            event::occurred_at.asc(),
            event::event_id.asc(),
        ))
        .limit(limit as i64 + 1)
        .load(connection)?)
}

fn seek<'a>(
    query: event::BoxedQuery<'a, DbBackend>,
    anchor: Option<&'a Anchor>,
) -> event::BoxedQuery<'a, DbBackend> {
    let Some(anchor) = anchor else { return query };
    let after_time = event::occurred_at
        .gt(&anchor.occurred_at)
        .or(event::occurred_at
            .eq(&anchor.occurred_at)
            .and(event::event_id.gt(&anchor.event_id)));
    match anchor.sequence {
        Some(sequence) => query.filter(
            event::source_sequence
                .is_null()
                .or(event::source_sequence.gt(sequence))
                .or(event::source_sequence.eq(sequence).and(after_time)),
        ),
        None => query.filter(event::source_sequence.is_null().and(after_time)),
    }
}

fn event_page_len(
    heads: &[EventHead],
    selection: &Selection<'_>,
    content: &Content,
    limit: usize,
) -> usize {
    let envelope_bytes = 2 * (selection.application.len() + selection.environment.len()) + 96;
    let mut bytes = 128;
    heads
        .iter()
        .take(limit)
        .take_while(|head| {
            let payload_bytes = match content {
                Content::Metadata => 0,
                Content::WithPayload => head.payload_bytes as usize,
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
    selection: &Selection<'_>,
    content: &Content,
    payloads: &HashMap<String, String>,
) -> Result<TraceEventView> {
    let mut event: TraceEvent = serde_json::from_str(&head.event_json)?;
    event.payload = payloads
        .get(&head.id)
        .map(|value| serde_json::from_str(value))
        .transpose()?;
    let payload_state = payload_state(head, content, &event);
    Ok(TraceEventView {
        event,
        application_id: selection.application.to_owned(),
        environment: selection.environment.to_owned(),
        has_payload: payload_state == TracePayloadState::Available,
        payload_state,
    })
}

fn payload_state(head: &EventHead, content: &Content, event: &TraceEvent) -> TracePayloadState {
    let expired_during_read = matches!(content, Content::WithPayload)
        && event.payload.is_none()
        && head.payload_bytes > 0;
    match (
        head.payload_expired || expired_during_read,
        head.payload_bytes > 0,
    ) {
        (true, _) => TracePayloadState::Expired,
        (false, true) => TracePayloadState::Available,
        _ => TracePayloadState::NotCaptured,
    }
}
