// @feature persistence
// @spec docs/features/persistence.md
use super::Store;
use anyhow::Result;
use diesel::connection::{AnsiTransactionManager, TransactionManager};
use diesel::prelude::*;
use diesel::result::Error;

pub(super) enum TransactionMode {
    Deferred,
    Immediate,
}

impl Store {
    pub(super) fn transaction<T>(
        &mut self,
        operation: impl FnOnce(&mut SqliteConnection) -> Result<T>,
    ) -> Result<T> {
        transaction(&mut self.connection, TransactionMode::Deferred, operation)
    }
}

pub(super) fn transaction<T>(
    connection: &mut SqliteConnection,
    mode: TransactionMode,
    operation: impl FnOnce(&mut SqliteConnection) -> Result<T>,
) -> Result<T> {
    let mut reached_commit = false;
    let operation = |connection: &mut SqliteConnection| {
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
