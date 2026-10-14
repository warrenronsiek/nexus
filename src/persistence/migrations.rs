// @feature persistence
// @spec docs/features/persistence.md
// @entrypoint migrate_database
use super::schema::flyway_migrations;
use crate::persistence::backend::DbConnection;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use diesel::connection::{SimpleConnection, TransactionManager};
use diesel::dsl::{max, min};
use diesel::prelude::*;
use flyway::{
    migrations, ChangelogFile, MigrationExecutor, MigrationRunner, MigrationState,
    MigrationStateManager, MigrationStatus, MigrationStore, MigrationsError,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[cfg(feature = "sqlite")]
#[migrations("migrations")]
struct NexusMigrations {}

#[cfg(feature = "postgres")]
#[migrations("postgres_migrations")]
struct NexusMigrations {}

pub(super) fn establish_connection(database_url: &str) -> Result<DbConnection> {
    let connection =
        DbConnection::establish(database_url).context("establish database connection")?;
    #[cfg(feature = "sqlite")]
    {
        let mut connection = connection;
        connection
            .batch_execute("PRAGMA busy_timeout = 2000;")
            .context("configure SQLite busy timeout")?;
        Ok(connection)
    }
    #[cfg(feature = "postgres")]
    {
        Ok(connection)
    }
}

pub(super) fn migrate_database(path: &Path) -> Result<Option<u64>> {
    #[cfg(feature = "sqlite")]
    {
        migrate_with(path, NexusMigrations {})
    }
    #[cfg(feature = "postgres")]
    {
        let _ = path;
        anyhow::bail!("filesystem database paths require the sqlite build");
    }
}

#[cfg(feature = "sqlite")]
fn migrate_with<S>(path: &Path, migrations: S) -> Result<Option<u64>>
where
    S: MigrationStore + Send + 'static,
{
    let path = path
        .to_str()
        .with_context(|| format!("database path is not valid UTF-8: {}", path.display()))?;
    migrate_url_with(path.to_owned(), migrations)
}

pub(super) fn migrate_database_url(database_url: &str) -> Result<Option<u64>> {
    migrate_url_with(database_url.to_owned(), NexusMigrations {})
}

#[cfg(feature = "sqlite")]
pub(super) fn validate_portable_source_migrations(connection: &mut DbConnection) -> Result<()> {
    let stored = flyway_migrations::table
        .order(flyway_migrations::version.asc())
        .select((
            flyway_migrations::version,
            flyway_migrations::name,
            flyway_migrations::checksum,
            flyway_migrations::status,
        ))
        .load::<(i64, String, String, String)>(connection)
        .context("read SQLite migration history before portable export")?;
    let mut expected = NexusMigrations {}.changelogs();
    expected.sort_by_key(ChangelogFile::version);

    for (version, name, checksum, status) in &stored {
        let unsigned_version = u64::try_from(*version)
            .with_context(|| format!("unsupported SQLite migration version {version}"))?;
        let Some(changelog) = expected
            .iter()
            .find(|changelog| changelog.version() == unsigned_version)
        else {
            anyhow::bail!("unsupported SQLite migration version {version}");
        };
        if name != &changelog.name
            || checksum != &changelog.checksum.to_string()
            || status != "deployed"
        {
            anyhow::bail!("SQLite migration version {version} does not match this Nexus binary");
        }
    }

    for changelog in &expected {
        let version = migration_version(changelog.version())
            .map_err(|error| anyhow!("validate embedded SQLite migration version: {error}"))?;
        if !stored.iter().any(|stored| stored.0 == version) {
            anyhow::bail!("SQLite migration history is missing supported version {version}");
        }
    }
    Ok(())
}

fn migrate_url_with<S>(database_url: String, migrations: S) -> Result<Option<u64>>
where
    S: MigrationStore + Send + 'static,
{
    std::thread::Builder::new()
        .name("nexus-database-migrations".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("build migration runtime")?;
            runtime.block_on(async move {
                let connection = establish_connection(&database_url)
                    .context("initialize migration connection")?;
                let driver = Arc::new(DieselMigrationDriver::new(connection));
                MigrationRunner::new(migrations, driver.clone(), driver, false)
                    .migrate()
                    .await
                    .map_err(|error| anyhow!("run flyway-rs migrations: {error}"))
            })
        })
        .context("start database migration thread")?
        .join()
        .map_err(|_| anyhow!("database migration thread panicked"))?
}

struct DieselMigrationDriver {
    connection: Mutex<DbConnection>,
}

enum VersionBound {
    Lowest,
    Highest,
}

#[derive(Insertable)]
#[diesel(table_name = flyway_migrations)]
struct NewMigration<'a> {
    version: i64,
    name: &'a str,
    checksum: String,
    status: &'a str,
}

impl DieselMigrationDriver {
    fn new(connection: DbConnection) -> Self {
        Self {
            connection: Mutex::new(connection),
        }
    }

    fn connection(&self) -> flyway::Result<std::sync::MutexGuard<'_, DbConnection>> {
        self.connection.lock().map_err(|_| {
            MigrationsError::migration_database_failed(
                None,
                Some(anyhow!("migration connection lock is poisoned").into()),
            )
        })
    }

    fn deployed_version(&self, bound: VersionBound) -> flyway::Result<Option<MigrationState>> {
        let version = match bound {
            VersionBound::Lowest => flyway_migrations::table
                .filter(flyway_migrations::status.eq("deployed"))
                .select(min(flyway_migrations::version))
                .first::<Option<i64>>(&mut *self.connection()?),
            VersionBound::Highest => flyway_migrations::table
                .filter(flyway_migrations::status.eq("deployed"))
                .select(max(flyway_migrations::version))
                .first::<Option<i64>>(&mut *self.connection()?),
        }
        .map_err(versioning_error)?;
        version.map(deployed_state).transpose()
    }
}

#[async_trait]
impl MigrationStateManager for DieselMigrationDriver {
    async fn prepare(&self) -> flyway::Result<()> {
        self.connection()?
            .batch_execute(migration_bootstrap_sql())
            .map_err(|error| MigrationsError::migration_setup_failed(Some(error.into())))
    }

    async fn lowest_version(&self) -> flyway::Result<Option<MigrationState>> {
        self.deployed_version(VersionBound::Lowest)
    }

    async fn highest_version(&self) -> flyway::Result<Option<MigrationState>> {
        self.deployed_version(VersionBound::Highest)
    }

    async fn list_versions(&self) -> flyway::Result<Vec<MigrationState>> {
        flyway_migrations::table
            .filter(flyway_migrations::status.eq("deployed"))
            .order(flyway_migrations::version.asc())
            .select(flyway_migrations::version)
            .load::<i64>(&mut *self.connection()?)
            .map_err(versioning_error)?
            .into_iter()
            .map(deployed_state)
            .collect()
    }

    // flyway 0.7 invokes these outside the changelog transaction. The executor records the
    // deployed version beside the schema change so both commit or roll back together.
    async fn begin_version(&self, _changelog: &ChangelogFile) -> flyway::Result<()> {
        Ok(())
    }

    async fn finish_version(&self, _changelog: &ChangelogFile) -> flyway::Result<()> {
        Ok(())
    }

    // Nexus always configures the runner to stop on failure, so this callback is unreachable.
    async fn skip_version(&self, _changelog: &ChangelogFile) -> flyway::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl MigrationExecutor for DieselMigrationDriver {
    async fn begin_transaction(&self) -> flyway::Result<()> {
        <DbConnection as Connection>::TransactionManager::begin_transaction(
            &mut *self.connection()?,
        )
        .map_err(database_error)
    }

    async fn execute_changelog_file(&self, changelog: &ChangelogFile) -> flyway::Result<()> {
        let mut connection = self.connection()?;
        connection
            .batch_execute(changelog.content())
            .map_err(database_step_error)?;
        diesel::insert_into(flyway_migrations::table)
            .values(NewMigration {
                version: migration_version(changelog.version())?,
                name: &changelog.name,
                checksum: changelog.checksum.to_string(),
                status: "deployed",
            })
            .execute(&mut *connection)
            .map(|_| ())
            .map_err(versioning_error)
    }

    async fn commit_transaction(&self) -> flyway::Result<()> {
        <DbConnection as Connection>::TransactionManager::commit_transaction(
            &mut *self.connection()?,
        )
        .map_err(database_error)
    }

    async fn rollback_transaction(&self) -> flyway::Result<()> {
        <DbConnection as Connection>::TransactionManager::rollback_transaction(
            &mut *self.connection()?,
        )
        .map_err(database_error)
    }
}

fn migration_version(version: u64) -> flyway::Result<i64> {
    i64::try_from(version).map_err(|error| {
        MigrationsError::custom_message(
            "migration version exceeds database integer range",
            None,
            Some(error.into()),
        )
    })
}

#[cfg(feature = "sqlite")]
fn migration_bootstrap_sql() -> &'static str {
    include_str!("../../migrations/V0_flyway_migrations.sql")
}

#[cfg(feature = "postgres")]
fn migration_bootstrap_sql() -> &'static str {
    include_str!("../../postgres_migrations/V0_flyway_migrations.sql")
}

fn deployed_state(version: i64) -> flyway::Result<MigrationState> {
    Ok(MigrationState {
        version: u64::try_from(version)
            .map_err(|error| MigrationsError::migration_versioning_failed(Some(error.into())))?,
        status: MigrationStatus::Deployed,
    })
}

fn database_error(error: diesel::result::Error) -> MigrationsError {
    MigrationsError::migration_database_failed(None, Some(error.into()))
}

fn database_step_error(error: diesel::result::Error) -> MigrationsError {
    MigrationsError::migration_database_step_failed(None, Some(error.into()))
}

fn versioning_error(error: diesel::result::Error) -> MigrationsError {
    MigrationsError::migration_versioning_failed(Some(error.into()))
}

#[cfg(all(test, feature = "sqlite"))]
mod tests;
