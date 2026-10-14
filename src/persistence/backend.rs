// @feature persistence
// @spec docs/features/persistence.md
#[cfg(all(feature = "sqlite", feature = "postgres"))]
compile_error!("enable exactly one persistence backend: sqlite or postgres");

#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
compile_error!("enable exactly one persistence backend: sqlite or postgres");

#[cfg(feature = "postgres")]
pub(crate) type DbBackend = diesel::pg::Pg;
#[cfg(feature = "postgres")]
pub(crate) type DbConnection = diesel::pg::PgConnection;

#[cfg(feature = "sqlite")]
pub(crate) type DbBackend = diesel::sqlite::Sqlite;
#[cfg(feature = "sqlite")]
pub(crate) type DbConnection = diesel::sqlite::SqliteConnection;
