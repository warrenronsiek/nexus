// @feature runtime
// @feature persistence
// @spec docs/features/runtime.md
// @spec docs/features/persistence.md
use anyhow::{bail, Context, Result};
use nexus::persistence::{ImportOutcome, Store};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(super) fn run(explicit_path: Option<PathBuf>, database_url: &str) -> Result<()> {
    SqliteImportRequest::from_sources(
        explicit_path,
        std::env::var_os("NEXUS_IMPORT_SQLITE").map(PathBuf::from),
        std::env::var_os("NEXUS_IMPORT_SQLITE_IF_PRESENT").map(PathBuf::from),
    )
    .resolve()?
    .execute(database_url)
}

#[derive(Debug, Eq, PartialEq)]
enum SqliteImportRequest {
    NotConfigured,
    Required(PathBuf),
    IfPresent(PathBuf),
}

impl SqliteImportRequest {
    fn from_sources(
        explicit_path: Option<PathBuf>,
        required_environment: Option<PathBuf>,
        optional_environment: Option<PathBuf>,
    ) -> Self {
        explicit_path
            .or(required_environment)
            .map(Self::Required)
            .or_else(|| optional_environment.map(Self::IfPresent))
            .unwrap_or(Self::NotConfigured)
    }

    fn resolve(self) -> Result<SqliteImportDecision> {
        match self {
            Self::NotConfigured => Ok(SqliteImportDecision::Skip(SkipReason::Unconfigured)),
            Self::Required(path) => required_source(path),
            Self::IfPresent(path) => optional_source(path),
        }
    }
}

enum SqliteImportDecision {
    Skip(SkipReason),
    Import(PathBuf),
}

impl SqliteImportDecision {
    fn execute(self, database_url: &str) -> Result<()> {
        match self {
            Self::Skip(reason) => {
                reason.log();
                Ok(())
            }
            Self::Import(path) => import_database(&path, database_url),
        }
    }
}

enum SkipReason {
    Unconfigured,
    Missing(PathBuf),
    NonFile(PathBuf),
}

impl SkipReason {
    fn log(self) {
        match self {
            Self::Unconfigured => {
                eprintln!("SQLite import skipped: no import source was configured");
            }
            Self::Missing(path) => {
                eprintln!("SQLite import skipped: {} is not present", path.display());
            }
            Self::NonFile(path) => eprintln!(
                "SQLite import skipped: {} is not a regular file",
                path.display()
            ),
        }
    }
}

fn required_source(path: PathBuf) -> Result<SqliteImportDecision> {
    let metadata = std::fs::metadata(&path)
        .with_context(|| format!("inspect SQLite import source {}", path.display()))?;
    if !metadata.is_file() {
        bail!(
            "SQLite import source is not a regular file: {}",
            path.display()
        );
    }
    Ok(SqliteImportDecision::Import(path))
}

fn optional_source(path: PathBuf) -> Result<SqliteImportDecision> {
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(SqliteImportDecision::Import(path)),
        Ok(_) => Ok(SqliteImportDecision::Skip(SkipReason::NonFile(path))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(SqliteImportDecision::Skip(SkipReason::Missing(path)))
        }
        Err(error) => {
            Err(error).with_context(|| format!("inspect SQLite import source {}", path.display()))
        }
    }
}

fn import_database(path: &Path, database_url: &str) -> Result<()> {
    let snapshot = export_sqlite_snapshot(path)?;
    let mut target = Store::connect(database_url).context("open PostgreSQL import target")?;
    match target.import_streamed_snapshot(snapshot.path())? {
        ImportOutcome::Imported { rows, digest } => eprintln!(
            "SQLite import completed: {rows} rows from {} (snapshot {digest})",
            path.display()
        ),
        ImportOutcome::AlreadyImported { rows, digest } => eprintln!(
            "SQLite import skipped: the same {rows}-row snapshot was already imported ({digest})"
        ),
    }
    Ok(())
}

fn export_sqlite_snapshot(database_path: &Path) -> Result<tempfile::NamedTempFile> {
    let current_executable = std::env::current_exe().context("locate nexus-server executable")?;
    let exporter = sibling_exporter(&current_executable);
    let output = tempfile::NamedTempFile::new().context("create portable snapshot output")?;
    let output_file = output
        .reopen()
        .context("open portable snapshot output for exporter")?;
    let status = Command::new(&exporter)
        .arg("--database")
        .arg(database_path)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::from(output_file))
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| {
            format!(
                "run SQLite snapshot helper {}; build or install nexus-sqlite-export beside nexus-server",
                exporter.display()
            )
        })?;
    if !status.success() {
        bail!("SQLite snapshot helper exited with {status}");
    }
    Ok(output)
}

fn sibling_exporter(server_executable: &Path) -> PathBuf {
    server_executable.with_file_name("nexus-sqlite-export")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_import_path_precedes_both_environment_sources() {
        let request = SqliteImportRequest::from_sources(
            Some(PathBuf::from("explicit.db")),
            Some(PathBuf::from("required.db")),
            Some(PathBuf::from("optional.db")),
        );

        assert_eq!(
            request,
            SqliteImportRequest::Required(PathBuf::from("explicit.db"))
        );
    }

    #[test]
    fn required_environment_precedes_the_optional_compose_source() {
        let request = SqliteImportRequest::from_sources(
            None,
            Some(PathBuf::from("required.db")),
            Some(PathBuf::from("optional.db")),
        );

        assert_eq!(
            request,
            SqliteImportRequest::Required(PathBuf::from("required.db"))
        );
    }

    #[test]
    fn a_missing_optional_compose_source_remains_a_skip() {
        let temporary = tempfile::tempdir().unwrap();
        let missing = temporary.path().join("missing.db");

        match SqliteImportRequest::IfPresent(missing.clone())
            .resolve()
            .unwrap()
        {
            SqliteImportDecision::Skip(SkipReason::Missing(path)) => {
                assert_eq!(path, missing);
            }
            _ => panic!("missing optional import source must be skipped"),
        }
    }

    #[test]
    fn snapshot_exporter_is_always_the_server_sibling() {
        assert_eq!(
            sibling_exporter(Path::new("/opt/nexus/bin/nexus-server")),
            PathBuf::from("/opt/nexus/bin/nexus-sqlite-export")
        );
    }
}
