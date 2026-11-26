// @feature agent-traces
// @feature persistence
// @spec docs/features/agent-traces.md
// @spec docs/features/persistence.md
// @entrypoint Store::trace_append
// @entrypoint Store::trace_project_pending
// @entrypoint Store::trace_read
// @boundary trace-storage: canonical structured events and detached trace content.
mod projection;
mod reads;
mod rows;

pub(super) use rows::{BatchRow, EventRow};

use super::backend::DbConnection;
use super::schema::{trace_batches, trace_events};
use super::Store;
use crate::agent_traces::{canonical_time, TraceBatch, TraceConflict, TraceQuery, TraceReceipt};
use anyhow::{ensure, Result};
use diesel::dsl::{exists, not};
use diesel::prelude::*;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

impl Store {
    pub fn trace_append(&mut self, project: &str, batch: TraceBatch) -> Result<TraceReceipt> {
        batch.validate()?;
        self.trace_append_validated(project, batch)
    }

    pub(crate) fn trace_append_validated(
        &mut self,
        project: &str,
        batch: TraceBatch,
    ) -> Result<TraceReceipt> {
        let receipt = BatchRow::new(project, &batch)?;
        let mut events = batch
            .events
            .into_iter()
            .map(|event| EventRow::new(&receipt, event))
            .collect::<Result<Vec<_>>>()?;
        events.sort_unstable_by(|left, right| left.id.cmp(&right.id));
        write_transaction(&mut self.connection, |connection| {
            append(connection, receipt, &events)
        })
    }

    pub fn trace_project_pending(&mut self, limit: usize) -> Result<usize> {
        let mut processed = 0;
        for _ in 0..limit.min(64) {
            if !write_transaction(&mut self.connection, projection::project_next)? {
                break;
            }
            processed += 1;
        }
        Ok(processed)
    }

    pub fn trace_expire_payloads(&mut self, cutoff: &str, limit: usize) -> Result<usize> {
        let cutoff = canonical_time(cutoff)?;
        write_transaction(&mut self.connection, |connection| {
            expire_payloads(connection, &cutoff, limit.min(512) as i64)
        })
    }

    pub fn trace_read(&mut self, project: &str, query: TraceQuery) -> Result<Value> {
        query.validate()?;
        reads::read(&mut self.connection, project, query)
    }
}

fn append(
    connection: &mut DbConnection,
    receipt: BatchRow,
    events: &[EventRow],
) -> Result<TraceReceipt> {
    let new_batch = diesel::insert_into(trace_batches::table)
        .values(&receipt)
        .on_conflict(trace_batches::id)
        .do_nothing()
        .execute(connection)?;
    if new_batch == 0 {
        return replay_batch(connection, &receipt);
    }
    let accepted = insert_events(connection, events)?;
    save_receipt(connection, receipt, accepted, events.len() - accepted)
}

fn replay_batch(connection: &mut DbConnection, receipt: &BatchRow) -> Result<TraceReceipt> {
    let existing = trace_batches::table
        .find(&receipt.id)
        .first::<BatchRow>(connection)?;
    ensure_conflict(existing.content_hash == receipt.content_hash, "batch")?;
    existing.receipt()
}

fn insert_events(connection: &mut DbConnection, events: &[EventRow]) -> Result<usize> {
    let inserted = diesel::insert_into(trace_events::table)
        .values(events)
        .on_conflict(trace_events::id)
        .do_nothing()
        .returning(trace_events::id)
        .get_results::<String>(connection)?;
    if events.len() != inserted.len() {
        check_replayed_events(connection, events)?;
    }
    Ok(inserted.len())
}

fn save_receipt(
    connection: &mut DbConnection,
    mut receipt: BatchRow,
    accepted: usize,
    duplicates: usize,
) -> Result<TraceReceipt> {
    receipt.accepted_events = i32::try_from(accepted)?;
    receipt.duplicate_events = i32::try_from(duplicates)?;
    receipt.projected = accepted == 0;
    diesel::update(trace_batches::table.find(&receipt.id))
        .set((
            trace_batches::accepted_events.eq(receipt.accepted_events),
            trace_batches::duplicate_events.eq(receipt.duplicate_events),
            trace_batches::projected.eq(receipt.projected),
        ))
        .execute(connection)?;
    receipt.receipt()
}

fn check_replayed_events(connection: &mut DbConnection, events: &[EventRow]) -> Result<()> {
    // A separate Read Committed statement sees any winner of the prior INSERT conflict.
    let stored = trace_events::table
        .filter(trace_events::id.eq_any(events.iter().map(|event| &event.id)))
        .select((trace_events::id, trace_events::content_hash))
        .load::<(String, String)>(connection)?
        .into_iter()
        .collect::<HashMap<_, _>>();
    ensure_conflict(
        events
            .iter()
            .all(|event| stored.get(&event.id) == Some(&event.content_hash)),
        "event",
    )
}

fn expire_payloads(connection: &mut DbConnection, cutoff: &str, limit: i64) -> Result<usize> {
    let eligible = trace_batches::table
        .inner_join(trace_events::table.on(trace_events::batch_key.eq(trace_batches::id)))
        .filter(trace_batches::received_at.lt(cutoff))
        .filter(trace_events::payload_json.is_not_null())
        .select(trace_events::id)
        .order((
            trace_batches::received_at.asc(),
            trace_batches::id.asc(),
            trace_events::id.asc(),
        ))
        .limit(limit)
        .load::<String>(connection)?;
    Ok(diesel::update(
        trace_events::table
            .filter(trace_events::id.eq_any(eligible))
            .filter(trace_events::payload_json.is_not_null()),
    )
    .set((
        trace_events::payload_json.eq::<Option<String>>(None),
        trace_events::payload_bytes.eq(0),
        trace_events::payload_expired.eq(true),
    ))
    .execute(connection)?)
}

fn ensure_conflict(same_content: bool, kind: &'static str) -> Result<()> {
    ensure!(same_content, TraceConflict(kind));
    Ok(())
}

#[cfg(feature = "postgres")]
fn write_transaction<T>(
    connection: &mut DbConnection,
    operation: impl FnOnce(&mut DbConnection) -> Result<T>,
) -> Result<T> {
    // Unique keys and the projection transaction's separate lock protect only trace state.
    connection.transaction(operation)
}

#[cfg(feature = "sqlite")]
fn write_transaction<T>(
    connection: &mut DbConnection,
    operation: impl FnOnce(&mut DbConnection) -> Result<T>,
) -> Result<T> {
    super::transaction::transaction(
        connection,
        super::transaction::TransactionMode::Immediate,
        operation,
    )
}

fn content_hash(value: &impl Serialize) -> Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(value)?)
        .to_hex()
        .to_string())
}

pub(super) fn rebuild_snapshot(connection: &mut DbConnection) -> Result<()> {
    let unbound = trace_events::table.filter(not(exists(
        trace_batches::table
            .filter(trace_batches::id.eq(trace_events::batch_key))
            .filter(trace_batches::project_id.eq(trace_events::project_id))
            .filter(trace_batches::application_id.eq(trace_events::application_id))
            .filter(trace_batches::environment.eq(trace_events::environment))
            .filter(trace_batches::accepted_events.gt(0)),
    )));
    ensure!(
        !diesel::select(exists(unbound)).get_result::<bool>(connection)?,
        "portable trace event references an invalid batch receipt"
    );
    diesel::update(trace_batches::table)
        .filter(trace_batches::accepted_events.gt(0))
        .set(trace_batches::projected.eq(false))
        .execute(connection)?;
    while projection::project_next(connection)? {}
    Ok(())
}
