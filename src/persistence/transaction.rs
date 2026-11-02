// @feature persistence
// @spec docs/features/persistence.md
use super::Store;
use crate::persistence::backend::DbConnection;
use anyhow::Result;
#[cfg(feature = "sqlite")]
use diesel::connection::{AnsiTransactionManager, TransactionManager};
use diesel::prelude::*;
#[cfg(feature = "sqlite")]
use diesel::result::Error;

#[cfg(feature = "postgres")]
diesel::define_sql_function! {
    fn pg_advisory_xact_lock(key: diesel::sql_types::BigInt);
}

#[cfg(feature = "postgres")]
const POSTGRES_WRITER_LOCK_KEY: i64 = 0x4e45585553; // ASCII NEXUS, database-scoped.

pub(super) enum TransactionMode {
    Deferred,
    Immediate,
}

impl Store {
    pub(super) fn transaction<T>(
        &mut self,
        operation: impl FnOnce(&mut DbConnection) -> Result<T>,
    ) -> Result<T> {
        transaction(&mut self.connection, TransactionMode::Deferred, operation)
    }
}

pub(super) fn transaction<T>(
    connection: &mut DbConnection,
    mode: TransactionMode,
    operation: impl FnOnce(&mut DbConnection) -> Result<T>,
) -> Result<T> {
    #[cfg(feature = "postgres")]
    {
        let _ = mode;
        connection.transaction::<_, anyhow::Error, _>(|connection| {
            // Preserve single-writer aggregate semantics across the service connections.
            diesel::select(pg_advisory_xact_lock(POSTGRES_WRITER_LOCK_KEY)).execute(connection)?;
            operation(connection)
        })
    }
    #[cfg(feature = "sqlite")]
    {
        sqlite_transaction(connection, mode, operation)
    }
}

pub(super) fn prepared_immediate_transaction<Input, Prepared, Output>(
    connection: &mut DbConnection,
    input: Input,
    prepare: impl FnOnce(Input) -> Result<Prepared>,
    operation: impl FnOnce(&mut DbConnection, &Prepared) -> Result<Output>,
) -> Result<Output> {
    let prepared = prepare(input)?;
    transaction(connection, TransactionMode::Immediate, |connection| {
        operation(connection, &prepared)
    })
}

#[cfg(feature = "sqlite")]
fn sqlite_transaction<T>(
    connection: &mut DbConnection,
    mode: TransactionMode,
    operation: impl FnOnce(&mut DbConnection) -> Result<T>,
) -> Result<T> {
    let mut reached_commit = false;
    let operation = |connection: &mut DbConnection| {
        let value = operation(connection)?;
        reached_commit = true;
        Ok(value)
    };
    let result = match mode {
        TransactionMode::Deferred => connection.transaction::<_, anyhow::Error, _>(operation),
        TransactionMode::Immediate => {
            connection.immediate_transaction::<_, anyhow::Error, _>(operation)
        }
    };
    match result {
        // SQLite keeps a busy COMMIT active; Diesel leaves rollback to the caller.
        Err(error) if reached_commit => {
            match AnsiTransactionManager::rollback_transaction(connection) {
                Ok(()) | Err(Error::NotInTransaction) => Err(error),
                Err(rollback_error) => {
                    Err(error.context(format!("SQLite rollback failed: {rollback_error}")))
                }
            }
        }
        result => result,
    }
}

#[cfg(all(test, feature = "postgres"))]
mod postgres_tests;
