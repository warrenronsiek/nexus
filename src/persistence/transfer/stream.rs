// @feature persistence
// @spec docs/features/persistence.md
// @entrypoint Store::write_streamed_snapshot
// @entrypoint Store::import_streamed_snapshot
// @boundary portable-database-stream
use super::super::backend::DbConnection;
use super::super::schema::{
    advisories, analytics_actions, analytics_calls, analytics_categories, analytics_exposures,
    analytics_resources, analytics_revisions, capability_uses, claims, conflicts, database_imports,
    events, memory_activations, memory_compaction_attempts, memory_compaction_queue,
    memory_entries, memory_frontier_snapshots, memory_spaces, memory_summaries,
    papercut_duplicate_reviews, papercut_review_attempts, papercut_upvotes, papercuts, sessions,
    sos_issue_lifecycle, sos_issues, sos_reports, sos_triage_attempts,
};
use super::super::Store;
use super::rows::*;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

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

const FORMAT_VERSION: i32 = 3;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;

#[cfg(feature = "sqlite")]
type LoadingMode = diesel::connection::DefaultLoadingMode;
#[cfg(feature = "postgres")]
type LoadingMode = diesel::pg::PgRowByRowLoadingMode;

/// Content identity and number of application rows in one complete portable stream.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct SnapshotManifest {
    #[serde(skip)]
    format_version: i32,
    pub rows: usize,
    pub digest: String,
}

macro_rules! batches {
    ($( $ordinal:literal: $variant:ident($row:ident) => $table:ident, $order:expr; )+) => {
        #[derive(Serialize, Deserialize)]
        #[serde(tag = "table", content = "rows", rename_all = "snake_case", deny_unknown_fields)]
        enum Batch { $( $variant(Vec<$row>), )+ }

        impl Batch {
            fn ordinal(&self) -> usize { match self { $(Self::$variant(_) => $ordinal,)+ } }
            fn len(&self) -> usize { match self { $(Self::$variant(rows) => rows.len(),)+ } }
            fn insert(&self, connection: &mut DbConnection) -> Result<()> {
                match self { $(Self::$variant(rows) => {
                    for chunk in rows.chunks(INSERT_CHUNK_ROWS) {
                        diesel::insert_into($table::table).values(chunk).execute(connection)?;
                    }
                },)+ }
                Ok(())
            }
        }

        fn export(connection: &mut DbConnection, writer: &mut StreamWriter<impl Write>) -> Result<()> {
            $( export_table!(connection, writer, $variant, $row, $table, $order); )+
            Ok(())
        }
        fn target_has_application_data(connection: &mut DbConnection) -> QueryResult<bool> {
            $(
                if diesel::select(diesel::dsl::exists($table::table.select($table::all_columns)))
                    .get_result::<bool>(connection)? {
                    return Ok(true);
                }
            )+
            Ok(false)
        }
        const TABLE_COUNT: usize = [$(stringify!($table)),+].len();
    }
}

macro_rules! export_table {
    ($connection:expr, $writer:expr, Papercuts, $row:ident, $table:ident, $order:expr) => {
        export_papercuts($connection, $writer)?;
    };
    ($connection:expr, $writer:expr, $variant:ident, $row:ident, $table:ident, $order:expr) => {{
        let mut cursor = $table::table
            .order($order)
            .load_iter::<$row, LoadingMode>($connection)?;
        let mut emitted = false;
        loop {
            let rows = cursor
                .by_ref()
                .take(INSERT_CHUNK_ROWS)
                .collect::<QueryResult<Vec<$row>>>()?;
            if rows.is_empty() && emitted {
                break;
            }
            let done = rows.len() < INSERT_CHUNK_ROWS;
            $writer.batch(Batch::$variant(rows))?;
            emitted = true;
            if done {
                break;
            }
        }
    }};
}

// Dependency order is part of the portable stream, so import never buffers a table.
batches! {
    0: Sessions(SessionTransfer) => sessions, (sessions::project_id.asc(), sessions::session_id.asc());
    1: Claims(ClaimTransfer) => claims, claims::id.asc();
    2: Conflicts(ConflictTransfer) => conflicts, conflicts::id.asc();
    3: Advisories(AdvisoryTransfer) => advisories, advisories::id.asc();
    4: CapabilityUses(CapabilityUseTransfer) => capability_uses, capability_uses::id.asc();
    5: Events(EventTransfer) => events, events::id.asc();
    6: MemorySpaces(MemorySpaceTransfer) => memory_spaces, memory_spaces::id.asc();
    7: MemoryEntries(MemoryEntryTransfer) => memory_entries, memory_entries::id.asc();
    8: MemorySummaries(MemorySummaryTransfer) => memory_summaries, memory_summaries::id.asc();
    9: MemoryFrontierSnapshots(MemoryFrontierTransfer) => memory_frontier_snapshots, memory_frontier_snapshots::space_id.asc();
    10: MemoryCompactionQueue(MemoryCompactionQueueTransfer) => memory_compaction_queue, (memory_compaction_queue::space_id.asc(), memory_compaction_queue::level.asc(), memory_compaction_queue::start_ordinal.asc(), memory_compaction_queue::end_ordinal.asc());
    11: MemoryActivations(MemoryActivationTransfer) => memory_activations, memory_activations::id.asc();
    12: MemoryCompactionAttempts(MemoryCompactionAttemptTransfer) => memory_compaction_attempts, memory_compaction_attempts::id.asc();
    13: Papercuts(PapercutTransfer) => papercuts, papercuts::id.asc();
    14: PapercutUpvotes(PapercutUpvoteTransfer) => papercut_upvotes, papercut_upvotes::id.asc();
    15: PapercutDuplicateReviews(PapercutDuplicateReviewTransfer) => papercut_duplicate_reviews, papercut_duplicate_reviews::id.asc();
    16: PapercutReviewAttempts(PapercutReviewAttemptTransfer) => papercut_review_attempts, papercut_review_attempts::id.asc();
    17: SosIssues(SosIssueTransfer) => sos_issues, sos_issues::id.asc();
    18: SosReports(SosReportTransfer) => sos_reports, sos_reports::id.asc();
    19: SosTriageAttempts(SosTriageAttemptTransfer) => sos_triage_attempts, sos_triage_attempts::id.asc();
    20: SosIssueLifecycle(SosIssueLifecycleTransfer) => sos_issue_lifecycle, sos_issue_lifecycle::id.asc();
    21: AnalyticsCalls(AnalyticsCallsTransfer) => analytics_calls, analytics_calls::id.asc();
    22: AnalyticsActions(AnalyticsActionsTransfer) => analytics_actions, analytics_actions::id.asc();
    23: AnalyticsExposures(AnalyticsExposuresTransfer) => analytics_exposures, analytics_exposures::id.asc();
    24: AnalyticsCategories(AnalyticsCategoriesTransfer) => analytics_categories, analytics_categories::id.asc();
    25: AnalyticsRevisions(AnalyticsRevisionsTransfer) => analytics_revisions, analytics_revisions::id.asc();
    26: AnalyticsResources(AnalyticsResourcesTransfer) => analytics_resources, analytics_resources::id.asc();
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    Header { format_version: i32 },
    Batch { data: Batch },
    Complete { rows: usize },
}

struct StreamWriter<W> {
    output: W,
    hash: blake3::Hasher,
    bytes: u64,
    rows: usize,
}

impl<W: Write> StreamWriter<W> {
    fn new(output: W) -> Self {
        Self {
            output,
            hash: blake3::Hasher::new(),
            bytes: 0,
            rows: 0,
        }
    }

    fn record(&mut self, record: &Record) -> Result<()> {
        let mut bytes = serde_json::to_vec(record)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_RECORD_BYTES {
            bail!("portable stream record exceeds the {MAX_RECORD_BYTES}-byte limit");
        }
        self.bytes += bytes.len() as u64;
        if self.bytes > MAX_FILE_BYTES {
            bail!("portable stream exceeds the {MAX_FILE_BYTES}-byte limit");
        }
        self.output.write_all(&bytes)?;
        self.hash.update(&bytes);
        Ok(())
    }

    fn batch(&mut self, data: Batch) -> Result<()> {
        self.rows += data.len();
        self.record(&Record::Batch { data })
    }

    fn finish(mut self) -> Result<SnapshotManifest> {
        self.record(&Record::Complete { rows: self.rows })?;
        self.output.flush()?;
        Ok(SnapshotManifest {
            format_version: FORMAT_VERSION,
            rows: self.rows,
            digest: self.hash.finalize().to_hex().to_string(),
        })
    }
}

impl Store {
    /// Exports a deterministic record stream while retaining at most one ordinary batch in memory.
    pub fn write_streamed_snapshot(&mut self, output: impl Write) -> Result<SnapshotManifest> {
        self.connection.transaction(|connection| {
            let mut writer = StreamWriter::new(output);
            writer.record(&Record::Header {
                format_version: FORMAT_VERSION,
            })?;
            export(connection, &mut writer)?;
            writer.finish()
        })
    }

    /// Imports a complete stream atomically, refusing a different source or unmarked target data.
    pub fn import_streamed_snapshot(&mut self, path: &Path) -> Result<ImportOutcome> {
        let source = SnapshotSource::open(path)?;
        self.connection.transaction(|connection| {
            match existing_import_outcome(connection, &source.digest)? {
                Some(outcome) => Ok(outcome),
                None => import_new_target(connection, source),
            }
        })
    }
}

struct SnapshotSource {
    file: File,
    digest: String,
}

impl SnapshotSource {
    fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).context("open portable stream")?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            bail!("portable stream must be a regular file of at most {MAX_FILE_BYTES} bytes");
        }
        Self::fingerprint(file)
    }

    fn fingerprint(mut file: File) -> Result<Self> {
        let mut hash = blake3::Hasher::new();
        std::io::copy(&mut (&mut file).take(MAX_FILE_BYTES + 1), &mut hash)?;
        let digest = hash.finalize().to_hex().to_string();
        file.seek(SeekFrom::Start(0))?;
        Ok(Self { file, digest })
    }
}

fn import_new_target(
    connection: &mut DbConnection,
    source: SnapshotSource,
) -> Result<ImportOutcome> {
    if target_has_application_data(connection)? {
        bail!("target database is not empty; refusing to merge a SQLite snapshot");
    }
    let manifest = import(connection, BufReader::new(source.file))?;
    record_import(connection, manifest, source.digest)
}

fn record_import(
    connection: &mut DbConnection,
    manifest: SnapshotManifest,
    digest: String,
) -> Result<ImportOutcome> {
    if manifest.digest != digest {
        bail!("portable stream changed during import");
    }
    let rows = manifest.rows;
    let row_count = i64::try_from(rows).context("portable snapshot row count exceeds i64")?;
    diesel::insert_into(database_imports::table)
        .values((
            database_imports::source_digest.eq(&digest),
            database_imports::source_kind.eq("sqlite"),
            database_imports::format_version.eq(manifest.format_version),
            database_imports::row_count.eq(row_count),
            database_imports::imported_at.eq(Utc::now().to_rfc3339()),
        ))
        .execute(connection)?;
    Ok(ImportOutcome::Imported { rows, digest })
}

#[derive(Default)]
struct ImportProgress {
    format_version: i32,
    rows: usize,
    seen: u32,
    bytes: u64,
    last_table: usize,
    maximum_event_id: i64,
}

impl ImportProgress {
    fn accept_batch(&mut self, data: Batch, connection: &mut DbConnection) -> Result<()> {
        let table = data.ordinal();
        if table < self.last_table || data.len() > INSERT_CHUNK_ROWS {
            bail!("invalid portable stream batch order or size");
        }
        self.last_table = table;
        self.seen |= 1 << table;
        if let Batch::Events(events) = &data {
            self.maximum_event_id = self
                .maximum_event_id
                .max(events.iter().map(|event| event.id).max().unwrap_or(0));
        }
        data.insert(connection)?;
        self.rows += data.len();
        Ok(())
    }
}

fn import(connection: &mut DbConnection, mut input: impl BufRead) -> Result<SnapshotManifest> {
    let mut hash = blake3::Hasher::new();
    let mut progress = ImportProgress::default();
    let header = read_record(&mut input, &mut hash, &mut progress.bytes)?;
    let tables = match header {
        Some(Record::Header { format_version: 2 }) => {
            progress.format_version = 2;
            21
        }
        Some(Record::Header {
            format_version: FORMAT_VERSION,
        }) => {
            progress.format_version = FORMAT_VERSION;
            TABLE_COUNT
        }
        _ => bail!("unsupported or missing portable stream header"),
    };
    loop {
        match read_record(&mut input, &mut hash, &mut progress.bytes)? {
            Some(Record::Batch { data }) => progress.accept_batch(data, connection)?,
            Some(Record::Complete { rows: expected })
                if expected == progress.rows && progress.seen == (1 << tables) - 1 =>
            {
                return finish_import(connection, &mut input, hash, progress);
            }
            _ => bail!("incomplete or invalid portable stream"),
        }
    }
}

fn finish_import(
    connection: &mut DbConnection,
    input: &mut impl BufRead,
    mut hash: blake3::Hasher,
    mut progress: ImportProgress,
) -> Result<SnapshotManifest> {
    if read_record(input, &mut hash, &mut progress.bytes)?.is_some() {
        bail!("portable stream has trailing records");
    }
    sync_event_identity(connection, progress.maximum_event_id)?;
    Ok(SnapshotManifest {
        format_version: progress.format_version,
        rows: progress.rows,
        digest: hash.finalize().to_hex().to_string(),
    })
}

fn read_record(
    input: &mut impl BufRead,
    hash: &mut blake3::Hasher,
    total: &mut u64,
) -> Result<Option<Record>> {
    let mut line = Vec::new();
    let length = input
        .take((MAX_RECORD_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)?;
    if length == 0 {
        return Ok(None);
    }
    *total += length as u64;
    if length > MAX_RECORD_BYTES || *total > MAX_FILE_BYTES || line.last() != Some(&b'\n') {
        bail!("portable stream exceeds its bounds or contains an incomplete record");
    }
    hash.update(&line);
    Ok(Some(
        serde_json::from_slice(&line).context("decode portable stream record")?,
    ))
}

fn export_papercuts(
    connection: &mut DbConnection,
    writer: &mut StreamWriter<impl Write>,
) -> Result<()> {
    let mut emitted = HashSet::new();
    let mut visiting = HashSet::new();
    let mut after = String::new();
    loop {
        let ids = papercuts::table
            .filter(papercuts::id.gt(&after))
            .select(papercuts::id)
            .order(papercuts::id.asc())
            .limit(INSERT_CHUNK_ROWS as i64)
            .load::<String>(connection)?;
        if ids.is_empty() {
            break;
        }
        for id in &ids {
            export_papercut(id, connection, writer, &mut emitted, &mut visiting)?;
        }
        after = ids.last().unwrap().clone();
    }
    if emitted.is_empty() {
        writer.batch(Batch::Papercuts(Vec::new()))?;
    }
    Ok(())
}

fn export_papercut(
    id: &str,
    connection: &mut DbConnection,
    writer: &mut StreamWriter<impl Write>,
    emitted: &mut HashSet<String>,
    visiting: &mut HashSet<String>,
) -> Result<()> {
    if emitted.contains(id) {
        return Ok(());
    }
    if visiting.len() >= 1024 || !visiting.insert(id.to_owned()) {
        bail!("cyclic or excessively deep papercut consolidation graph");
    }
    let row = papercuts::table
        .find(id)
        .first::<PapercutTransfer>(connection)?;
    if let Some(target) = &row.consolidated_into {
        export_papercut(target, connection, writer, emitted, visiting)?;
    }
    writer.batch(Batch::Papercuts(vec![row]))?;
    visiting.remove(id);
    emitted.insert(id.to_owned());
    Ok(())
}

#[cfg(all(test, feature = "sqlite"))]
#[path = "stream_tests.rs"]
mod tests;
