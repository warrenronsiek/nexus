// @feature persistence
// @spec docs/features/persistence.md
// @entrypoint export_sqlite_stream
// @boundary SQLite online-backup FFI
use super::Store;
use anyhow::{bail, Context, Result};
use libsqlite3_sys as sqlite;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::time::{Duration, Instant};

const MAX_SQLITE_IMPORT_SOURCE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

pub fn export_sqlite_stream(
    source_path: &Path,
    output: impl std::io::Write,
) -> Result<super::SnapshotManifest> {
    ensure_sqlite_source(source_path)?;
    let copy = tempfile::NamedTempFile::new().context("create temporary SQLite snapshot")?;
    sqlite_online_backup(source_path, copy.path())?;
    let mut store = Store::open(copy.path()).context("migrate temporary SQLite snapshot")?;
    super::super::migrations::validate_portable_source_migrations(&mut store.connection)
        .context("validate SQLite source migration history")?;
    store.write_streamed_snapshot(output)
}

pub fn backup_sqlite_database(source_path: &Path, destination_path: &Path) -> Result<()> {
    ensure_sqlite_source(source_path)?;
    ensure_distinct_destination(source_path, destination_path)?;
    let copy = temporary_backup_beside(destination_path)?;
    sqlite_online_backup(source_path, copy.path())?;
    publish_backup(copy, destination_path)
}

fn publish_backup(copy: tempfile::NamedTempFile, destination_path: &Path) -> Result<()> {
    copy.as_file()
        .sync_all()
        .context("sync staged SQLite backup")?;
    copy.persist(destination_path)
        .map_err(|error| error.error)
        .with_context(|| {
            format!(
                "publish staged SQLite backup at {}",
                destination_path.display()
            )
        })?;
    Ok(())
}

fn temporary_backup_beside(destination_path: &Path) -> Result<tempfile::NamedTempFile> {
    let parent = destination_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tempfile::NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "create temporary SQLite backup beside {}",
            destination_path.display()
        )
    })
}

fn ensure_distinct_destination(source_path: &Path, destination_path: &Path) -> Result<()> {
    if destination_path.exists()
        && std::fs::canonicalize(source_path)? == std::fs::canonicalize(destination_path)?
    {
        bail!("SQLite backup destination must differ from its source");
    }
    Ok(())
}

fn ensure_sqlite_source(source_path: &Path) -> Result<()> {
    let metadata = std::fs::metadata(source_path)
        .with_context(|| format!("inspect SQLite import source {}", source_path.display()))?;
    if !metadata.is_file() {
        bail!(
            "SQLite import source is not a regular file: {}",
            source_path.display()
        );
    }
    ensure_sqlite_size(
        metadata.len(),
        "SQLite import source",
        MAX_SQLITE_IMPORT_SOURCE_BYTES,
    )?;
    Ok(())
}

fn ensure_sqlite_size(size: u64, description: &str, limit: u64) -> Result<()> {
    if size > limit {
        bail!("{description} exceeds the {limit}-byte limit");
    }
    Ok(())
}

fn sqlite_online_backup(source_path: &Path, destination_path: &Path) -> Result<()> {
    let source = SqliteConnection::open(
        source_path,
        sqlite::SQLITE_OPEN_READONLY,
        "SQLite import source",
    )?;
    let destination = SqliteConnection::open(
        destination_path,
        sqlite::SQLITE_OPEN_READWRITE,
        "temporary SQLite snapshot",
    )?;
    let backup = SqliteBackup::new(&destination, &source)?;
    backup.copy_to_completion()?;
    let copied_size = std::fs::metadata(destination_path)
        .with_context(|| format!("inspect SQLite backup {}", destination_path.display()))?
        .len();
    ensure_sqlite_size(
        copied_size,
        "SQLite import backup",
        MAX_SQLITE_IMPORT_SOURCE_BYTES,
    )
}

struct SqliteConnection {
    raw: *mut sqlite::sqlite3,
}

impl SqliteConnection {
    fn open(path: &Path, flags: i32, description: &str) -> Result<Self> {
        let name = CString::new(path.as_os_str().as_bytes())
            .with_context(|| format!("SQLite path contains a NUL byte: {}", path.display()))?;
        let mut raw = ptr::null_mut();
        // SAFETY: `name` outlives the call and `raw` points to initialized local storage.
        let code = unsafe { sqlite::sqlite3_open_v2(name.as_ptr(), &mut raw, flags, ptr::null()) };
        if code == sqlite::SQLITE_OK {
            return Ok(Self { raw });
        }
        let detail = sqlite_message(raw);
        close_sqlite(raw);
        bail!("open {description}: {detail}");
    }

    fn message(&self) -> String {
        sqlite_message(self.raw)
    }
}

impl Drop for SqliteConnection {
    fn drop(&mut self) {
        close_sqlite(self.raw);
    }
}

struct SqliteBackup<'connection> {
    raw: *mut sqlite::sqlite3_backup,
    destination: &'connection SqliteConnection,
    _source: &'connection SqliteConnection,
}

impl<'connection> SqliteBackup<'connection> {
    fn new(
        destination: &'connection SqliteConnection,
        source: &'connection SqliteConnection,
    ) -> Result<Self> {
        let main = c"main";
        // SAFETY: both connections remain borrowed for this backup's lifetime, and `main` is a
        // valid NUL-terminated schema name.
        let raw = unsafe {
            sqlite::sqlite3_backup_init(destination.raw, main.as_ptr(), source.raw, main.as_ptr())
        };
        if raw.is_null() {
            bail!("initialize SQLite online backup: {}", destination.message());
        }
        Ok(Self {
            raw,
            destination,
            _source: source,
        })
    }

    fn copy_to_completion(mut self) -> Result<()> {
        let transfer = self.step_until_complete();
        let finish = self.finish();
        transfer.and(finish)
    }

    fn step_until_complete(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            // SAFETY: `raw` remains live until `finish` or `drop`.
            match unsafe { sqlite::sqlite3_backup_step(self.raw, 128) } {
                sqlite::SQLITE_DONE => return Ok(()),
                sqlite::SQLITE_OK => {}
                sqlite::SQLITE_BUSY | sqlite::SQLITE_LOCKED if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                sqlite::SQLITE_BUSY | sqlite::SQLITE_LOCKED => {
                    bail!("SQLite import source remained busy for 10 seconds");
                }
                code => bail!(
                    "copy SQLite import source (error {code}): {}",
                    self.destination.message()
                ),
            }
        }
    }

    fn finish(&mut self) -> Result<()> {
        // SAFETY: `raw` is live and finish is called at most once because it is nulled here.
        let code = unsafe { sqlite::sqlite3_backup_finish(self.raw) };
        self.raw = ptr::null_mut();
        if code != sqlite::SQLITE_OK {
            bail!(
                "finish SQLite online backup: {}",
                self.destination.message()
            );
        }
        Ok(())
    }
}

impl Drop for SqliteBackup<'_> {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: this is the sole fallback owner of an unfinished backup handle.
            let _ = unsafe { sqlite::sqlite3_backup_finish(self.raw) };
        }
    }
}

fn sqlite_message(connection: *mut sqlite::sqlite3) -> String {
    if connection.is_null() {
        return "SQLite did not provide an error message".to_owned();
    }
    // SAFETY: SQLite owns this NUL-terminated message for the lifetime of the connection.
    unsafe { CStr::from_ptr(sqlite::sqlite3_errmsg(connection)) }
        .to_string_lossy()
        .into_owned()
}

fn close_sqlite(connection: *mut sqlite::sqlite3) {
    if !connection.is_null() {
        // SAFETY: each connection pointer is owned by one `SqliteConnection` or failed open.
        let _ = unsafe { sqlite::sqlite3_close(connection) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::schema::flyway_migrations;
    use diesel::prelude::*;

    #[test]
    fn snapshot_rejects_a_source_created_by_a_newer_nexus_schema() {
        let temporary = tempfile::tempdir().unwrap();
        let source_path = temporary.path().join("future.db");
        let mut store = Store::open(&source_path).unwrap();
        diesel::insert_into(flyway_migrations::table)
            .values((
                flyway_migrations::version.eq(9_999_i64),
                flyway_migrations::name.eq("future_schema"),
                flyway_migrations::checksum.eq("future-checksum"),
                flyway_migrations::status.eq("deployed"),
            ))
            .execute(&mut store.connection)
            .unwrap();
        drop(store);

        let error = export_sqlite_stream(&source_path, std::io::sink()).unwrap_err();

        assert!(
            format!("{error:#}").contains("unsupported SQLite migration version 9999"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn snapshot_rejects_a_source_with_divergent_migration_contents() {
        let temporary = tempfile::tempdir().unwrap();
        let source_path = temporary.path().join("divergent.db");
        let mut store = Store::open(&source_path).unwrap();
        diesel::update(flyway_migrations::table.filter(flyway_migrations::version.eq(10_i64)))
            .set(flyway_migrations::checksum.eq("divergent-checksum"))
            .execute(&mut store.connection)
            .unwrap();
        drop(store);

        let error = export_sqlite_stream(&source_path, std::io::sink()).unwrap_err();

        assert!(
            format!("{error:#}").contains("SQLite migration version 10 does not match"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn snapshot_rejects_a_source_with_incomplete_migration_history() {
        let temporary = tempfile::tempdir().unwrap();
        let source_path = temporary.path().join("incomplete.db");
        let mut store = Store::open(&source_path).unwrap();
        diesel::delete(flyway_migrations::table.filter(flyway_migrations::version.eq(9_i64)))
            .execute(&mut store.connection)
            .unwrap();
        drop(store);

        let error = export_sqlite_stream(&source_path, std::io::sink()).unwrap_err();

        assert!(
            format!("{error:#}").contains("migration history is missing supported version 9"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn snapshot_rejects_an_oversized_sqlite_source_before_opening_it() {
        let source = tempfile::NamedTempFile::new().unwrap();
        source
            .as_file()
            .set_len(4 * 1024 * 1024 * 1024 + 1)
            .unwrap();

        let error = export_sqlite_stream(source.path(), std::io::sink()).unwrap_err();

        assert!(
            format!("{error:#}").contains("SQLite import source exceeds"),
            "unexpected error: {error:#}"
        );
    }
}
