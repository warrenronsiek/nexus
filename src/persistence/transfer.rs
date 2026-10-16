// @feature persistence
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @entrypoint Store::portable_snapshot
// @entrypoint Store::import_snapshot
// @boundary portable-database-snapshot
use super::backend::DbConnection;
use super::schema::{
    advisories, capability_uses, claims, conflicts, database_imports, events, memory_activations,
    memory_compaction_attempts, memory_compaction_queue, memory_entries, memory_frontier_snapshots,
    memory_spaces, memory_summaries, papercut_duplicate_reviews, papercut_review_attempts,
    papercut_upvotes, papercuts, sessions, sos_issue_lifecycle, sos_issues, sos_reports,
    sos_triage_attempts,
};
use super::Store;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod codec;
mod rows;
mod stream;
use rows::*;
pub use stream::SnapshotManifest;
#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::{backup_sqlite_database, export_sqlite_stream, snapshot_sqlite_database};

const PORTABLE_SNAPSHOT_VERSION: u32 = 1;
const INSERT_CHUNK_ROWS: usize = 16;

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortableSnapshot {
    format_version: u32,
    advisories: Vec<AdvisoryTransfer>,
    capability_uses: Vec<CapabilityUseTransfer>,
    claims: Vec<ClaimTransfer>,
    conflicts: Vec<ConflictTransfer>,
    events: Vec<EventTransfer>,
    memory_activations: Vec<MemoryActivationTransfer>,
    memory_compaction_attempts: Vec<MemoryCompactionAttemptTransfer>,
    memory_compaction_queue: Vec<MemoryCompactionQueueTransfer>,
    memory_entries: Vec<MemoryEntryTransfer>,
    memory_frontier_snapshots: Vec<MemoryFrontierTransfer>,
    memory_spaces: Vec<MemorySpaceTransfer>,
    memory_summaries: Vec<MemorySummaryTransfer>,
    papercut_duplicate_reviews: Vec<PapercutDuplicateReviewTransfer>,
    papercut_review_attempts: Vec<PapercutReviewAttemptTransfer>,
    papercut_upvotes: Vec<PapercutUpvoteTransfer>,
    papercuts: Vec<PapercutTransfer>,
    sessions: Vec<SessionTransfer>,
    sos_issue_lifecycle: Vec<SosIssueLifecycleTransfer>,
    sos_issues: Vec<SosIssueTransfer>,
    sos_reports: Vec<SosReportTransfer>,
    sos_triage_attempts: Vec<SosTriageAttemptTransfer>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ImportOutcome {
    Imported { rows: usize, digest: String },
    AlreadyImported { rows: usize, digest: String },
}

impl PortableSnapshot {
    pub fn row_count(&self) -> usize {
        self.advisories.len()
            + self.capability_uses.len()
            + self.claims.len()
            + self.conflicts.len()
            + self.events.len()
            + self.memory_activations.len()
            + self.memory_compaction_attempts.len()
            + self.memory_compaction_queue.len()
            + self.memory_entries.len()
            + self.memory_frontier_snapshots.len()
            + self.memory_spaces.len()
            + self.memory_summaries.len()
            + self.papercut_duplicate_reviews.len()
            + self.papercut_review_attempts.len()
            + self.papercut_upvotes.len()
            + self.papercuts.len()
            + self.sessions.len()
            + self.sos_issue_lifecycle.len()
            + self.sos_issues.len()
            + self.sos_reports.len()
            + self.sos_triage_attempts.len()
    }
}

impl Store {
    pub fn portable_snapshot(&mut self) -> Result<PortableSnapshot> {
        self.connection
            .transaction(snapshot_transaction)
            .context("export portable database snapshot")
    }

    pub fn import_snapshot(
        &mut self,
        snapshot: &PortableSnapshot,
        expected_digest: &str,
    ) -> Result<ImportOutcome> {
        snapshot.validate()?;
        let digest = snapshot.digest()?;
        if expected_digest != digest {
            bail!("portable snapshot digest does not match its contents");
        }
        let rows = snapshot.row_count();
        let outcome = self
            .connection
            .transaction(|connection| {
                let outcome = import_transaction(connection, snapshot, &digest, rows)?;
                sync_event_identity(
                    connection,
                    snapshot
                        .events
                        .iter()
                        .map(|event| event.id)
                        .max()
                        .unwrap_or(0),
                )?;
                Ok::<_, anyhow::Error>(outcome)
            })
            .context("import portable database snapshot")?;
        Ok(outcome)
    }
}

macro_rules! load_ordered {
    ($connection:expr, $table:ident, $order:expr) => {
        $table::table.order($order).load($connection)?
    };
}

macro_rules! load_snapshot {
    ($connection:expr) => {
        PortableSnapshot {
            format_version: PORTABLE_SNAPSHOT_VERSION,
            advisories: load_ordered!($connection, advisories, advisories::id.asc()),
            capability_uses: load_ordered!($connection, capability_uses, capability_uses::id.asc()),
            claims: load_ordered!($connection, claims, claims::id.asc()),
            conflicts: load_ordered!($connection, conflicts, conflicts::id.asc()),
            events: load_ordered!($connection, events, events::id.asc()),
            memory_activations: load_ordered!(
                $connection,
                memory_activations,
                memory_activations::id.asc()
            ),
            memory_compaction_attempts: load_ordered!(
                $connection,
                memory_compaction_attempts,
                memory_compaction_attempts::id.asc()
            ),
            memory_compaction_queue: load_ordered!(
                $connection,
                memory_compaction_queue,
                (
                    memory_compaction_queue::space_id.asc(),
                    memory_compaction_queue::level.asc(),
                    memory_compaction_queue::start_ordinal.asc(),
                    memory_compaction_queue::end_ordinal.asc(),
                )
            ),
            memory_entries: load_ordered!($connection, memory_entries, memory_entries::id.asc()),
            memory_frontier_snapshots: load_ordered!(
                $connection,
                memory_frontier_snapshots,
                memory_frontier_snapshots::space_id.asc()
            ),
            memory_spaces: load_ordered!($connection, memory_spaces, memory_spaces::id.asc()),
            memory_summaries: load_ordered!(
                $connection,
                memory_summaries,
                memory_summaries::id.asc()
            ),
            papercut_duplicate_reviews: load_ordered!(
                $connection,
                papercut_duplicate_reviews,
                papercut_duplicate_reviews::id.asc()
            ),
            papercut_review_attempts: load_ordered!(
                $connection,
                papercut_review_attempts,
                papercut_review_attempts::id.asc()
            ),
            papercut_upvotes: load_ordered!(
                $connection,
                papercut_upvotes,
                papercut_upvotes::id.asc()
            ),
            papercuts: load_ordered!($connection, papercuts, papercuts::id.asc()),
            sessions: load_ordered!(
                $connection,
                sessions,
                (sessions::project_id.asc(), sessions::session_id.asc())
            ),
            sos_issue_lifecycle: load_ordered!(
                $connection,
                sos_issue_lifecycle,
                sos_issue_lifecycle::id.asc()
            ),
            sos_issues: load_ordered!($connection, sos_issues, sos_issues::id.asc()),
            sos_reports: load_ordered!($connection, sos_reports, sos_reports::id.asc()),
            sos_triage_attempts: load_ordered!(
                $connection,
                sos_triage_attempts,
                sos_triage_attempts::id.asc()
            ),
        }
    };
}

fn snapshot_transaction(connection: &mut DbConnection) -> QueryResult<PortableSnapshot> {
    Ok(load_snapshot!(connection))
}

fn import_transaction(
    connection: &mut DbConnection,
    snapshot: &PortableSnapshot,
    digest: &str,
    rows: usize,
) -> Result<ImportOutcome> {
    match existing_import_outcome(connection, digest)? {
        Some(outcome) => Ok(outcome),
        None => import_into_empty_target(connection, snapshot, digest, rows),
    }
}

fn existing_import_outcome(
    connection: &mut DbConnection,
    digest: &str,
) -> Result<Option<ImportOutcome>> {
    let previous = database_imports::table
        .select((database_imports::source_digest, database_imports::row_count))
        .load::<(String, i64)>(connection)?;
    match previous.as_slice() {
        [] => Ok(None),
        [(previous_digest, rows)] if previous_digest == digest => {
            Ok(Some(ImportOutcome::AlreadyImported {
                rows: usize::try_from(*rows)?,
                digest: digest.to_owned(),
            }))
        }
        _ => bail!("target database was already initialized from a different SQLite snapshot"),
    }
}

fn import_into_empty_target(
    connection: &mut DbConnection,
    snapshot: &PortableSnapshot,
    digest: &str,
    rows: usize,
) -> Result<ImportOutcome> {
    if target_has_application_data(connection)? {
        bail!("target database is not empty; refusing to merge a SQLite snapshot");
    }

    insert_snapshot(connection, snapshot)?;
    record_import_marker(connection, digest, PORTABLE_SNAPSHOT_VERSION as i32, rows)
}

fn record_import_marker(
    connection: &mut DbConnection,
    digest: &str,
    version: i32,
    rows: usize,
) -> Result<ImportOutcome> {
    let row_count = i64::try_from(rows).context("portable snapshot row count exceeds i64")?;
    diesel::insert_into(database_imports::table)
        .values((
            database_imports::source_digest.eq(digest),
            database_imports::source_kind.eq("sqlite"),
            database_imports::format_version.eq(version),
            database_imports::row_count.eq(row_count),
            database_imports::imported_at.eq(Utc::now().to_rfc3339()),
        ))
        .execute(connection)?;
    Ok(ImportOutcome::Imported {
        rows,
        digest: digest.to_owned(),
    })
}

macro_rules! table_has_rows {
    ($connection:expr, $table:ident, $column:ident) => {
        $table::table
            .select($table::$column)
            .first::<String>($connection)
            .optional()?
            .is_some()
    };
}

fn target_has_application_data(connection: &mut DbConnection) -> QueryResult<bool> {
    let text_tables_have_rows = [
        table_has_rows!(connection, advisories, id),
        table_has_rows!(connection, capability_uses, id),
        table_has_rows!(connection, claims, id),
        table_has_rows!(connection, conflicts, id),
        table_has_rows!(connection, memory_activations, id),
        table_has_rows!(connection, memory_compaction_attempts, id),
        table_has_rows!(connection, memory_compaction_queue, space_id),
        table_has_rows!(connection, memory_entries, id),
        table_has_rows!(connection, memory_frontier_snapshots, space_id),
        table_has_rows!(connection, memory_spaces, id),
        table_has_rows!(connection, memory_summaries, id),
        table_has_rows!(connection, papercut_duplicate_reviews, id),
        table_has_rows!(connection, papercut_review_attempts, id),
        table_has_rows!(connection, papercut_upvotes, id),
        table_has_rows!(connection, papercuts, id),
        table_has_rows!(connection, sessions, session_id),
        table_has_rows!(connection, sos_issue_lifecycle, id),
        table_has_rows!(connection, sos_issues, id),
        table_has_rows!(connection, sos_reports, id),
        table_has_rows!(connection, sos_triage_attempts, id),
    ];
    let events_have_rows = events::table
        .select(events::id)
        .first::<i64>(connection)
        .optional()?
        .is_some();
    Ok(events_have_rows || text_tables_have_rows.into_iter().any(|has_rows| has_rows))
}

macro_rules! insert_rows {
    ($connection:expr, $table:ident, $rows:expr) => {
        for chunk in $rows.chunks(INSERT_CHUNK_ROWS) {
            diesel::insert_into($table::table)
                .values(chunk)
                .execute($connection)?;
        }
    };
}

fn insert_snapshot(connection: &mut DbConnection, snapshot: &PortableSnapshot) -> Result<()> {
    insert_rows!(connection, sessions, snapshot.sessions);
    insert_rows!(connection, claims, snapshot.claims);
    insert_rows!(connection, conflicts, snapshot.conflicts);
    insert_rows!(connection, advisories, snapshot.advisories);
    insert_rows!(connection, capability_uses, snapshot.capability_uses);
    insert_rows!(connection, events, snapshot.events);
    insert_rows!(connection, memory_spaces, snapshot.memory_spaces);
    insert_rows!(connection, memory_entries, snapshot.memory_entries);
    insert_rows!(connection, memory_summaries, snapshot.memory_summaries);
    insert_rows!(
        connection,
        memory_frontier_snapshots,
        snapshot.memory_frontier_snapshots
    );
    insert_rows!(
        connection,
        memory_compaction_queue,
        snapshot.memory_compaction_queue
    );
    insert_rows!(connection, memory_activations, snapshot.memory_activations);
    insert_rows!(
        connection,
        memory_compaction_attempts,
        snapshot.memory_compaction_attempts
    );
    let ordered_papercuts = papercuts_in_dependency_order(&snapshot.papercuts)?;
    insert_rows!(connection, papercuts, ordered_papercuts);
    insert_rows!(connection, papercut_upvotes, snapshot.papercut_upvotes);
    insert_rows!(
        connection,
        papercut_duplicate_reviews,
        snapshot.papercut_duplicate_reviews
    );
    insert_rows!(
        connection,
        papercut_review_attempts,
        snapshot.papercut_review_attempts
    );
    insert_rows!(connection, sos_issues, snapshot.sos_issues);
    insert_rows!(connection, sos_reports, snapshot.sos_reports);
    insert_rows!(
        connection,
        sos_triage_attempts,
        snapshot.sos_triage_attempts
    );
    insert_rows!(
        connection,
        sos_issue_lifecycle,
        snapshot.sos_issue_lifecycle
    );
    Ok(())
}

fn papercuts_in_dependency_order(rows: &[PapercutTransfer]) -> Result<Vec<PapercutTransfer>> {
    let indices = rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    if indices.len() != rows.len() {
        bail!("portable snapshot contains duplicate papercut identifiers");
    }
    let mut states = vec![0; rows.len()];
    let mut ordered = Vec::with_capacity(rows.len());
    for row_index in 0..rows.len() {
        visit_papercut(row_index, rows, &indices, &mut states, &mut ordered)?;
    }
    Ok(ordered)
}

fn visit_papercut(
    row_index: usize,
    rows: &[PapercutTransfer],
    indices: &HashMap<&str, usize>,
    states: &mut [u8],
    ordered: &mut Vec<PapercutTransfer>,
) -> Result<()> {
    match states[row_index] {
        2 => return Ok(()),
        1 => bail!(
            "papercut consolidation cycle includes {}",
            rows[row_index].id
        ),
        _ => {}
    }
    states[row_index] = 1;
    if let Some(target_id) = rows[row_index].consolidated_into.as_deref() {
        let target_index = indices.get(target_id).copied().with_context(|| {
            format!(
                "papercut {} references missing consolidation target {target_id}",
                rows[row_index].id
            )
        })?;
        visit_papercut(target_index, rows, indices, states, ordered)?;
    }
    states[row_index] = 2;
    ordered.push(rows[row_index].clone());
    Ok(())
}

#[cfg(feature = "sqlite")]
fn sync_event_identity(_connection: &mut DbConnection, _imported_max: i64) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn papercut(id: &str, consolidated_into: Option<&str>) -> PapercutTransfer {
        PapercutTransfer {
            id: id.to_owned(),
            project_id: "project-a".to_owned(),
            title: format!("issue {id}"),
            description: String::new(),
            fingerprint: "a".repeat(64),
            reporter_token_hash: "b".repeat(64),
            status: if consolidated_into.is_some() {
                "consolidated".to_owned()
            } else {
                "open".to_owned()
            },
            consolidated_into: consolidated_into.map(str::to_owned),
            resolution: None,
            creator_agent: "agent".to_owned(),
            creator_session_id: "session".to_owned(),
            creator_model_id: None,
            creator_config_hash: "config".to_owned(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            resolved_at: None,
        }
    }

    #[test]
    fn consolidated_papercuts_are_inserted_after_their_canonical_dependencies() {
        let source = papercut("000-source", Some("zzz-canonical"));
        let canonical = papercut("zzz-canonical", None);

        let ordered = papercuts_in_dependency_order(&[source, canonical]).unwrap();

        assert_eq!(
            ordered
                .iter()
                .map(|papercut| papercut.id.as_str())
                .collect::<Vec<_>>(),
            ["zzz-canonical", "000-source"]
        );
    }
}

#[cfg(feature = "postgres")]
fn sync_event_identity(connection: &mut DbConnection, imported_max: i64) -> Result<()> {
    use diesel::sql_types::BigInt;
    diesel::define_sql_function! {
        fn nexus_sync_event_identity(imported_max: BigInt) -> BigInt;
    }
    diesel::select(nexus_sync_event_identity(imported_max))
        .get_result::<i64>(connection)
        .context("synchronize PostgreSQL event identity after import")?;
    Ok(())
}
