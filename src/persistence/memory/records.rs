// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::*;

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = memory_spaces)]
pub(super) struct MemorySpaceRow {
    pub(super) id: String,
    pub(super) scope: String,
    pub(super) project_id: Option<String>,
    pub(super) next_ordinal: i64,
    pub(super) revision: i64,
}

#[derive(Insertable)]
#[diesel(table_name = memory_spaces)]
pub(super) struct NewMemorySpace<'a> {
    pub(super) id: &'a str,
    pub(super) scope: &'a str,
    pub(super) project_id: Option<&'a str>,
    pub(super) next_ordinal: i64,
    pub(super) revision: i64,
    pub(super) created_at: &'a str,
    pub(super) updated_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = memory_entries)]
pub(super) struct MemoryEntryRow {
    pub(super) id: String,
    pub(super) ordinal: i64,
    pub(super) content: String,
    pub(super) content_hash: String,
    pub(super) agent: String,
    pub(super) session_id: String,
    pub(super) model_id: Option<String>,
    pub(super) config_hash: String,
    pub(super) created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = memory_entries)]
pub(super) struct NewMemoryEntry<'a> {
    pub(super) id: &'a str,
    pub(super) space_id: &'a str,
    pub(super) ordinal: i64,
    pub(super) content: &'a str,
    pub(super) content_hash: &'a str,
    pub(super) agent: &'a str,
    pub(super) session_id: &'a str,
    pub(super) model_id: Option<&'a str>,
    pub(super) config_hash: &'a str,
    pub(super) created_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = memory_summaries)]
pub(super) struct MemorySummaryRow {
    pub(super) id: String,
    pub(super) space_id: String,
    pub(super) level: i32,
    pub(super) start_ordinal: i64,
    pub(super) end_ordinal: i64,
    pub(super) content: String,
    pub(super) source_hash: String,
    pub(super) provider: String,
    pub(super) model_id: Option<String>,
    pub(super) created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = memory_summaries)]
pub(super) struct NewMemorySummary<'a> {
    pub(super) id: &'a str,
    pub(super) space_id: &'a str,
    pub(super) level: i32,
    pub(super) start_ordinal: i64,
    pub(super) end_ordinal: i64,
    pub(super) content: &'a str,
    pub(super) content_hash: &'a str,
    pub(super) source_hash: &'a str,
    pub(super) provider: &'a str,
    pub(super) model_id: Option<&'a str>,
    pub(super) created_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = memory_frontier_snapshots)]
pub(super) struct MemoryFrontierSnapshotRow {
    pub(super) space_id: String,
    pub(super) revision: i64,
    pub(super) node_refs_json: String,
    pub(super) omissions_json: String,
    pub(super) item_count: i32,
    pub(super) omission_count: i32,
    pub(super) byte_count: i32,
    pub(super) built_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = memory_frontier_snapshots)]
pub(super) struct NewMemoryFrontierSnapshot<'a> {
    pub(super) space_id: &'a str,
    pub(super) revision: i64,
    pub(super) node_refs_json: &'a str,
    pub(super) omissions_json: &'a str,
    pub(super) item_count: i32,
    pub(super) omission_count: i32,
    pub(super) byte_count: i32,
    pub(super) built_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = memory_compaction_queue)]
pub(super) struct MemoryCompactionQueueRow {
    pub(super) space_id: String,
    pub(super) level: i32,
    pub(super) start_ordinal: i64,
    pub(super) end_ordinal: i64,
    pub(super) source_hash: String,
}

#[derive(Insertable)]
#[diesel(table_name = memory_compaction_queue)]
pub(super) struct NewMemoryCompactionQueue<'a> {
    pub(super) space_id: &'a str,
    pub(super) level: i32,
    pub(super) start_ordinal: i64,
    pub(super) end_ordinal: i64,
    pub(super) source_hash: &'a str,
    pub(super) codex_retry_at: Option<&'a str>,
    pub(super) claude_retry_at: Option<&'a str>,
    pub(super) queued_at: &'a str,
}

#[derive(Insertable)]
#[diesel(table_name = memory_activations)]
pub(super) struct NewMemoryActivation<'a> {
    pub(super) id: &'a str,
    pub(super) agent: &'a str,
    pub(super) session_id: &'a str,
    pub(super) read_scope: &'a str,
    pub(super) project_id: Option<&'a str>,
    pub(super) delivered_items: i32,
    pub(super) omitted_ranges: i32,
    pub(super) activated_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = memory_compaction_attempts)]
pub(super) struct MemoryCompactionAttemptRow {
    pub(super) provider: String,
    pub(super) model_id: Option<String>,
    pub(super) outcome: String,
    pub(super) consecutive_failures: i32,
    pub(super) attempted_at: String,
    pub(super) next_retry_at: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = memory_compaction_attempts)]
pub(super) struct NewMemoryCompactionAttempt<'a> {
    pub(super) id: &'a str,
    pub(super) space_id: &'a str,
    pub(super) level: i32,
    pub(super) start_ordinal: i64,
    pub(super) end_ordinal: i64,
    pub(super) source_hash: &'a str,
    pub(super) provider: &'a str,
    pub(super) model_id: Option<&'a str>,
    pub(super) outcome: &'a str,
    pub(super) diagnostic: Option<&'a str>,
    pub(super) was_fallback: bool,
    pub(super) consecutive_failures: i32,
    pub(super) attempted_at: &'a str,
    pub(super) next_retry_at: Option<&'a str>,
}
