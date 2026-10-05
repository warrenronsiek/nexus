// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
// @entrypoint Store::add_memory
use super::schema::{
    memory_activations, memory_compaction_attempts, memory_compaction_queue, memory_entries,
    memory_frontier_snapshots, memory_spaces, memory_summaries,
};
use super::transaction::{transaction, TransactionMode};
use super::Store;
use crate::memory::{
    MemoryCompactionAttempt, MemoryCompactionJob, MemoryCompactionOutcome, MemoryContextLimits,
    MemoryContextStatus, MemoryEntry, MemoryHealth, MemoryNode, MemoryProvenance,
    MemoryProviderHealth, MemoryReadScope, MemoryScope, MemoryScopeHealth, MemorySummary,
    MemorySummaryInsert, MemoryWriteResult,
};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use regex::RegexBuilder;
use std::collections::BTreeMap;
use std::time::{Duration as StdDuration, Instant};
use uuid::Uuid;

mod compaction;
mod context;
mod health;
mod queue;
mod records;
mod snapshot;
use records::*;
#[cfg(test)]
mod tests;

impl Store {
    pub fn add_memory(
        &mut self,
        scope: &MemoryScope,
        content: &str,
        provenance: &MemoryProvenance,
    ) -> Result<MemoryWriteResult> {
        self.write_memory(scope, content, provenance, true)
    }

    pub(crate) fn add_memory_from_lifecycle(
        &mut self,
        scope: &MemoryScope,
        content: &str,
        provenance: &MemoryProvenance,
    ) -> Result<MemoryWriteResult> {
        self.write_memory(scope, content, provenance, false)
    }

    fn write_memory(
        &mut self,
        scope: &MemoryScope,
        content: &str,
        provenance: &MemoryProvenance,
        repair_snapshot: bool,
    ) -> Result<MemoryWriteResult> {
        let content = normalize_memory_text(content, crate::memory::MAX_MEMORY_ENTRY_BYTES)?;
        validate_provenance(provenance)?;
        let pending = PendingMemory {
            scope,
            provenance,
            content_hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
            content,
            now: Utc::now().to_rfc3339(),
        };
        let commit = retry_memory_write(&mut self.connection, |connection| {
            transaction(connection, TransactionMode::Immediate, |connection| {
                write_memory_transaction(connection, &pending)
            })
        })?;
        if repair_snapshot && !commit.snapshot_current {
            snapshot::rebuild_frontier_snapshot(&mut self.connection, scope)?;
        }
        Ok(commit.result)
    }

    pub fn search_memories(
        &mut self,
        scope: &MemoryReadScope,
        pattern: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>> {
        let expression = RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .context("invalid memory search regular expression")?;
        let mut entries = Vec::new();
        for concrete_scope in read_scopes(scope) {
            entries.extend(load_entries(&mut self.connection, &concrete_scope)?);
        }
        entries.retain(|entry| expression.is_match(&entry.content));
        entries.sort_by(compare_memory_recency);
        entries.truncate(limit.min(crate::memory::MAX_MEMORY_SEARCH_RESULTS));
        Ok(entries)
    }
}

fn compare_memory_recency(left: &MemoryEntry, right: &MemoryEntry) -> std::cmp::Ordering {
    right
        .created_at
        .cmp(&left.created_at)
        .then_with(|| right.ordinal.cmp(&left.ordinal))
        .then_with(|| left.id.cmp(&right.id))
}

struct PendingMemory<'a> {
    scope: &'a MemoryScope,
    provenance: &'a MemoryProvenance,
    content: String,
    content_hash: String,
    now: String,
}

struct MemoryWriteCommit {
    result: MemoryWriteResult,
    snapshot_current: bool,
}

fn write_memory_transaction(
    connection: &mut SqliteConnection,
    pending: &PendingMemory<'_>,
) -> Result<MemoryWriteCommit> {
    let space = get_or_create_space(connection, pending.scope, &pending.now)?;
    if let Some(row) = find_duplicate_entry(connection, &space.id, &pending.content_hash)? {
        return Ok(MemoryWriteCommit {
            result: MemoryWriteResult {
                entry: entry_from_row(row, pending.scope.clone())?,
                inserted: false,
            },
            snapshot_current: snapshot::space_snapshot_is_current(connection, &space)?,
        });
    }
    insert_memory_entry(connection, &space, pending)
}

fn find_duplicate_entry(
    connection: &mut SqliteConnection,
    space_id: &str,
    content_hash: &str,
) -> Result<Option<MemoryEntryRow>> {
    memory_entries::table
        .filter(memory_entries::space_id.eq(space_id))
        .filter(memory_entries::content_hash.eq(content_hash))
        .select(MemoryEntryRow::as_select())
        .first(connection)
        .optional()
        .map_err(Into::into)
}

fn insert_memory_entry(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    pending: &PendingMemory<'_>,
) -> Result<MemoryWriteCommit> {
    let entry_id = Uuid::new_v4().to_string();
    let entry = persist_memory_entry(connection, space, pending, &entry_id)?;
    let snapshot_current =
        snapshot::advance_snapshot_for_append(connection, space, &entry, &pending.now)?;
    queue::enqueue_raw_parent(connection, space, &entry, &pending.now)?;
    advance_memory_space(connection, space, &pending.now)?;
    Ok(MemoryWriteCommit {
        result: MemoryWriteResult {
            entry,
            inserted: true,
        },
        snapshot_current,
    })
}

fn persist_memory_entry(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    pending: &PendingMemory<'_>,
    entry_id: &str,
) -> Result<MemoryEntry> {
    diesel::insert_into(memory_entries::table)
        .values(NewMemoryEntry {
            id: entry_id,
            space_id: &space.id,
            ordinal: space.next_ordinal,
            content: &pending.content,
            content_hash: &pending.content_hash,
            agent: &pending.provenance.agent,
            session_id: &pending.provenance.session_id,
            model_id: pending.provenance.model_id.as_deref(),
            config_hash: &pending.provenance.config_hash,
            created_at: &pending.now,
        })
        .execute(connection)?;
    let row = memory_entries::table
        .find(entry_id)
        .select(MemoryEntryRow::as_select())
        .first(connection)?;
    entry_from_row(row, pending.scope.clone())
}

fn advance_memory_space(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    now: &str,
) -> Result<()> {
    diesel::update(memory_spaces::table.find(&space.id))
        .set((
            memory_spaces::next_ordinal.eq(space.next_ordinal + 1),
            memory_spaces::revision.eq(space.revision + 1),
            memory_spaces::updated_at.eq(now),
        ))
        .execute(connection)?;
    Ok(())
}

fn load_raw_nodes(
    connection: &mut SqliteConnection,
    space_id: &str,
    scope: &MemoryScope,
) -> Result<Vec<MemoryNode>> {
    memory_entries::table
        .filter(memory_entries::space_id.eq(space_id))
        .order(memory_entries::ordinal.asc())
        .select(MemoryEntryRow::as_select())
        .load(connection)?
        .into_iter()
        .map(|row| entry_from_row(row, scope.clone()).map(MemoryNode::Raw))
        .collect()
}

fn load_summaries(
    connection: &mut SqliteConnection,
    space_id: &str,
    scope: &MemoryScope,
) -> Result<Vec<MemorySummary>> {
    memory_summaries::table
        .filter(memory_summaries::space_id.eq(space_id))
        .order((
            memory_summaries::level.asc(),
            memory_summaries::start_ordinal.asc(),
        ))
        .select(MemorySummaryRow::as_select())
        .load(connection)?
        .into_iter()
        .map(|row| compaction::summary_from_row(row, scope.clone()))
        .collect()
}

fn node_bytes(nodes: &[MemoryNode]) -> usize {
    nodes.iter().map(|node| node.content().len()).sum()
}

fn read_scopes(scope: &MemoryReadScope) -> Vec<MemoryScope> {
    match scope {
        MemoryReadScope::Global => vec![MemoryScope::Global],
        MemoryReadScope::Project { project_id } => vec![MemoryScope::project(project_id.clone())],
        MemoryReadScope::Layered { project_id } => vec![
            MemoryScope::Global,
            MemoryScope::project(project_id.clone()),
        ],
    }
}

fn load_entries(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
) -> Result<Vec<MemoryEntry>> {
    let Some(space) = find_space(connection, scope)? else {
        return Ok(Vec::new());
    };
    memory_entries::table
        .filter(memory_entries::space_id.eq(&space.id))
        .order(memory_entries::ordinal.asc())
        .select(MemoryEntryRow::as_select())
        .load(connection)?
        .into_iter()
        .map(|row| entry_from_row(row, scope.clone()))
        .collect()
}

fn get_or_create_space(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
    now: &str,
) -> Result<MemorySpaceRow> {
    validate_scope(scope)?;
    if let Some(space) = find_space(connection, scope)? {
        return Ok(space);
    }
    let id = Uuid::new_v4().to_string();
    let (scope_text, project_id) = scope_parts(scope);
    diesel::insert_into(memory_spaces::table)
        .values(NewMemorySpace {
            id: &id,
            scope: scope_text,
            project_id,
            next_ordinal: 0,
            revision: 0,
            created_at: now,
            updated_at: now,
        })
        .execute(connection)?;
    memory_spaces::table
        .find(id)
        .select(MemorySpaceRow::as_select())
        .first(connection)
        .map_err(Into::into)
}

fn find_space(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
) -> Result<Option<MemorySpaceRow>> {
    let query = memory_spaces::table
        .select(MemorySpaceRow::as_select())
        .into_boxed();
    match scope {
        MemoryScope::Global => query
            .filter(memory_spaces::scope.eq("global"))
            .first(connection)
            .optional()
            .map_err(Into::into),
        MemoryScope::Project { project_id } => query
            .filter(memory_spaces::scope.eq("project"))
            .filter(memory_spaces::project_id.eq(project_id))
            .first(connection)
            .optional()
            .map_err(Into::into),
    }
}

fn scope_parts(scope: &MemoryScope) -> (&'static str, Option<&str>) {
    match scope {
        MemoryScope::Global => ("global", None),
        MemoryScope::Project { project_id } => ("project", Some(project_id)),
    }
}

fn validate_scope(scope: &MemoryScope) -> Result<()> {
    if matches!(scope, MemoryScope::Project { project_id } if project_id.trim().is_empty()) {
        bail!("project memory scope requires a non-empty project id");
    }
    Ok(())
}

fn validate_provenance(provenance: &MemoryProvenance) -> Result<()> {
    for (name, value) in [
        ("agent", provenance.agent.as_str()),
        ("session_id", provenance.session_id.as_str()),
        ("config_hash", provenance.config_hash.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("memory provenance {name} cannot be empty");
        }
    }
    Ok(())
}

fn normalize_memory_text(content: &str, max_bytes: usize) -> Result<String> {
    if content
        .chars()
        .any(|character| character.is_control() || matches!(character, '\u{2028}' | '\u{2029}'))
    {
        bail!("memory text must be a single line without control characters");
    }
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        bail!("memory text cannot be empty");
    }
    if normalized.len() > max_bytes {
        bail!("memory text exceeds the {max_bytes}-byte limit");
    }
    Ok(normalized)
}

fn entry_from_row(row: MemoryEntryRow, scope: MemoryScope) -> Result<MemoryEntry> {
    Ok(MemoryEntry {
        id: row.id,
        scope,
        ordinal: u64::try_from(row.ordinal).context("memory entry ordinal is negative")?,
        content: row.content,
        content_hash: row.content_hash,
        provenance: MemoryProvenance {
            agent: row.agent,
            session_id: row.session_id,
            model_id: row.model_id,
            config_hash: row.config_hash,
        },
        created_at: DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
    })
}

fn retry_memory_write<T>(
    connection: &mut SqliteConnection,
    mut operation: impl FnMut(&mut SqliteConnection) -> Result<T>,
) -> Result<T> {
    let deadline = Instant::now() + StdDuration::from_secs(2);
    let mut delay = StdDuration::from_millis(5);
    loop {
        match operation(connection) {
            Err(error) if sqlite_is_locked(&error) && Instant::now() < deadline => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(StdDuration::from_millis(100));
            }
            result => return result,
        }
    }
}

fn sqlite_is_locked(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let Some(diesel::result::Error::DatabaseError(_, information)) =
            cause.downcast_ref::<diesel::result::Error>()
        else {
            return false;
        };
        matches!(
            information.message(),
            "database is locked" | "database is busy"
        )
    })
}
