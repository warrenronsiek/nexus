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
// @entrypoint Store::write_streamed_snapshot
// @entrypoint Store::import_streamed_snapshot
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

mod rows;
mod stream;
pub use stream::SnapshotManifest;
#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::{backup_sqlite_database, export_sqlite_stream};

const INSERT_CHUNK_ROWS: usize = 16;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ImportOutcome {
    Imported { rows: usize, digest: String },
    AlreadyImported { rows: usize, digest: String },
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

#[cfg(feature = "sqlite")]
fn sync_event_identity(_connection: &mut DbConnection, _imported_max: i64) -> Result<()> {
    Ok(())
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

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::rows::PapercutTransfer;
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

        let mut store = Store::open_memory().unwrap();
        for row in [&canonical, &source] {
            diesel::insert_into(papercuts::table)
                .values(row)
                .execute(&mut store.connection)
                .unwrap();
        }
        let snapshot = tempfile::NamedTempFile::new().unwrap();
        let manifest = store.write_streamed_snapshot(snapshot.as_file()).unwrap();
        let mut target = Store::open_memory().unwrap();
        assert!(matches!(
            target.import_streamed_snapshot(snapshot.path()).unwrap(),
            ImportOutcome::Imported { rows: 2, .. }
        ));
        assert_eq!(manifest.rows, 2);
        let registry = target.papercut_registry(Some("project-a")).unwrap();
        assert_eq!(registry.papercuts.len(), 2);
        let copied_source = registry
            .papercuts
            .iter()
            .find(|row| row.id == "000-source")
            .unwrap();
        assert_eq!(
            copied_source.consolidated_into.as_deref(),
            Some("zzz-canonical")
        );
    }
}
