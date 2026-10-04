// @feature persistence
// @spec docs/features/persistence.md
// @entrypoint migrate_database
use super::schema::flyway_migrations;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use diesel::connection::{SimpleConnection, TransactionManager};
use diesel::dsl::{max, min};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use flyway::{
    migrations, ChangelogFile, MigrationExecutor, MigrationRunner, MigrationState,
    MigrationStateManager, MigrationStatus, MigrationStore, MigrationsError,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[migrations("migrations")]
struct NexusMigrations {}

fn establish_migration_connection(database_url: &str) -> Result<SqliteConnection> {
    let mut connection = SqliteConnection::establish(database_url)
        .context("establish SQLite database connection")?;
    connection
        .batch_execute("PRAGMA busy_timeout = 2000;")
        .context("configure SQLite busy timeout")?;
    Ok(connection)
}

pub(super) fn migrate_database(path: &Path) -> Result<Option<u64>> {
    migrate_with(path, NexusMigrations {})
}

fn migrate_with<S>(path: &Path, migrations: S) -> Result<Option<u64>>
where
    S: MigrationStore + Send + 'static,
{
    let path = path
        .to_str()
        .with_context(|| format!("database path is not valid UTF-8: {}", path.display()))?;
    let database_url = path.to_owned();

    std::thread::Builder::new()
        .name("nexus-database-migrations".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("build migration runtime")?;
            runtime.block_on(async move {
                let connection = establish_migration_connection(&database_url)
                    .context("initialize flyway-rs SQLite connection")?;
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
    connection: Mutex<SqliteConnection>,
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
    fn new(connection: SqliteConnection) -> Self {
        Self {
            connection: Mutex::new(connection),
        }
    }

    fn connection(&self) -> flyway::Result<std::sync::MutexGuard<'_, SqliteConnection>> {
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
            .batch_execute(include_str!("../../migrations/V0_flyway_migrations.sql"))
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

    async fn begin_version(&self, changelog: &ChangelogFile) -> flyway::Result<()> {
        diesel::insert_into(flyway_migrations::table)
            .values(NewMigration {
                version: migration_version(changelog.version())?,
                name: &changelog.name,
                checksum: changelog.checksum.to_string(),
                status: "in_progress",
            })
            .execute(&mut *self.connection()?)
            .map(|_| ())
            .map_err(versioning_error)
    }

    async fn finish_version(&self, changelog: &ChangelogFile) -> flyway::Result<()> {
        set_status(self, changelog, "deployed")
    }

    async fn skip_version(&self, changelog: &ChangelogFile) -> flyway::Result<()> {
        set_status(self, changelog, "skipped")
    }
}

#[async_trait]
impl MigrationExecutor for DieselMigrationDriver {
    async fn begin_transaction(&self) -> flyway::Result<()> {
        <SqliteConnection as Connection>::TransactionManager::begin_transaction(
            &mut *self.connection()?,
        )
        .map_err(database_error)
    }

    async fn execute_changelog_file(&self, changelog: &ChangelogFile) -> flyway::Result<()> {
        self.connection()?
            .batch_execute(changelog.content())
            .map_err(database_step_error)
    }

    async fn commit_transaction(&self) -> flyway::Result<()> {
        <SqliteConnection as Connection>::TransactionManager::commit_transaction(
            &mut *self.connection()?,
        )
        .map_err(database_error)
    }

    async fn rollback_transaction(&self) -> flyway::Result<()> {
        <SqliteConnection as Connection>::TransactionManager::rollback_transaction(
            &mut *self.connection()?,
        )
        .map_err(database_error)
    }
}

fn set_status(
    driver: &DieselMigrationDriver,
    changelog: &ChangelogFile,
    status: &str,
) -> flyway::Result<()> {
    diesel::update(
        flyway_migrations::table
            .filter(flyway_migrations::version.eq(migration_version(changelog.version())?)),
    )
    .set(flyway_migrations::status.eq(status))
    .execute(&mut *driver.connection()?)
    .map(|_| ())
    .map_err(versioning_error)
}

fn migration_version(version: u64) -> flyway::Result<i64> {
    i64::try_from(version).map_err(|error| {
        MigrationsError::custom_message(
            "migration version exceeds SQLite integer range",
            None,
            Some(error.into()),
        )
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use flyway::ChangelogFile;

    struct VersionZeroOnly;

    impl MigrationStore for VersionZeroOnly {
        fn changelogs(&self) -> Vec<ChangelogFile> {
            NexusMigrations {}
                .changelogs()
                .into_iter()
                .filter(|changelog| changelog.version() == 0)
                .collect()
        }
    }

    struct ThroughVersionTwo;

    impl MigrationStore for ThroughVersionTwo {
        fn changelogs(&self) -> Vec<ChangelogFile> {
            NexusMigrations {}
                .changelogs()
                .into_iter()
                .filter(|changelog| changelog.version() <= 2)
                .collect()
        }
    }

    #[test]
    fn applies_only_unseen_versions_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("upgrade.db");

        assert_eq!(migrate_with(&path, VersionZeroOnly).unwrap(), Some(0));
        assert_eq!(migrate_database(&path).unwrap(), Some(3));
        assert_eq!(migrate_database(&path).unwrap(), Some(3));
    }

    #[test]
    fn upgrades_an_existing_v2_database_without_losing_events() {
        use super::super::models::NewEvent;
        use super::super::schema::events;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("upgrade-v2.db");
        assert_eq!(migrate_with(&path, ThroughVersionTwo).unwrap(), Some(2));

        let mut connection = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        diesel::insert_into(events::table)
            .values(NewEvent {
                project_id: "project-before-upgrade",
                session_id: None,
                kind: "pre_upgrade_event",
                payload_json: "{}".to_owned(),
                created_at: chrono::Utc::now().to_rfc3339(),
            })
            .execute(&mut connection)
            .unwrap();
        drop(connection);

        assert_eq!(migrate_database(&path).unwrap(), Some(3));
        let mut store = super::super::Store::open(&path).unwrap();
        let events = store
            .list_events(Some("project-before-upgrade"), 10)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "pre_upgrade_event");
        assert!(store
            .usage_summary(
                None,
                chrono::Utc::now() - chrono::Duration::days(7),
                chrono::Utc::now(),
            )
            .unwrap()
            .tools
            .is_empty());
    }
}
