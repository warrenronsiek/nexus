// @feature agent-traces
// @feature persistence
// @spec docs/features/agent-traces.md
// @spec docs/features/persistence.md
use super::content_hash;
use super::rows::{EventRow, InvocationRow};
use crate::agent_traces::{TraceCapability, TraceEvent, TraceEventKind};
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{
    trace_batches as batch, trace_events as event, trace_invocations as call,
};
use anyhow::{Context, Result};
use diesel::prelude::*;
use diesel::upsert::excluded;
use std::collections::BTreeMap;

#[cfg(feature = "postgres")]
diesel::define_sql_function! {
    fn pg_try_advisory_xact_lock(key: diesel::sql_types::BigInt) -> diesel::sql_types::Bool;
}

pub(super) fn project_next(connection: &mut DbConnection) -> Result<bool> {
    // A single projector serializes invocation upserts without taking the Nexus-wide aggregate lock.
    #[cfg(feature = "postgres")]
    if !diesel::select(pg_try_advisory_xact_lock(0x4e585452414345))
        .get_result::<bool>(connection)?
    {
        return Ok(false);
    }
    let Some(batch) = next_batch(connection)? else {
        return Ok(false);
    };
    project_batch(connection, batch)
}

#[derive(Queryable)]
struct PendingBatch {
    id: String,
    project_id: String,
    received_at: String,
}

fn next_batch(connection: &mut DbConnection) -> Result<Option<PendingBatch>> {
    batch::table
        .filter(batch::projected.eq(false))
        .order((batch::received_at.asc(), batch::id.asc()))
        .select((batch::id, batch::project_id, batch::received_at))
        .first(connection)
        .optional()
        .map_err(Into::into)
}

fn project_batch(connection: &mut DbConnection, pending: PendingBatch) -> Result<bool> {
    apply_batch(connection, &pending.id)?;
    crate::persistence::dashboard::discover_project(
        connection,
        &pending.project_id,
        &pending.received_at,
    )?;
    diesel::update(batch::table.find(pending.id))
        .set(batch::projected.eq(true))
        .execute(connection)?;
    Ok(true)
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = event)]
struct SourceEvent {
    project_id: String,
    application_id: String,
    environment: String,
    occurred_at: String,
    event_json: String,
}

struct InvocationEvent {
    source: SourceEvent,
    event: TraceEvent,
    kind: TraceCapability,
    phase: InvocationPhase,
}

enum InvocationPhase {
    Started,
    Finished,
}

impl InvocationEvent {
    fn decode(source: SourceEvent) -> Result<Option<Self>> {
        use InvocationPhase::{Finished, Started};
        use TraceCapability::{Skill, Tool};
        let event: TraceEvent = serde_json::from_str(&source.event_json)?;
        let (kind, phase) = match event.kind {
            TraceEventKind::ToolStarted => (Tool, Started),
            TraceEventKind::ToolFinished => (Tool, Finished),
            TraceEventKind::SkillStarted => (Skill, Started),
            TraceEventKind::SkillFinished => (Skill, Finished),
            _ => return Ok(None),
        };
        Ok(Some(Self {
            source,
            event,
            kind,
            phase,
        }))
    }

    fn key(&self) -> Result<String> {
        content_hash(&[
            self.source.project_id.as_str(),
            &self.source.application_id,
            &self.source.environment,
            &self.event.trace_id,
            &self.event.agent_id,
            self.kind.as_str(),
            self.event
                .invocation_id
                .as_deref()
                .context("missing projected invocation ID")?,
        ])
    }

    fn order_key(&self) -> (&str, &str) {
        (&self.source.occurred_at, &self.event.event_id)
    }
}

#[derive(Default)]
struct InvocationEvents {
    start: Option<InvocationEvent>,
    finish: Option<InvocationEvent>,
}

impl InvocationEvents {
    fn observe(&mut self, observed: InvocationEvent) {
        match observed.phase {
            InvocationPhase::Started
                if self
                    .start
                    .as_ref()
                    .is_none_or(|previous| observed.order_key() < previous.order_key()) =>
            {
                self.start = Some(observed);
            }
            InvocationPhase::Finished
                if self
                    .finish
                    .as_ref()
                    .is_none_or(|previous| observed.order_key() > previous.order_key()) =>
            {
                self.finish = Some(observed);
            }
            _ => {}
        }
    }

    fn finish(self, id: String) -> Result<InvocationRow> {
        let first = self
            .start
            .as_ref()
            .or(self.finish.as_ref())
            .context("no invocation boundary")?;
        let terminal = self.finish.as_ref().map(|end| &end.event);
        let source = &first.source;
        Ok(InvocationRow {
            id,
            project_id: source.project_id.clone(),
            application_id: source.application_id.clone(),
            environment: source.environment.clone(),
            trace_id: first.event.trace_id.clone(),
            agent_id: first.event.agent_id.clone(),
            invocation_id: first
                .event
                .invocation_id
                .clone()
                .context("missing projected invocation ID")?,
            kind: first.kind.as_str().to_owned(),
            name: first
                .event
                .name
                .clone()
                .or_else(|| terminal.and_then(|event| event.name.clone())),
            version: first
                .event
                .version
                .clone()
                .or_else(|| terminal.and_then(|event| event.version.clone())),
            occurred_at: source.occurred_at.clone(),
            outcome: terminal
                .and_then(|event| event.outcome)
                .map(|outcome| outcome.as_str().to_owned()),
            duration_ms: terminal.and_then(|event| event.duration_ms),
            started_event_id: self
                .start
                .as_ref()
                .map(|start| start.event.event_id.clone()),
            finished_event_id: self.finish.as_ref().map(|end| end.event.event_id.clone()),
        })
    }
}

fn apply_batch(connection: &mut DbConnection, key: &str) -> Result<()> {
    let mut invocations = batch_invocations(connection, key)?;
    if invocations.is_empty() {
        return Ok(());
    }
    let accepted = previous_events(connection, &invocations)?;
    add_events(&mut invocations, accepted)?;
    save_invocations(connection, invocations)
}

fn batch_invocations(
    connection: &mut DbConnection,
    key: &str,
) -> Result<BTreeMap<String, InvocationEvents>> {
    let incoming = event::table
        .filter(event::batch_key.eq(key))
        .select(SourceEvent::as_select())
        .load(connection)?;
    let mut invocations = BTreeMap::new();
    add_events(&mut invocations, incoming)?;
    Ok(invocations)
}

fn previous_events(
    connection: &mut DbConnection,
    invocations: &BTreeMap<String, InvocationEvents>,
) -> Result<Vec<SourceEvent>> {
    let receipts = call::table
        .filter(call::id.eq_any(invocations.keys()))
        .select((
            call::project_id,
            call::application_id,
            call::environment,
            call::started_event_id,
            call::finished_event_id,
        ))
        .load::<(String, String, String, Option<String>, Option<String>)>(connection)?;
    let keys = receipts
        .iter()
        .flat_map(|(project, app, env, start, finish)| {
            [start, finish]
                .into_iter()
                .flatten()
                .map(move |event_id| EventRow::key(project, app, env, event_id))
        })
        .collect::<Result<Vec<_>>>()?;
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    Ok(event::table
        .filter(event::id.eq_any(keys))
        .select(SourceEvent::as_select())
        .load(connection)?)
}

fn save_invocations(
    connection: &mut DbConnection,
    invocations: BTreeMap<String, InvocationEvents>,
) -> Result<()> {
    let rows = invocations
        .into_iter()
        .map(|(id, events)| events.finish(id))
        .collect::<Result<Vec<_>>>()?;
    upsert_invocations(connection, &rows)
}

fn add_events(
    invocations: &mut BTreeMap<String, InvocationEvents>,
    events: Vec<SourceEvent>,
) -> Result<()> {
    for source in events {
        if let Some(observed) = InvocationEvent::decode(source)? {
            invocations
                .entry(observed.key()?)
                .or_default()
                .observe(observed);
        }
    }
    Ok(())
}

fn upsert_invocations(connection: &mut DbConnection, rows: &[InvocationRow]) -> Result<()> {
    diesel::insert_into(call::table)
        .values(rows)
        .on_conflict(call::id)
        .do_update()
        .set((
            call::name.eq(excluded(call::name)),
            call::version.eq(excluded(call::version)),
            call::occurred_at.eq(excluded(call::occurred_at)),
            call::outcome.eq(excluded(call::outcome)),
            call::duration_ms.eq(excluded(call::duration_ms)),
            call::started_event_id.eq(excluded(call::started_event_id)),
            call::finished_event_id.eq(excluded(call::finished_event_id)),
        ))
        .execute(connection)?;
    Ok(())
}
